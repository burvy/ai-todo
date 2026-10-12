//! Daily SQLite backups.
//!
//! At startup and every 24 hours, the database is copied with `VACUUM INTO`
//! to `data/backups/rambl-YYYY-MM-DD.db`. Backups older than 14 days are deleted.
//! Once the backup is safely written, anonymous accounts idle for 30 days are deleted.
//! Failures are logged and never crash the server.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Duration as Span, Local, Utc};
use rusqlite::Connection;
use tokio::sync::Mutex;
use tokio::time;

use todo_core::Clock;

use super::store::Store;

/// Where backups live, relative to the data directory.
const BACKUP_DIR: &str = "backups";

/// Backup file name pattern: rambl-YYYY-MM-DD.db
fn backup_filename(date: chrono::NaiveDate) -> String {
    format!("rambl-{}.db", date.format("%Y-%m-%d"))
}

/// An anonymous account unused for this long is deleted with its tasks.
const ANONYMOUS_IDLE_LIMIT: Span = Span::days(30);

/// The backup service. Holds the database path, the store whose idle anonymous
/// accounts it clears, and a clock for testing.
pub struct BackupService {
    db_path: PathBuf,
    store: Arc<dyn Store>,
    clock: Arc<dyn Clock>,
    state: Mutex<BackupState>,
}

/// Shared state for the background task.
struct BackupState {
    last_run: Option<DateTime<Utc>>,
}

impl BackupService {
    /// Start the service. `db_path` is the path to the SQLite database to back up.
    pub fn start(db_path: PathBuf, store: Arc<dyn Store>, clock: Arc<dyn Clock>) -> Arc<Self> {
        let state = Mutex::new(BackupState { last_run: None });
        let service = Arc::new(Self {
            db_path,
            store,
            clock,
            state,
        });
        Self::spawn(service.clone());
        service
    }

    /// Spawn the background task that runs the first backup and then every 24 hours.
    fn spawn(service: Arc<Self>) {
        tokio::spawn(async move {
            Self::run_backup(&service).await;
            let mut interval = time::interval(Duration::from_secs(24 * 60 * 60));
            loop {
                interval.tick().await;
                Self::run_backup(&service).await;
            }
        });
    }

    /// Run one cycle: copy the database, clean up old backups, then expire idle
    /// anonymous accounts.
    async fn run_backup(service: &Arc<Self>) {
        let db_path = service.db_path.clone();
        let store = service.store.clone();
        let clock = service.clock.clone();
        let now = clock.now();

        let result =
            tokio::task::spawn_blocking(move || run_cycle_sync(&db_path, store.as_ref(), now))
                .await;

        match result {
            Ok(Ok(())) => {
                let mut state = service.state.lock().await;
                state.last_run = Some(clock.now());
            }
            Ok(Err(e)) => {
                eprintln!("Backup failed: {}", e);
            }
            Err(join_err) => {
                eprintln!("Backup task panicked: {}", join_err);
            }
        }
    }
}

/// One cycle, in order: backup first, expiry only if the backup succeeded, so an
/// account is never deleted without a copy of the database holding it.
fn run_cycle_sync(db_path: &Path, store: &dyn Store, now: DateTime<Utc>) -> Result<(), String> {
    run_backup_sync(db_path, now.with_timezone(&Local))?;
    expire_idle_anonymous(store, now)?;
    Ok(())
}

/// Delete anonymous accounts not seen for [`ANONYMOUS_IDLE_LIMIT`], with their tasks.
/// Signed-in accounts are never expired. Returns how many accounts were deleted.
fn expire_idle_anonymous(store: &dyn Store, now: DateTime<Utc>) -> Result<usize, String> {
    store
        .expire_anonymous(now - ANONYMOUS_IDLE_LIMIT)
        .map_err(|error| format!("expiring idle anonymous accounts: {error}"))
}

/// Execute the backup synchronously. Returns an error description, never panics.
fn run_backup_sync(db_path: &Path, now: DateTime<Local>) -> Result<(), String> {
    let backup_dir = db_path
        .parent()
        .ok_or("db_path has no parent")?
        .join(BACKUP_DIR);

    std::fs::create_dir_all(&backup_dir).map_err(|e| format!("creating backup dir: {e}"))?;

    let today = now.date_naive();
    let backup_file = backup_dir.join(backup_filename(today));

    // Remove existing backup file from today if it exists (VACUUM INTO fails if file exists)
    let _ = std::fs::remove_file(&backup_file);

    let conn = Connection::open(db_path).map_err(|e| format!("opening db for backup: {e}"))?;

    let target = backup_file.to_string_lossy().replace('\'', "''");
    conn.execute(&format!("VACUUM INTO '{target}'"), [])
        .map_err(|e| format!("VACUUM INTO failed: {e}"))?;

    delete_old_backups(&backup_dir, today)?;

    Ok(())
}

/// Delete backups older than 14 days from the backup directory.
fn delete_old_backups(backup_dir: &Path, today: chrono::NaiveDate) -> Result<(), String> {
    let cutoff = today - chrono::Duration::days(14);

    let entries = std::fs::read_dir(backup_dir)
        .map_err(|e| format!("reading backup dir for cleanup: {e}"))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("reading backup dir entry: {e}"))?;
        let path = entry.path();

        if let Some(filename) = path.file_name().and_then(|n| n.to_str())
            && filename.starts_with("rambl-")
            && filename.ends_with(".db")
        {
            let date_str = &filename[6..16];
            if let Ok(backup_date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                && backup_date < cutoff
            {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("removing old backup {}: {}", path.display(), e))?;
            }
        }
    }

    Ok(())
}

/// Get the database path from the environment or use the default.
pub fn db_path_from_env() -> PathBuf {
    let path = std::env::var("RAMBL_DATA").unwrap_or_else(|_| "data/rambl.db".to_string());
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::store::SqliteStore;
    use chrono::NaiveDate;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn backup_creates_copy_and_opens_as_valid_db() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE test (id INTEGER)", []).unwrap();
        conn.execute("INSERT INTO test VALUES (1)", []).unwrap();
        drop(conn);

        let now = Local::now();
        run_backup_sync(&db_path, now).unwrap();

        let backup_dir = db_path.parent().unwrap().join(BACKUP_DIR);
        let backup_file = backup_dir.join(backup_filename(now.date_naive()));
        assert!(backup_file.exists(), "backup file should exist");

        let backup_conn = Connection::open(&backup_file).unwrap();
        let count: i64 = backup_conn
            .query_row("SELECT COUNT(*) FROM test", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "backup should contain the data");
    }

    #[test]
    fn delete_old_backups_removes_files_older_than_14_days() {
        let dir = tempdir().unwrap();
        let backup_dir = dir.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let old_date = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        let recent_date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();

        let old_backup = backup_dir.join(backup_filename(old_date));
        let recent_backup = backup_dir.join(backup_filename(recent_date));
        fs::File::create(&old_backup).unwrap();
        fs::File::create(&recent_backup).unwrap();

        delete_old_backups(&backup_dir, today).unwrap();

        assert!(!old_backup.exists(), "old backup should be deleted");
        assert!(recent_backup.exists(), "recent backup should be kept");
    }

    #[test]
    fn delete_old_backups_keeps_files_on_exact_14th_day() {
        let dir = tempdir().unwrap();
        let backup_dir = dir.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let exactly_14_days_ago = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();

        let backup_file = backup_dir.join(backup_filename(exactly_14_days_ago));
        fs::File::create(&backup_file).unwrap();

        delete_old_backups(&backup_dir, today).unwrap();

        assert!(
            backup_file.exists(),
            "backup exactly 14 days old should be kept"
        );
    }

    #[test]
    fn backup_on_same_day_does_not_fail() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE test (id INTEGER)", []).unwrap();
        conn.execute("INSERT INTO test VALUES (1)", []).unwrap();
        drop(conn);

        let now = Local::now();
        run_backup_sync(&db_path, now).unwrap();
        run_backup_sync(&db_path, now).unwrap();

        let backup_dir = db_path.parent().unwrap().join(BACKUP_DIR);
        let backup_file = backup_dir.join(backup_filename(now.date_naive()));
        assert!(backup_file.exists(), "backup should exist after second run");
    }

    #[test]
    fn backup_filename_format() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        assert_eq!(backup_filename(date), "rambl-2026-10-12.db");
    }

    fn anon_id(n: u32) -> String {
        format!("anon_{n:032x}")
    }

    /// A file database holding one idle and one recent anonymous account, each with a task.
    fn store_with_idle_and_recent(dir: &Path, now: DateTime<Utc>) -> (PathBuf, SqliteStore) {
        let path = dir.join("rambl.db");
        let store = SqliteStore::open(&path).unwrap();
        for (id, seen) in [
            (anon_id(1), now - Span::days(31)),
            (anon_id(2), now - Span::days(2)),
        ] {
            store.ensure_anonymous(&id, seen).unwrap();
            store.put(&id, sample_task(&seen)).unwrap();
        }
        (path, store)
    }

    fn sample_task(at: &DateTime<Utc>) -> todo_core::Task {
        todo_core::Task {
            id: todo_core::TaskId(format!("t{}", at.timestamp())),
            title: "Task".into(),
            duration_min: 30,
            when: todo_core::When::Anytime,
            done: false,
            order: 0,
            label: todo_core::Label::Other,
            priority: todo_core::Priority::Low,
            source: todo_core::Source::Text,
            created: *at,
            updated_at: *at,
            external_uid: None,
        }
    }

    #[test]
    fn a_cycle_backs_up_first_then_deletes_idle_anonymous_accounts() {
        let dir = tempdir().unwrap();
        let now = Utc::now();
        let (path, store) = store_with_idle_and_recent(dir.path(), now);

        run_cycle_sync(&path, &store, now).unwrap();

        assert!(
            !store.anonymous_exists(&anon_id(1)).unwrap(),
            "idle account kept"
        );
        assert!(store.tasks(&anon_id(1)).unwrap().is_empty());
        assert!(
            store.anonymous_exists(&anon_id(2)).unwrap(),
            "recent account lost"
        );
        // The backup was taken before the deletion, so it still holds the idle account.
        let backup = path
            .parent()
            .unwrap()
            .join(BACKUP_DIR)
            .join(backup_filename(now.with_timezone(&Local).date_naive()));
        let copy = SqliteStore::open(&backup).unwrap();
        assert!(copy.anonymous_exists(&anon_id(1)).unwrap());
    }

    #[test]
    fn nothing_is_deleted_when_the_backup_fails() {
        let dir = tempdir().unwrap();
        let now = Utc::now();
        let (path, store) = store_with_idle_and_recent(dir.path(), now);
        // A file where the backup folder should be makes the backup fail.
        fs::write(dir.path().join(BACKUP_DIR), b"not a folder").unwrap();

        assert!(run_cycle_sync(&path, &store, now).is_err());
        assert!(
            store.anonymous_exists(&anon_id(1)).unwrap(),
            "an account must not be deleted without a backup"
        );
    }
}
