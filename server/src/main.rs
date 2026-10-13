mod v2;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use axum::{
    Router,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use include_dir::{Dir, include_dir};

static WEB_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../web/dist");

fn app(store: v2::app::App<Box<dyn v2::store::Store>>) -> Router {
    v2::http::router(store, judge::AnyJudge::from_env()).fallback(embedded_web)
}

/// The web page, compiled into the exe. Unknown paths get `index.html` so the
/// page can handle its own routes; unknown `/api/` paths are a plain 404.
async fn embedded_web(uri: axum::http::Uri) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let relative = path.trim_start_matches('/');
    let file = if relative.is_empty() {
        WEB_DIST.get_file("index.html")
    } else {
        WEB_DIST
            .get_file(relative)
            .or_else(|| WEB_DIST.get_file("index.html"))
    };
    match file {
        Some(file) => {
            let content_type = match Path::new(file.path())
                .extension()
                .and_then(|ext| ext.to_str())
            {
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("svg") => "image/svg+xml",
                Some("wasm") => "application/wasm",
                Some("json") => "application/json",
                _ => "text/html; charset=utf-8",
            };
            ([(header::CONTENT_TYPE, content_type)], file.contents()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[tokio::main]
async fn main() {
    // Order matters: find the folder and settings first, then open (and migrate) the
    // database once, and only then start the tasks that open their own connections.
    run_from_exe_folder();
    // Real environment variables win over the file, so a shell or service can override it.
    dotenvy::from_filename(CONFIG_FILE).ok();
    if std::env::var("TYPESAFE_API_KEY").is_err() {
        println!(
            "note: TYPESAFE_API_KEY is not set (put it in {CONFIG_FILE} next to the exe); Jev is off"
        );
    }
    let store = v2::app_from_env().unwrap_or_else(|problem| fatal(&problem));
    v2::start_backups().unwrap_or_else(|problem| fatal(&problem));
    v2::admin::start_console(v2::store::data_path_from_env());
    let port = port_from_env();
    let addr = SocketAddr::from((
        bind_from_env(std::env::var("RAMBL_BIND").ok()).unwrap_or_else(|problem| fatal(&problem)),
        port,
    ));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => fatal(&format!("could not listen on {addr}: {e}")),
    };
    println!("{}", running_on(addr));
    let service = app(store).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, service).await {
        fatal(&format!("server stopped: {e}"));
    }
}

/// The settings file, read from the working folder. Its own name, not `.env`, so it cannot
/// be mistaken for another server's file in a shared folder.
const CONFIG_FILE: &str = "rambl.env";

/// The port: `RAMBL_PORT`, else `PORT`, else 3160.
fn port_from_env() -> u16 {
    port_from(std::env::var("RAMBL_PORT").ok(), std::env::var("PORT").ok())
}

/// The port from the two variables' values. A value that is not a port number is
/// ignored, as if it were unset.
fn port_from(rambl_port: Option<String>, port: Option<String>) -> u16 {
    [rambl_port, port]
        .into_iter()
        .flatten()
        .find_map(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

const DEFAULT_PORT: u16 = 3160;

/// The address the server listens on: `RAMBL_BIND`, else loopback.
///
/// Loopback by default, because the Cloudflare tunnel on this machine is the way in.
/// Listening on every interface would put the server on the LAN, where a client could
/// reach it directly and put any address it liked in `CF-Connecting-IP`, which is the
/// header the per-address limits and blocks rely on. It also pops a firewall prompt on
/// Windows every time a new build path runs the exe.
///
/// A value that is not an IP address stops the server rather than quietly falling back
/// to loopback: an operator who asked for `0.0.0.0` and did not get it would rather be
/// told than find out from a visitor.
fn bind_from_env(raw: Option<String>) -> Result<IpAddr, String> {
    match raw
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => Ok(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        Some(value) => value.parse().map_err(|_| {
            format!("RAMBL_BIND is not an IP address: {value:?} (try 127.0.0.1 or 0.0.0.0)")
        }),
    }
}

/// What the window says once the socket is open, naming the address actually bound
/// rather than the one that was asked for.
fn running_on(addr: SocketAddr) -> String {
    let open = if addr.ip().is_unspecified() {
        format!("http://localhost:{}", addr.port())
    } else {
        format!("http://{addr}")
    };
    format!("Rambl is listening on {addr} — open {open} (close this window to stop it)")
}

#[cfg(test)]
mod settings_tests {
    use super::bind_from_env;
    use super::port_from;
    use super::running_on;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn text(value: &str) -> Option<String> {
        Some(value.to_owned())
    }

    #[test]
    fn rambl_port_wins_then_port_then_the_default() {
        assert_eq!(port_from(text("3170"), text("8080")), 3170);
        assert_eq!(port_from(None, text("8080")), 8080);
        assert_eq!(port_from(None, None), 3160);
    }

    #[test]
    fn a_value_that_is_not_a_port_is_ignored() {
        assert_eq!(port_from(text("not-a-port"), text("8080")), 8080);
        assert_eq!(port_from(text(""), None), 3160);
    }

    /// Unset, empty or blank means the same thing: this machine only. The tunnel is
    /// here, and a listener on every interface would also pop a firewall prompt on
    /// Windows for every new build path.
    #[test]
    fn the_default_is_loopback() {
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        for unset in [None, text(""), text("   ")] {
            assert_eq!(
                bind_from_env(unset.clone()),
                Ok(loopback),
                "{unset:?} should bind loopback"
            );
        }
    }

    #[test]
    fn a_listened_address_is_honoured() {
        for (given, want) in [
            ("0.0.0.0", "0.0.0.0"),
            (" 10.0.0.5 ", "10.0.0.5"),
            ("::", "::"),
            ("::1", "::1"),
        ] {
            assert_eq!(
                bind_from_env(text(given)),
                Ok(want.parse().unwrap()),
                "{given} should be listened on as itself"
            );
        }
    }

    /// A value that is not an address stops the server. Falling back to loopback would
    /// leave an operator who asked for `0.0.0.0` wondering why nothing was reachable.
    #[test]
    fn an_address_that_is_not_an_address_refuses_to_start() {
        for given in [
            "localhost",
            "not-an-ip",
            "0.0.0.0:3160",
            "3160",
            "999.1.1.1",
        ] {
            let refusal = bind_from_env(text(given)).unwrap_err();
            assert!(
                refusal.contains("RAMBL_BIND") && refusal.contains(given),
                "{given} should be refused by name, got: {refusal}"
            );
        }
    }

    /// The window names the address that was bound, and points at one a browser can
    /// open. `0.0.0.0` is what was listened on, `localhost` is what to type.
    #[test]
    fn the_window_names_the_address_actually_bound() {
        let loopback: SocketAddr = "127.0.0.1:3160".parse().unwrap();
        assert_eq!(
            running_on(loopback),
            "Rambl is listening on 127.0.0.1:3160 — open http://127.0.0.1:3160 \
             (close this window to stop it)"
        );
        let everywhere: SocketAddr = "0.0.0.0:3160".parse().unwrap();
        assert!(
            running_on(everywhere).contains("open http://localhost:3160"),
            "{}",
            running_on(everywhere)
        );
        let six: SocketAddr = "[::1]:3160".parse().unwrap();
        assert!(
            running_on(six).contains("open http://[::1]:3160"),
            "an IPv6 address needs its brackets",
        );
    }
}

/// A double-clicked exe starts in some other folder; work from the exe's own folder when
/// it holds the release layout, so `data/` and `rambl.env` next to it are found.
/// `cargo run` (no `rambl.env.example` beside the exe) is unaffected.
fn run_from_exe_folder() {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        && dir.join("rambl.env.example").is_file()
    {
        let _ = std::env::set_current_dir(dir);
    }
}

/// Print the error and wait for Enter, so a double-clicked window does not
/// vanish before the message can be read.
fn fatal(message: &str) -> ! {
    eprintln!("error: {message}");
    eprintln!("press Enter to close");
    let _ = std::io::stdin().read_line(&mut String::new());
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let store: Box<dyn v2::store::Store> = Box::new(v2::store::MemoryStore::default());
        let store = v2::app::App::new(store, std::sync::Arc::new(todo_core::SystemClock));
        app(store)
    }

    #[tokio::test]
    async fn the_root_serves_the_embedded_page() {
        let response = test_app()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response.headers().get(header::CONTENT_TYPE).unwrap();
        assert!(content_type.to_str().unwrap().starts_with("text/html"));
    }

    #[tokio::test]
    async fn an_unknown_api_path_is_a_404_not_the_page() {
        let response = test_app()
            .oneshot(Request::get("/api/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
