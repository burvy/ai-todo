//! The `Content-Security-Policy` for the pages this server hands out.
//!
//! Scripts may come from this site, from Google's sign-in script, or be one of the
//! inline scripts the page itself carries, named by hash: trunk writes an inline
//! module that starts the wasm, and its file names change with every build, so the
//! hashes are taken from the HTML as served, not written down here.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

/// The policy for one HTML document.
pub fn for_html(html: &str) -> String {
    let hashes: String = inline_scripts(html)
        .map(|body| {
            format!(
                " 'sha256-{}'",
                STANDARD.encode(Sha256::digest(body.as_bytes()))
            )
        })
        .collect();
    [
        "default-src 'self'".to_owned(),
        // `wasm-unsafe-eval` lets the browser compile the wasm; it does not allow `eval`.
        format!(
            "script-src 'self' 'wasm-unsafe-eval' https://accounts.google.com/gsi/client{hashes}"
        ),
        // The page sets style attributes on elements, which only `unsafe-inline` allows.
        "style-src 'self' 'unsafe-inline' https://accounts.google.com/gsi/style".to_owned(),
        // Profile pictures come from Google's image hosts.
        "img-src 'self' data: https:".to_owned(),
        "font-src 'self' data:".to_owned(),
        "media-src 'self' blob: data:".to_owned(),
        "connect-src 'self' https://accounts.google.com/gsi/".to_owned(),
        "frame-src https://accounts.google.com/gsi/".to_owned(),
        "object-src 'none'".to_owned(),
        "base-uri 'self'".to_owned(),
        "form-action 'self'".to_owned(),
        "frame-ancestors 'none'".to_owned(),
    ]
    .join("; ")
}

/// The text of every `<script>` that has no `src`, exactly as the browser will hash it.
fn inline_scripts(html: &str) -> impl Iterator<Item = &str> {
    let mut rest = html;
    std::iter::from_fn(move || {
        loop {
            let open = rest.find("<script")?;
            let after_open = &rest[open..];
            let tag_end = after_open.find('>')?;
            let tag = &after_open[..tag_end];
            let body_start = tag_end + 1;
            let close = after_open[body_start..].find("</script>")?;
            let body = &after_open[body_start..body_start + close];
            rest = &after_open[body_start + close + "</script>".len()..];
            if !tag.contains(" src=") {
                return Some(body);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inline_script_is_named_by_its_hash() {
        // sha256("alert(1)") in base64, computed independently.
        let policy = for_html("<html><script>alert(1)</script></html>");
        assert!(
            policy.contains("'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI='"),
            "{policy}"
        );
    }

    #[test]
    fn a_script_with_a_source_adds_no_hash() {
        let policy =
            for_html(r#"<script src="/app.js"></script><script type="module">x()</script>"#);
        assert_eq!(policy.matches("'sha256-").count(), 1, "{policy}");
    }

    #[test]
    fn the_policy_lets_wasm_load_but_never_eval_or_foreign_scripts() {
        let policy = for_html("<html></html>");
        assert!(policy.contains("'wasm-unsafe-eval'"));
        assert!(!policy.contains("'unsafe-eval'"));
        let scripts = policy
            .split("; ")
            .find(|d| d.starts_with("script-src"))
            .unwrap();
        assert!(!scripts.contains("unsafe-inline"), "{scripts}");
        assert!(policy.contains("frame-ancestors 'none'"));
        assert!(policy.contains("object-src 'none'"));
    }
}
