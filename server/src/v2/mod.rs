//! v2 API under `/api/v1` (ARCHITECTURE.md sections 6 and 7). Runs alongside the v1
//! routes until phase 4 removes them.

pub mod app;
pub mod auth;
pub mod backup;
pub mod http;
pub mod ics_export;
pub mod store;

use std::sync::Arc;

use todo_core::SystemClock;

use app::App;
use backup::BackupService;
use store::{SqliteStore, Store};

/// The production app: SQLite at `RAMBL_DATA`, or `data/rambl.db` when that is
/// unset. v2 is on disk by default, so a restart never loses tasks; [`MemoryStore`]
/// is for tests only.
///
/// A database that will not open is a startup failure, not something to recover
/// from at runtime: serving reads against a store that quietly dropped every
/// write would be worse than refusing to start.
pub fn app_from_env() -> Result<App<Box<dyn Store>>, String> {
    let store = SqliteStore::open_from_env().map_err(|error| {
        let path =
            std::env::var("RAMBL_DATA").unwrap_or_else(|_| store::DEFAULT_DATA_PATH.to_owned());
        format!("could not open the Rambl database at `{path}`: {error}")
    })?;

    // Start the daily backup service
    let db_path = backup::db_path_from_env();
    BackupService::start(db_path, Arc::new(SystemClock));

    Ok(App::new(Box::new(store), Arc::new(SystemClock)))
}
