//! The code pointers in the docs (`path:name, name` or `path:line`) must point at
//! something real: the file exists, each name is in it as a whole word, and a line number
//! is inside the file. A move or a rename
//! that leaves a pointer behind fails here, so the docs cannot drift unnoticed.

use std::fs;
use std::path::{Path, PathBuf};

const DOCS: [&str; 4] = [
    "docs/FEATURES.md",
    "docs/ARCHITECTURE.md",
    "docs/TUNING.md",
    "docs/jev.md",
];
const ROOTS: [&str; 5] = ["crates/", "server/", "web/", "tools/", "docs/"];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A pointer: a repo path and the names the docs say are in it.
struct Pointer<'a> {
    path: &'a str,
    names: Vec<&'a str>,
}

/// The pointers in `text`: every backtick span that starts with a repo folder.
fn pointers(text: &str) -> Vec<Pointer<'_>> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| ROOTS.iter().any(|root| span.starts_with(root)))
        .filter_map(|span| {
            let (path, names) = span.split_once(':').unwrap_or((span, ""));
            let plain = path
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c));
            plain.then(|| Pointer {
                path,
                names: names
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty() && !name.contains([' ', '*']))
                    .collect(),
            })
        })
        .collect()
}

/// The part of `name` to look for: `Type::method` is found by its last part.
fn needle(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

/// Whether `source` has `word` as a whole identifier (not as part of a longer one).
fn has_word(source: &str, word: &str) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    source.match_indices(word).any(|(at, _)| {
        !source[..at].ends_with(ident) && !source[at + word.len()..].starts_with(ident)
    })
}

/// Why `name` does not point at anything in `source`, if it does not. A number is a line.
fn miss(source: &str, name: &str) -> Option<String> {
    match name.parse::<usize>() {
        Ok(line) => {
            (line == 0 || line > source.lines().count()).then(|| format!("has no line {name}"))
        }
        Err(_) => (!has_word(source, needle(name))).then(|| format!("has no `{name}`")),
    }
}

#[test]
fn every_code_pointer_in_the_docs_points_at_something_real() {
    let root = repo();
    let mut stale = Vec::new();
    for doc in DOCS {
        let text = fs::read_to_string(root.join(doc)).expect("the doc is there");
        for pointer in pointers(&text) {
            let path = root.join(pointer.path);
            if !path.exists() {
                stale.push(format!("{doc}: no file {}", pointer.path));
                continue;
            }
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            stale.extend(
                pointer
                    .names
                    .iter()
                    .filter_map(|name| miss(&source, name))
                    .map(|why| format!("{doc}: {} {why}", pointer.path)),
            );
        }
    }
    assert!(
        stale.is_empty(),
        "stale code pointers (fix the doc, or move the pointer with the code):\n{}",
        stale.join("\n")
    );
}

#[test]
fn a_pointer_is_a_repo_path_and_its_names() {
    let found = pointers("see `web/src/a.rs:One, Two::three` and `cargo test` and `docs/X.md`");
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].path, "web/src/a.rs");
    assert_eq!(found[0].names, ["One", "Two::three"]);
    assert_eq!(needle("Two::three"), "three");
    assert!(found[1].names.is_empty());
}

#[test]
fn a_name_matches_whole_words_and_a_line_must_exist() {
    let source = "fn fall_in() {}
let x = 1;
";
    assert!(miss(source, "fall_in").is_none());
    assert!(
        miss(source, "fall").is_some(),
        "a part of a longer name is not the name"
    );
    assert!(miss(source, "Type::fall_in").is_none());
    assert!(miss(source, "2").is_none());
    assert!(miss(source, "3").is_some());
    assert!(miss(source, "0").is_some());
}
