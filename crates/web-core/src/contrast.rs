//! The page's text is readable on its paper in both themes (WCAG 2.1 AA, ATD-144):
//! every ink colour used for text has a contrast ratio of at least 4.5:1 on every paper
//! it is written on. Read straight from `theme.css`, so a token nudged out of range
//! fails here, not on a reader's screen.

use std::collections::HashMap;

/// AA for normal-size text.
const AA_TEXT: f64 = 4.5;

/// The inks that colour text.
const TEXT_INKS: [&str; 5] = ["ink", "ink-soft", "ink-faint", "accent", "danger"];
/// The papers text in any ink is written on: the page, and the sheets and scraps laid
/// on it (the picker page among them).
const PAPERS: [&str; 2] = ["paper", "paper-raised"];

/// The theme's tokens: the light ones, and the dark ones laid over them.
fn themes() -> [(&'static str, HashMap<String, String>); 2] {
    let css = include_str!("../../../web/theme.css");
    let block = |head: &str| {
        let start = css
            .find(head)
            .unwrap_or_else(|| panic!("theme.css has {head}"));
        let body = &css[start..];
        let open = body.find('{').expect("a block opens");
        let close = body.find('}').expect("a block closes");
        &body[open + 1..close]
    };
    // The first `:root,` opens the light block (line endings differ by checkout).
    let light = tokens(block(":root,"));
    let mut dark = light.clone();
    dark.extend(tokens(block(":root[data-theme=\"dark\"],")));
    [("light", light), ("dark", dark)]
}

/// `--name: value;` declarations, comments dropped.
fn tokens(block: &str) -> HashMap<String, String> {
    // Every `/*` starts a comment that its next `*/` ends.
    let code: String = block
        .split("/*")
        .enumerate()
        .map(|(i, part)| match i {
            0 => part,
            _ => part.split_once("*/").map_or("", |(_, after)| after),
        })
        .collect();
    code.split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.trim().split_once(':')?;
            let name = name.trim().strip_prefix("--")?;
            Some((name.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// The `#rrggbb` a token comes to, following `var(--other)`.
fn colour(tokens: &HashMap<String, String>, name: &str) -> [f64; 3] {
    let value = tokens
        .get(name)
        .unwrap_or_else(|| panic!("--{name} is defined"));
    if let Some(other) = value
        .strip_prefix("var(--")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return colour(tokens, other);
    }
    let hex = value
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6)
        .unwrap_or_else(|| panic!("--{name} is a #rrggbb colour, not {value}"));
    std::array::from_fn(|i| {
        f64::from(u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex digits")) / 255.0
    })
}

/// WCAG relative luminance.
fn luminance(rgb: [f64; 3]) -> f64 {
    let [r, g, b] = rgb.map(|c| {
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG contrast ratio, 1 to 21.
fn ratio(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn the_ratio_is_wcags() {
    assert!((ratio([0.0; 3], [1.0; 3]) - 21.0).abs() < 1e-9);
    assert!((ratio([0.5; 3], [0.5; 3]) - 1.0).abs() < 1e-9);
}

#[test]
fn every_text_ink_meets_aa_on_every_paper_in_both_themes() {
    let failing: Vec<String> = themes()
        .iter()
        .flat_map(|(theme, tokens)| {
            TEXT_INKS.iter().flat_map(move |ink| {
                PAPERS.iter().filter_map(move |paper| {
                    let contrast = ratio(colour(tokens, ink), colour(tokens, paper));
                    (contrast < AA_TEXT)
                        .then(|| format!("{theme}: --{ink} on --{paper} is {contrast:.2}"))
                })
            })
        })
        .collect();
    assert!(failing.is_empty(), "below AA {AA_TEXT}:1: {failing:#?}");
}

#[test]
fn ink_is_aa_on_sunken_paper_the_only_ink_written_there() {
    // A pressed choice or button sinks into ecru and carries only --ink: the other
    // inks never go there, so ecru keeps its depth instead of lightening for them.
    for (theme, tokens) in themes() {
        let contrast = ratio(colour(&tokens, "ink"), colour(&tokens, "paper-sunken"));
        assert!(
            contrast >= AA_TEXT,
            "{theme}: --ink on --paper-sunken is {contrast:.2}"
        );
    }
}

#[test]
fn the_now_line_is_as_readable_as_text_on_the_page() {
    // The planner's now line is a mark, which AA asks only 3:1 of; it is held to text's
    // 4.5 on the papers the planner is drawn on, so it never fades into them.
    for (theme, tokens) in themes() {
        for paper in ["paper", "paper-raised"] {
            let contrast = ratio(colour(&tokens, "now"), colour(&tokens, paper));
            assert!(
                contrast >= AA_TEXT,
                "{theme}: --now on --{paper} is {contrast:.2}"
            );
        }
    }
}
