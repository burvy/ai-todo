//! The v2 API under `/api/v1` (ARCHITECTURE.md sections 6 and 7). It is the whole server.

pub mod admin;
pub mod admin_web;
pub mod app;
pub mod auth;
pub mod backup;
pub mod csp;
pub mod http;
pub mod ics_export;
pub mod ics_import;
pub mod store;
pub mod tuning;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use todo_core::SystemClock;

use app::App;
use store::{SqliteStore, Store};

/// Locking that survives a panic elsewhere. A panic while a lock is held poisons the
/// mutex, and `lock().unwrap()` would then panic on every later request until a restart.
/// What the lock guards is a database handle and an app whose truth lives in the
/// database, so carrying on is safe.
pub trait LockExt<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Starts the daily database backup and the expiry of idle anonymous accounts that
/// follows it. Call once from the server's `main`, never from code that tests run.
///
/// The service opens its own handle on the database, as the admin console does; the
/// database is already migrated by [`app_from_env`], so this open applies nothing.
pub fn start_backups() -> Result<(), String> {
    let store = SqliteStore::open_from_env()
        .map_err(|error| format!("could not open the database for backups: {error}"))?;
    backup::BackupService::start(
        backup::db_path_from_env(),
        Arc::new(store),
        Arc::new(SystemClock),
    );
    Ok(())
}

/// The production app: SQLite at `RAMBL_DATA`, or `data/rambl.db` when that is unset.
/// v2 is on disk by default, so a restart never loses tasks; `MemoryStore` is for tests
/// only.
///
/// A database that will not open is a startup failure, not something to recover from at
/// runtime: serving reads against a store that quietly dropped every write would be worse
/// than refusing to start.
pub fn app_from_env() -> Result<App<Box<dyn Store>>, String> {
    let store = SqliteStore::open_from_env().map_err(|error| {
        let path =
            std::env::var("RAMBL_DATA").unwrap_or_else(|_| store::DEFAULT_DATA_PATH.to_owned());
        format!("could not open the Rambl database at `{path}`: {error}")
    })?;

    Ok(App::new(Box::new(store), Arc::new(SystemClock)))
}
