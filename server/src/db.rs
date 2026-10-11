//! SQLite persistence for tasks and busy blocks.
//!
//! Everything that touches SQLite lives here so that `main.rs` only needs a few
//! one-line calls at the existing mutation points. The in-memory `Store` stays
//! the working copy; this module is the durable record.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;
use todo_core::{Source, Task};

/// Default location, overridable with the `DATA_PATH` env var.
pub const DEFAULT_DATA_PATH: &str = "data/ai-todo.db";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    notes       TEXT NOT NULL DEFAULT '',
    duration_min INTEGER NOT NULL DEFAULT 30,
    deadline    TEXT,
    priority    INTEGER NOT NULL DEFAULT 0,
    status      TEXT NOT NULL DEFAULT 'todo',
    confidence  REAL NOT NULL DEFAULT 0.5,
    source      TEXT NOT NULL DEFAULT 'text',
    label       TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS busy_blocks (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    start TEXT NOT NULL,
    end   TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT ''
);
";

/// A persisted busy block. The server maps this onto its own response type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusyRow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub title: String,
}

/// `Connection` is `Send` but not `Sync`, so it is held behind a `Mutex` to
/// make `Db` usable as shared axum state.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// Open (creating if needed) the database at `DATA_PATH`, defaulting to
    /// `data/ai-todo.db`.
    ///
    /// `DATA_PATH=:memory:` opens a throwaway in-memory database, which is what
    /// the tests use.
    pub fn open_from_env() -> rusqlite::Result<Self> {
        let configured =
            std::env::var("DATA_PATH").unwrap_or_else(|_| DEFAULT_DATA_PATH.to_string());
        Self::open(Path::new(&configured))
    }

    /// Open the database at `path`, creating the parent folder if needed.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if path != Path::new(":memory:")
            && let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        }
        let db = Self {
            conn: Mutex::new(Connection::open(path)?),
        };
        db.migrate()?;
        Ok(db)
    }

    /// A migrated database that lives only as long as this handle.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let db = Self {
            conn: Mutex::new(Connection::open_in_memory()?),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        self.conn.lock().expect("db mutex").execute_batch(SCHEMA)
    }

    // -- write-through helpers ------------------------------------------
    //
    // These are what the handlers call. A persistence failure is logged rather
    // than surfaced as a 500: during a live demo a database hiccup should be
    // visible in the server log without taking the whole API down.

    pub fn save_tasks(&self, tasks: &[Task]) {
        report(self.insert_tasks(tasks), "captured tasks");
    }

    pub fn save_task(&self, task: &Task) {
        report(self.update_task(task), &format!("task {}", task.id));
    }

    pub fn remove_task(&self, id: &str) {
        report(self.delete_task(id), &format!("deleted task {id}"));
    }

    pub fn save_busy(&self, rows: &[BusyRow]) {
        report(self.replace_busy(rows), "busy blocks");
    }

    /// Delete every stored task and busy block (used by `POST /api/reset`).
    pub fn clear(&self) {
        let result = self
            .conn
            .lock()
            .expect("db mutex")
            .execute_batch("DELETE FROM tasks; DELETE FROM busy_blocks;");
        report(result, "reset");
    }

    // -- tasks ----------------------------------------------------------

    /// Every task, oldest id first, so `GET /api/tasks` keeps a stable order.
    pub fn load_tasks(&self) -> rusqlite::Result<Vec<Task>> {
        let conn = self.conn.lock().expect("db mutex");
        let mut stmt = conn.prepare(
            "SELECT id, title, notes, duration_min, deadline, priority, status,
                    confidence, source, label
             FROM tasks ORDER BY CAST(id AS INTEGER) ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            // Column order: id, title, notes, duration_min, deadline,
            // priority, status, confidence, source, label.
            let deadline: Option<String> = row.get(4)?;
            let source: String = row.get(8)?;
            Ok(Task {
                id: row.get(0)?,
                title: row.get(1)?,
                notes: row.get(2)?,
                duration_min: row.get(3)?,
                deadline: deadline.as_deref().and_then(parse_ts),
                priority: row.get(5)?,
                status: row.get(6)?,
                confidence: row.get(7)?,
                source: parse_source(&source),
                label: row.get(9)?,
            })
        })?;
        rows.collect()
    }

    /// Insert tasks, ignoring ids that are already present so a repeated
    /// capture can never duplicate or clobber an existing row.
    fn insert_tasks(&self, tasks: &[Task]) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db mutex");
        let mut stmt = conn.prepare(
            "INSERT OR IGNORE INTO tasks
                (id, title, notes, duration_min, deadline, priority, status, confidence, source, label)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for task in tasks {
            stmt.execute(params![
                task.id,
                task.title,
                task.notes,
                task.duration_min,
                task.deadline.map(format_ts),
                task.priority,
                task.status,
                task.confidence,
                source_str(task.source),
                task.label,
            ])?;
        }
        Ok(())
    }

    fn update_task(&self, task: &Task) -> rusqlite::Result<()> {
        self.conn.lock().expect("db mutex").execute(
            "UPDATE tasks SET title = ?2, notes = ?3, duration_min = ?4, deadline = ?5,
                    priority = ?6, status = ?7, confidence = ?8, source = ?9, label = ?10
             WHERE id = ?1",
            params![
                task.id,
                task.title,
                task.notes,
                task.duration_min,
                task.deadline.map(format_ts),
                task.priority,
                task.status,
                task.confidence,
                source_str(task.source),
                task.label,
            ],
        )?;
        Ok(())
    }

    fn delete_task(&self, id: &str) -> rusqlite::Result<()> {
        self.conn
            .lock()
            .expect("db mutex")
            .execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(())
    }

    // -- busy blocks ---------------------------------------------------

    /// Every stored busy block, in chronological order.
    pub fn load_busy(&self) -> rusqlite::Result<Vec<BusyRow>> {
        let conn = self.conn.lock().expect("db mutex");
        let mut stmt =
            conn.prepare("SELECT start, end, title FROM busy_blocks ORDER BY start ASC")?;
        let rows = stmt.query_map([], |row| {
            let start: String = row.get(0)?;
            let end: String = row.get(1)?;
            Ok(BusyRow {
                start: parse_ts(&start).unwrap_or_else(Utc::now),
                end: parse_ts(&end).unwrap_or_else(Utc::now),
                title: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Make the stored blocks exactly `rows`, as one transaction.
    ///
    /// Wholesale replacement keeps the server's busy list authoritative: the
    /// in-memory vector is small, and this way cancelling an event needs no id
    /// bookkeeping in the handlers.
    fn replace_busy(&self, rows: &[BusyRow]) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().expect("db mutex");
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM busy_blocks", [])?;
        {
            let mut stmt =
                tx.prepare("INSERT INTO busy_blocks (start, end, title) VALUES (?1, ?2, ?3)")?;
            for row in rows {
                stmt.execute(params![format_ts(row.start), format_ts(row.end), row.title])?;
            }
        }
        tx.commit()
    }
}

/// SQLite has no date type; UTC timestamps are stored as RFC 3339 text.
fn format_ts(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339()
}

/// Log a failed write instead of propagating it into the request path.
fn report(result: rusqlite::Result<()>, what: &str) {
    if let Err(error) = result {
        eprintln!("warning: {what} was not saved to SQLite: {error}");
    }
}

fn parse_ts(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn source_str(source: Source) -> &'static str {
    match source {
        Source::Voice => "voice",
        Source::Pdf => "pdf",
        Source::Text => "text",
    }
}

/// Unknown values fall back to `Text` rather than failing the whole load, so a
/// hand-edited database cannot stop the server from starting.
fn parse_source(text: &str) -> Source {
    match text {
        "voice" => Source::Voice,
        "pdf" => Source::Pdf,
        _ => Source::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, hour, 0, 0).unwrap()
    }

    fn task(id: &str, title: &str) -> Task {
        Task {
            id: id.into(),
            title: title.into(),
            notes: "some notes".into(),
            duration_min: 90,
            deadline: Some(at(17)),
            priority: 3,
            status: "todo".into(),
            confidence: 0.87,
            label: "School".into(),
            source: Source::Voice,
        }
    }

    #[test]
    fn round_trips_a_task_with_every_field() {
        let db = Db::open_in_memory().unwrap();
        let original = task("t1", "File the tax return");
        db.insert_tasks(std::slice::from_ref(&original)).unwrap();

        let loaded = db.load_tasks().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded[0], original,
            "every field must survive the round trip"
        );
    }

    #[test]
    fn round_trips_a_task_with_no_deadline() {
        let db = Db::open_in_memory().unwrap();
        let mut original = task("t2", "Water the plants");
        original.deadline = None;
        original.confidence = 0.0;
        db.insert_tasks(std::slice::from_ref(&original)).unwrap();

        let loaded = db.load_tasks().unwrap();
        assert_eq!(loaded[0].deadline, None);
        assert_eq!(loaded[0].confidence, 0.0);
    }

    #[test]
    fn survives_reopening_the_same_file() {
        let dir = std::env::temp_dir().join(format!("ai-todo-test-{}", std::process::id()));
        let path = dir.join("nested").join("ai-todo.db");
        let _ = std::fs::remove_dir_all(&dir);

        {
            let db = Db::open(&path).unwrap();
            db.insert_tasks(&[task("t1", "Persist me")]).unwrap();
        }

        // Fresh handle, same file: this is the restart case.
        let db = Db::open(&path).unwrap();
        let loaded = db.load_tasks().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "Persist me");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_and_delete_persist() {
        let db = Db::open_in_memory().unwrap();
        db.insert_tasks(&[task("t1", "Before")]).unwrap();

        let mut changed = task("t1", "After");
        changed.status = "done".into();
        db.update_task(&changed).unwrap();
        let loaded = db.load_tasks().unwrap();
        assert_eq!(loaded[0].title, "After");
        assert_eq!(loaded[0].status, "done");

        db.delete_task("t1").unwrap();
        assert!(db.load_tasks().unwrap().is_empty());
    }

    #[test]
    fn inserting_the_same_id_twice_does_not_duplicate() {
        let db = Db::open_in_memory().unwrap();
        db.insert_tasks(&[task("t1", "Once")]).unwrap();
        db.insert_tasks(&[task("t1", "Once")]).unwrap();
        assert_eq!(db.load_tasks().unwrap().len(), 1);
    }

    #[test]
    fn loads_in_insertion_order() {
        let db = Db::open_in_memory().unwrap();
        db.insert_tasks(&[task("t3", "c"), task("t1", "a"), task("t2", "b")])
            .unwrap();
        let ids: Vec<_> = db.load_tasks().unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["t1", "t2", "t3"]);
    }

    #[test]
    fn busy_blocks_round_trip() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.load_busy().unwrap().is_empty());

        let seeded = vec![
            BusyRow {
                start: at(9),
                end: at(10),
                title: "Standup".into(),
            },
            BusyRow {
                start: at(12),
                end: at(13),
                title: "Lunch".into(),
            },
        ];
        db.replace_busy(&seeded).unwrap();
        assert!(!db.load_busy().unwrap().is_empty());
        assert_eq!(db.load_busy().unwrap(), seeded);
    }

    #[test]
    fn replace_busy_makes_the_stored_set_exactly_match() {
        let db = Db::open_in_memory().unwrap();
        db.replace_busy(&[
            BusyRow {
                start: at(9),
                end: at(10),
                title: "Standup".into(),
            },
            BusyRow {
                start: at(12),
                end: at(13),
                title: "Lunch".into(),
            },
            BusyRow {
                start: at(15),
                end: at(16),
                title: "1:1".into(),
            },
        ])
        .unwrap();

        // The user cancels the 3pm event.
        let kept = vec![
            BusyRow {
                start: at(9),
                end: at(10),
                title: "Standup".into(),
            },
            BusyRow {
                start: at(12),
                end: at(13),
                title: "Lunch".into(),
            },
        ];
        db.replace_busy(&kept).unwrap();

        let loaded = db.load_busy().unwrap();
        assert_eq!(loaded.len(), 2, "the cancelled event must be gone");
        assert!(!loaded.iter().any(|b| b.title == "1:1"));
    }

    #[test]
    fn replace_busy_with_nothing_empties_the_table() {
        let db = Db::open_in_memory().unwrap();
        db.replace_busy(&[BusyRow {
            start: at(9),
            end: at(10),
            title: "Standup".into(),
        }])
        .unwrap();
        db.replace_busy(&[]).unwrap();
        assert!(db.load_busy().unwrap().is_empty());
    }

    #[test]
    fn unknown_source_degrades_instead_of_failing() {
        let db = Db::open_in_memory().unwrap();
        db.conn
            .lock()
            .expect("db mutex")
            .execute(
                "INSERT INTO tasks (id, title, source) VALUES ('t9', 'Odd', 'quantum')",
                [],
            )
            .unwrap();
        assert_eq!(db.load_tasks().unwrap()[0].source, Source::Text);
    }
}
