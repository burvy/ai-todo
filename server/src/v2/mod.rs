//! v2 API under `/api/v1` (ARCHITECTURE.md sections 6 and 7). Runs alongside the v1
//! routes until phase 4 removes them.

pub mod app;
pub mod http;
pub mod ics_export;
pub mod store;

use std::sync::Arc;

use todo_core::SystemClock;

use app::App;
use store::{MemoryStore, SqliteStore, Store};

/// The production app: SQLite at `RAMBL_DATA` when set, otherwise in memory.
pub fn app_from_env() -> App<Box<dyn Store>> {
    let store: Box<dyn Store> = match std::env::var("RAMBL_DATA") {
        Ok(path) => Box::new(SqliteStore::open(&path).expect("open RAMBL_DATA database")),
        Err(_) => Box::new(MemoryStore::default()),
    };
    App::new(store, Arc::new(SystemClock))
}
