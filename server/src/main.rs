mod v2;

use std::net::SocketAddr;
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
    v2::start_backups();
    run_from_exe_folder();
    dotenvy::dotenv().ok();
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => fatal(&format!("could not listen on port {port}: {e}")),
    };
    if std::env::var("TYPESAFE_API_KEY").is_err() {
        println!("note: TYPESAFE_API_KEY is not set (put it in .env next to the exe); Jev is off");
    }
    let store = v2::app_from_env().unwrap_or_else(|problem| fatal(&problem));
    println!("Rambl is running on http://localhost:{port} (close this window to stop it)");
    if let Err(e) = axum::serve(listener, app(store)).await {
        fatal(&format!("server stopped: {e}"));
    }
}

/// A double-clicked exe starts in some other folder; work from the exe's own
/// folder when it holds the release layout, so `data/` and `.env` next to it are
/// found. `cargo run` (no `.env.example` beside the exe) is unaffected.
fn run_from_exe_folder() {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        && dir.join(".env.example").is_file()
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
