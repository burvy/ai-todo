use std::fs;
use std::path::Path;

fn main() {
    let dist = Path::new("../web/dist");
    if !dist.exists() {
        fs::create_dir_all(dist).expect("create web/dist for server build");
        fs::write(
            dist.join("index.html"),
            "<!doctype html><title>Rambl</title>",
        )
        .expect("create placeholder web/dist/index.html");
    }
    println!("cargo:rerun-if-changed=../web/dist");
}
