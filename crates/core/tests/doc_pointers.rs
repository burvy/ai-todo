//! The code pointers in the feature list and the architecture (`path:name, name`) must
//! point at something real: the file exists and each name is in it. A move or a rename
//! that leaves a pointer behind fails here, so the docs cannot drift unnoticed.

use std::fs;
use std::path::{Path, PathBuf};

const DOCS: [&str; 2] = ["docs/FEATURES.md", "docs/ARCHITECTURE.md"];
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
                    .filter(|name| !source.contains(needle(name)))
                    .map(|name| format!("{doc}: {} has no `{name}`", pointer.path)),
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
