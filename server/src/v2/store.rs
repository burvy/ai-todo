//! Persistence for v2 tasks (`docs/ARCHITECTURE.md` section 7).
//!
//! The server holds a `Store`, not a database. [`MemoryStore`] and
//! [`SqliteStore`] satisfy the same [`Store`] trait and the same test suite
//! runs against both, so the HTTP tests never touch a disk.
//!
//! v2 tables are named `v2_*` and live in their own file (`RAMBL_DATA`). They
//! deliberately share no table with v1's `tasks` / `busy_blocks`, so pointing
//! both at one file cannot corrupt the old data.

#![allow(dead_code)] // not wired into the router yet: App and handlers are next

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use todo_core::{Label, Priority, Source, Task, TaskId, When};

fn parse_plan(value: &str) -> super::auth::Plan {
    match value {
        "anon" => super::auth::Plan::Anon,
        "paid" => super::auth::Plan::Paid,
        _ => super::auth::Plan::Free,
    }
}

/// Where the v2 database lives when `RAMBL_DATA` is not set.
pub const DEFAULT_DATA_PATH: &str = "data/rambl.db";

/// Bumped whenever [`MIGRATIONS`] grows an entry. The count on disk is
/// `PRAGMA user_version`; anything below this has the remaining migrations
/// applied on open.
const SCHEMA_VERSION: i64 = 3;

/// Ordered by `user_version`. Entry n applies to a database sitting at
/// version n - 1, so the list is append-only and never reordered.
const MIGRATIONS: &[&str] = &[MIGRATION_1, MIGRATION_2, MIGRATION_3];

const MIGRATION_1: &str = "
CREATE TABLE IF NOT EXISTS v2_tasks (
    id           TEXT PRIMARY KEY,
    title        TEXT NOT NULL,
    duration_min INTEGER NOT NULL,
    -- `When` is a tagged enum on the wire ({\"kind\": ..., \"time\": ...});
    -- in SQL it is the kind plus the instant it may carry.
    when_kind    TEXT NOT NULL,
    when_time    TEXT,
    done         INTEGER NOT NULL DEFAULT 0,
    \"order\"      INTEGER NOT NULL DEFAULT 0,
    label        TEXT NOT NULL,
    priority     TEXT NOT NULL,
    source       TEXT NOT NULL,
    created      TEXT NOT NULL,
    -- iCalendar UID of an imported event; NULL for everything else.
    external_uid TEXT
);
CREATE INDEX IF NOT EXISTS v2_tasks_order_idx ON v2_tasks (\"order\");
-- Re-importing the same UID must find the same task, so a UID can never be
-- held by two rows. Partial, because most tasks have no UID at all.
CREATE UNIQUE INDEX IF NOT EXISTS v2_tasks_uid_idx
    ON v2_tasks (external_uid) WHERE external_uid IS NOT NULL;
";

/// Accounts. A task belongs to a `user_id`; every read and write is scoped to one, so
/// two people never see each other's tasks. Existing rows get `LOCAL_USER` so a
/// database written before sign-in keeps its tasks instead of becoming invisible.
const MIGRATION_2: &str = "
CREATE TABLE IF NOT EXISTS v2_users (
    id         TEXT PRIMARY KEY,
    email      TEXT NOT NULL,
    name       TEXT NOT NULL,
    picture    TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS v2_sessions (
    token      TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS v2_sessions_user_idx ON v2_sessions (user_id);
-- Tasks belong to a user. SQLite cannot add a NOT NULL column with a REFERENCES
-- constraint and a default in one statement, so `delete_user` removes the tasks in
-- the same transaction instead of leaning on the engine to cascade.
ALTER TABLE v2_tasks ADD COLUMN user_id TEXT NOT NULL DEFAULT 'local';
CREATE INDEX IF NOT EXISTS v2_tasks_user_idx ON v2_tasks (user_id);
INSERT OR IGNORE INTO v2_users (id, email, name, picture, created_at)
    VALUES ('local', 'local@localhost', 'Local user', NULL, '1970-01-01T00:00:00Z');
";
const MIGRATION_3: &str = "ALTER TABLE v2_users ADD COLUMN plan TEXT NOT NULL DEFAULT 'free';";

/// Every column of `v2_tasks`, in one list.
///
/// Reads and writes are built from this string, so a field added here cannot
/// be added to the `SELECT` and forgotten in the `INSERT`. The index below is
/// checked against it by a test.
const TASK_COLUMNS: &str = "id, title, duration_min, when_kind, when_time, done, \"order\", \
                            label, priority, source, created, external_uid";

/// A store that could not answer.
///
/// Reads and writes are both fallible here: the caller is `App`, which runs a
/// load-mutate-plan inside one critical section, and it needs to know the
/// difference between "no such task" and "the database is unhappy".
#[derive(Debug)]
pub enum StoreError {
    /// No task with that id (or, for `find_by_uid`, no task with that UID).
    NotFound,
    /// The write was refused: most often two tasks claiming one
    /// `external_uid`.
    Conflict(String),
    /// The database itself failed.
    Backend(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound => write!(f, "no such task"),
            StoreError::Conflict(why) => write!(f, "conflict: {why}"),
            StoreError::Backend(why) => write!(f, "storage failed: {why}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Persistence for tasks.
///
/// Implementors must be safe to share across axum's worker threads.
pub trait Store: Send + Sync {
    /// Every task belonging to this user, in display order.
    ///
    /// The user id is a parameter rather than something the caller passes to a
    /// filter afterwards, so a handler cannot forget it and show one person's tasks to
    /// another.
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError>;

    /// Store `task` under its own id for this user, replacing any task already there.
    ///
    /// `put` is an upsert so it is total: the caller does not have to know
    /// whether the id is new. Use [`Store::find_by_uid`] before putting an
    /// imported task, so a re-import updates that task instead of colliding
    /// with it.
    fn put(&self, user: &str, task: Task) -> Result<(), StoreError>;

    /// Forget this user's task with that id. Reports whether there was one, so the
    /// caller can answer 404 rather than always 204.
    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError>;

    /// Make this user's order exactly `ids`, first to `0`, `1`, ... Ids that are not
    /// stored are skipped. One transaction, so a reorder never half-applies.
    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError>;

    /// This user's task imported from this iCalendar UID, if there is one.
    ///
    /// This is how a re-import updates instead of duplicating.
    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError>;

    /// Create or refresh the user and open a session, in one transaction.
    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError>;

    /// End one session. Signing out is not signing in everywhere else.
    fn delete_session(&self, token: &str) -> Result<(), StoreError>;

    /// The user a live session token belongs to. `None` once it has expired.
    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<super::auth::Me>, StoreError>;

    /// Forget a person, their sessions and every task they own.
    fn delete_user(&self, user: &str) -> Result<(), StoreError>;

    /// How many captures this user has run today, for the daily Jev cap.
    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError>;

    /// Move anonymous tasks to a signed-in user, keeping the signed-in copy for
    /// duplicate external UIDs.
    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError>;
    fn user_plan(&self, user: &str) -> Result<super::auth::Plan, StoreError>;
    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError>;
    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError>;
    fn ensure_anonymous(&self, user: &str) -> Result<(), StoreError>;
}

/// Tasks held in memory, with their owner. The HTTP tests use this so they never
/// touch a disk, and it has to model ownership exactly as the SQL store does or the
/// tests would not be testing the scoping that matters.
#[derive(Default)]
pub struct MemoryStore {
    tasks: Mutex<Vec<(String, Task)>>,
    users: Mutex<Vec<super::auth::Me>>,
    sessions: Mutex<Vec<(String, String, chrono::DateTime<Utc>)>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Forwards to whichever store was chosen. This is how `App` stays generic
/// over "SQLite or memory" without naming either.
impl<S: Store + ?Sized> Store for Box<S> {
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError> {
        (**self).tasks(user)
    }

    fn put(&self, user: &str, task: Task) -> Result<(), StoreError> {
        (**self).put(user, task)
    }

    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
        (**self).delete(user, id)
    }

    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        (**self).set_order(user, ids)
    }

    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError> {
        (**self).find_by_uid(user, uid)
    }

    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        (**self).upsert_user_and_session(user, token, expires_at)
    }

    fn delete_session(&self, token: &str) -> Result<(), StoreError> {
        (**self).delete_session(token)
    }

    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<super::auth::Me>, StoreError> {
        (**self).session_user(token, now)
    }

    fn delete_user(&self, user: &str) -> Result<(), StoreError> {
        (**self).delete_user(user)
    }

    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
        (**self).captures_today(user, day)
    }

    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
        (**self).adopt_anonymous(anonymous, user)
    }
    fn user_plan(&self, user: &str) -> Result<super::auth::Plan, StoreError> {
        (**self).user_plan(user)
    }
    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError> {
        (**self).set_user_plan(user, plan)
    }
    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
        (**self).anonymous_exists(user)
    }
    fn ensure_anonymous(&self, user: &str) -> Result<(), StoreError> {
        (**self).ensure_anonymous(user)
    }
}

impl Store for MemoryStore {
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError> {
        let mut stored: Vec<Task> = self
            .tasks
            .lock()
            .expect("memory store mutex")
            .iter()
            .filter(|(owner, _)| owner == user)
            .map(|(_, task)| task.clone())
            .collect();
        stored.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.created.cmp(&b.created))
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        Ok(stored)
    }

    fn put(&self, user: &str, task: Task) -> Result<(), StoreError> {
        let mut stored = self.tasks.lock().expect("memory store mutex");
        if let Some(uid) = task.external_uid.as_deref()
            && let Some(clash) = stored
                .iter()
                .filter(|(owner, _)| owner == user)
                .map(|(_, other)| other)
                .find(|other| other.id != task.id && other.external_uid.as_deref() == Some(uid))
        {
            return Err(StoreError::Conflict(format!(
                "{} and {} both claim uid {uid}",
                task.id.0, clash.id.0
            )));
        }
        match stored
            .iter_mut()
            .find(|(owner, other)| owner == user && other.id == task.id)
        {
            Some((_, existing)) => *existing = task,
            None => stored.push((user.to_owned(), task)),
        }
        Ok(())
    }

    fn user_plan(&self, user: &str) -> Result<super::auth::Plan, StoreError> {
        if user == super::auth::LOCAL_USER {
            return Ok(super::auth::Plan::Free);
        }
        Ok(self
            .users
            .lock()
            .expect("memory store mutex")
            .iter()
            .find(|held| held.id == user)
            .map(|held| held.plan)
            .unwrap_or(super::auth::Plan::Anon))
    }

    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError> {
        let mut users = self.users.lock().expect("memory store mutex");
        let Some(found) = users.iter_mut().find(|held| held.id == user) else {
            return Err(StoreError::NotFound);
        };
        found.plan = plan;
        Ok(())
    }

    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
        Ok(self
            .users
            .lock()
            .expect("memory store mutex")
            .iter()
            .any(|held| held.id == user && held.plan == super::auth::Plan::Anon))
    }

    fn ensure_anonymous(&self, user: &str) -> Result<(), StoreError> {
        let mut users = self.users.lock().expect("memory store mutex");
        if !users.iter().any(|held| held.id == user) {
            users.push(super::auth::Me {
                id: user.to_owned(),
                email: String::new(),
                name: String::new(),
                picture: None,
                plan: super::auth::Plan::Anon,
            });
        }
        Ok(())
    }

    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
        let mut stored = self.tasks.lock().expect("memory store mutex");
        let before = stored.len();
        stored.retain(|(owner, task)| !(owner == user && &task.id == id));
        Ok(stored.len() != before)
    }

    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        let mut stored = self.tasks.lock().expect("memory store mutex");
        for (position, id) in ids.iter().enumerate() {
            if let Some((_, task)) = stored
                .iter_mut()
                .find(|(owner, task)| owner == user && &task.id == id)
            {
                task.order = position as i64;
            }
        }
        Ok(())
    }

    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError> {
        Ok(self
            .tasks
            .lock()
            .expect("memory store mutex")
            .iter()
            .find(|(owner, task)| owner == user && task.external_uid.as_deref() == Some(uid))
            .map(|(_, task)| task.clone()))
    }

    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut users = self.users.lock().expect("memory store mutex");
        match users.iter_mut().find(|held| held.id == user.id) {
            Some(existing) => *existing = user.clone(),
            None => users.push(user.clone()),
        }
        drop(users);
        self.sessions.lock().expect("memory store mutex").push((
            token.to_owned(),
            user.id.clone(),
            expires_at,
        ));
        Ok(())
    }

    fn delete_session(&self, token: &str) -> Result<(), StoreError> {
        self.sessions
            .lock()
            .expect("memory store mutex")
            .retain(|(held, _, _)| held != token);
        Ok(())
    }

    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<super::auth::Me>, StoreError> {
        let sessions = self.sessions.lock().expect("memory store mutex");
        let Some((_, user_id, _expires_at)) = sessions
            .iter()
            .find(|(held, _, expires_at)| held == token && *expires_at > now)
        else {
            return Ok(None);
        };
        let user_id = user_id.clone();
        drop(sessions);
        Ok(self
            .users
            .lock()
            .expect("memory store mutex")
            .iter()
            .find(|user| user.id == user_id)
            .cloned())
    }

    fn delete_user(&self, user: &str) -> Result<(), StoreError> {
        self.tasks
            .lock()
            .expect("memory store mutex")
            .retain(|(owner, _)| owner != user);
        self.users
            .lock()
            .expect("memory store mutex")
            .retain(|held| held.id != user);
        self.sessions
            .lock()
            .expect("memory store mutex")
            .retain(|(_, owner, _)| owner != user);
        Ok(())
    }

    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
        // Captures are counted from the tasks they created, which is what the memory
        // store has to work with. The SQL store counts them properly.
        Ok(self
            .tasks
            .lock()
            .expect("memory store mutex")
            .iter()
            .filter(|(owner, task)| {
                owner == user
                    && task.source == todo_core::Source::Text
                    && task.created.with_timezone(&chrono_tz::UTC).date_naive() == day
            })
            .count() as u32)
    }

    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
        let mut stored = self.tasks.lock().expect("memory store mutex");
        let existing_uids: std::collections::HashSet<_> = stored
            .iter()
            .filter(|(owner, _)| owner == user)
            .filter_map(|(_, task)| task.external_uid.clone())
            .collect();
        let mut moved = Vec::new();
        stored.retain(|(owner, task)| {
            if owner != anonymous {
                return true;
            }
            if task
                .external_uid
                .as_deref()
                .is_some_and(|uid| existing_uids.contains(uid))
            {
                false
            } else {
                moved.push(task.clone());
                false
            }
        });
        stored.extend(moved.into_iter().map(|task| (user.to_owned(), task)));
        Ok(())
    }
}

/// Tasks in a SQLite file of its own, on tables of its own.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open the database named by `RAMBL_DATA`, or [`DEFAULT_DATA_PATH`].
    pub fn open_from_env() -> Result<Self, StoreError> {
        let configured =
            std::env::var("RAMBL_DATA").unwrap_or_else(|_| DEFAULT_DATA_PATH.to_string());
        Self::open(Path::new(&configured))
    }

    /// Open the database at `path`, creating the folder and applying any
    /// migrations it is behind on.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                StoreError::Backend(format!("could not create {}: {e}", parent.display()))
            })?;
        }
        let store = Self {
            conn: Mutex::new(
                Connection::open(path).map_err(|e| StoreError::Backend(e.to_string()))?,
            ),
        };
        store.migrate()?;
        Ok(store)
    }

    /// A migrated database that lives only as long as this handle, for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let store = Self {
            conn: Mutex::new(
                Connection::open_in_memory().map_err(|e| StoreError::Backend(e.to_string()))?,
            ),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Bring the schema up to [`SCHEMA_VERSION`]. Runs on every open; it is a
    /// no-op once the file is current.
    fn migrate(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().expect("db mutex");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        for statement in MIGRATIONS {
            tx.execute_batch(statement)
                .map_err(|e| StoreError::Backend(e.to_string()))?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        tx.commit().map_err(|e| StoreError::Backend(e.to_string()))
    }

    /// Number of rows held, for the tests that count directly.
    #[cfg(test)]
    fn count(&self) -> i64 {
        self.conn
            .lock()
            .expect("db mutex")
            .query_row("SELECT COUNT(*) FROM v2_tasks", [], |row| row.get(0))
            .expect("count")
    }
}

impl Store for SqliteStore {
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError> {
        let conn = self.conn.lock().expect("db mutex");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM v2_tasks WHERE user_id = ?1 \
                 ORDER BY \"order\" ASC, created ASC, id ASC"
            ))
            .map_err(backend)?;
        let rows = stmt.query_map(params![user], read_task).map_err(backend)?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(backend)?);
        }
        Ok(tasks)
    }

    fn put(&self, user: &str, task: Task) -> Result<(), StoreError> {
        let (when_kind, when_time) = when_to_parts(task.when);
        let conn = self.conn.lock().expect("db mutex");
        conn.execute(
            &format!(
                "INSERT INTO v2_tasks ({TASK_COLUMNS}, user_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
                 ON CONFLICT(id) DO UPDATE SET \
                     title = excluded.title, \
                     duration_min = excluded.duration_min, \
                     when_kind = excluded.when_kind, \
                     when_time = excluded.when_time, \
                     done = excluded.done, \
                     \"order\" = excluded.\"order\", \
                     label = excluded.label, \
                     priority = excluded.priority, \
                     source = excluded.source, \
                     created = excluded.created, \
                     external_uid = excluded.external_uid, \
                     user_id = excluded.user_id"
            ),
            params![
                task.id.0,
                task.title,
                i64::from(task.duration_min),
                when_kind,
                when_time.map(format_ts),
                i64::from(task.done),
                task.order,
                label_to_stored(task.label),
                priority_to_stored(task.priority),
                source_to_stored(task.source),
                format_ts(task.created),
                task.external_uid,
                user,
            ],
        )
        .map_err(conflict)?;
        Ok(())
    }

    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
        let removed = self
            .conn
            .lock()
            .expect("db mutex")
            .execute(
                "DELETE FROM v2_tasks WHERE id = ?1 AND user_id = ?2",
                params![id.0, user],
            )
            .map_err(backend)?;
        Ok(removed > 0)
    }

    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().expect("db mutex");
        let tx = conn.transaction().map_err(backend)?;
        {
            let mut stmt = tx
                .prepare("UPDATE v2_tasks SET \"order\" = ?3 WHERE id = ?1 AND user_id = ?2")
                .map_err(backend)?;
            for (position, id) in ids.iter().enumerate() {
                stmt.execute(params![id.0, user, position as i64])
                    .map_err(backend)?;
            }
        }
        tx.commit().map_err(backend)
    }

    fn find_by_uid(&self, user: &str, uid: &str) -> Result<Option<Task>, StoreError> {
        let conn = self.conn.lock().expect("db mutex");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM v2_tasks WHERE external_uid = ?1 AND user_id = ?2"
            ))
            .map_err(backend)?;
        let mut rows = stmt.query(params![uid, user]).map_err(backend)?;
        let Some(row) = rows.next().map_err(backend)? else {
            return Ok(None);
        };
        Ok(Some(read_task(row).map_err(backend)?))
    }

    fn upsert_user_and_session(
        &self,
        user: &super::auth::Me,
        token: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().expect("db mutex");
        let tx = conn.transaction().map_err(backend)?;
        // First sign-in creates the user, later ones refresh the name and picture, so
        // a rename at Google reaches the app.
        tx.execute(
            "INSERT INTO v2_users (id, email, name, picture, created_at, plan) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(id) DO UPDATE SET \
                 email = excluded.email, name = excluded.name, picture = excluded.picture",
            params![
                user.id,
                user.email,
                user.name,
                user.picture,
                format_ts(Utc::now()),
                user.plan.as_str()
            ],
        )
        .map_err(backend)?;
        tx.execute(
            "INSERT INTO v2_sessions (token, user_id, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![token, user.id, format_ts(expires_at), format_ts(Utc::now())],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)
    }

    fn delete_session(&self, token: &str) -> Result<(), StoreError> {
        self.conn
            .lock()
            .expect("db mutex")
            .execute("DELETE FROM v2_sessions WHERE token = ?1", params![token])
            .map_err(backend)?;
        Ok(())
    }

    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<super::auth::Me>, StoreError> {
        let conn = self.conn.lock().expect("db mutex");
        let mut stmt = conn
            .prepare(
                "SELECT users.id, users.email, users.name, users.picture, users.plan \
                 FROM v2_sessions JOIN v2_users ON v2_users.id = v2_sessions.user_id \
                 WHERE v2_sessions.token = ?1 AND v2_sessions.expires_at > ?2",
            )
            .map_err(backend)?;
        let mut rows = stmt
            .query(params![token, format_ts(now)])
            .map_err(backend)?;
        let Some(row) = rows.next().map_err(backend)? else {
            return Ok(None);
        };
        Ok(Some(super::auth::Me {
            id: row.get(0).map_err(backend)?,
            email: row.get(1).map_err(backend)?,
            name: row.get(2).map_err(backend)?,
            picture: row.get(3).map_err(backend)?,
            plan: parse_plan(&row.get::<_, String>(4).map_err(backend)?),
        }))
    }

    fn delete_user(&self, user: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().expect("db mutex");
        let tx = conn.transaction().map_err(backend)?;
        // Tasks go first: SQLite cannot cascade here because user_id is a column
        // added by migration with a default, not a declared constraint.
        tx.execute("DELETE FROM v2_tasks WHERE user_id = ?1", params![user])
            .map_err(backend)?;
        tx.execute("DELETE FROM v2_sessions WHERE user_id = ?1", params![user])
            .map_err(backend)?;
        tx.execute("DELETE FROM v2_users WHERE id = ?1", params![user])
            .map_err(backend)?;
        tx.commit().map_err(backend)
    }

    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
        let conn = self.conn.lock().expect("db mutex");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM v2_tasks \
                 WHERE user_id = ?1 AND created >= ?2 AND created < ?3",
                params![
                    user,
                    format_ts(day.and_time(chrono::NaiveTime::MIN).and_utc()),
                    format_ts(
                        day.succ_opt()
                            .unwrap()
                            .and_time(chrono::NaiveTime::MIN)
                            .and_utc()
                    ),
                ],
                |row| row.get(0),
            )
            .map_err(backend)?;
        Ok(count as u32)
    }

    fn user_plan(&self, user: &str) -> Result<super::auth::Plan, StoreError> {
        if user == super::auth::LOCAL_USER {
            return Ok(super::auth::Plan::Free);
        }
        let plan: String = self
            .conn
            .lock()
            .expect("db mutex")
            .query_row(
                "SELECT plan FROM v2_users WHERE id = ?1",
                params![user],
                |row| row.get(0),
            )
            .map_err(backend)?;
        Ok(parse_plan(&plan))
    }

    fn set_user_plan(&self, user: &str, plan: super::auth::Plan) -> Result<(), StoreError> {
        let changed = self
            .conn
            .lock()
            .expect("db mutex")
            .execute(
                "UPDATE v2_users SET plan = ?1 WHERE id = ?2",
                params![plan.as_str(), user],
            )
            .map_err(backend)?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().expect("db mutex");
        let tx = conn.transaction().map_err(backend)?;
        tx.execute(
            "DELETE FROM v2_tasks WHERE user_id = ?1 AND external_uid IS NOT NULL
             AND external_uid IN (
                 SELECT external_uid FROM v2_tasks
                 WHERE user_id = ?2 AND external_uid IS NOT NULL
             )",
            params![anonymous, user],
        )
        .map_err(backend)?;
        tx.execute(
            "UPDATE v2_tasks SET user_id = ?1 WHERE user_id = ?2",
            params![user, anonymous],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)
    }

    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
        let found: Option<String> = self
            .conn
            .lock()
            .expect("db mutex")
            .query_row(
                "SELECT id FROM v2_users WHERE id = ?1 AND plan = 'anon'",
                params![user],
                |row| row.get(0),
            )
            .optional()
            .map_err(backend)?;
        Ok(found.is_some())
    }

    fn ensure_anonymous(&self, user: &str) -> Result<(), StoreError> {
        self.conn
            .lock()
            .expect("db mutex")
            .execute(
                "INSERT OR IGNORE INTO v2_users (id,email,name,picture,created_at,plan) \
                 VALUES (?1,'','',NULL,?2,'anon')",
                params![user, format_ts(Utc::now())],
            )
            .map_err(backend)?;
        Ok(())
    }
}

/// Read one `TASK_COLUMNS` row into a [`Task`].
///
/// The column indexes are positional, so they follow `TASK_COLUMNS`. A test
/// checks that list against the real table, which catches a column being added
/// in one place and not the other.
fn read_task(row: &rusqlite::Row<'_>) -> Result<Task, rusqlite::Error> {
    let when_kind: String = row.get(3)?;
    let when_time: Option<String> = row.get(4)?;
    let done: i64 = row.get(5)?;
    let label: String = row.get(7)?;
    let priority: String = row.get(8)?;
    let source: String = row.get(9)?;
    let created: String = row.get(10)?;

    Ok(Task {
        id: TaskId(row.get(0)?),
        title: row.get(1)?,
        duration_min: row.get::<_, i64>(2)? as u16,
        when: when_from_parts(&when_kind, when_time.as_deref()),
        done: done != 0,
        order: row.get(6)?,
        label: label_from_stored(&label),
        priority: priority_from_stored(&priority),
        source: source_from_stored(&source),
        created: parse_ts(&created).unwrap_or_else(Utc::now),
        external_uid: row.get(11)?,
    })
}

fn backend(error: rusqlite::Error) -> StoreError {
    StoreError::Backend(error.to_string())
}

/// A uniqueness violation is a conflict the caller can act on; anything else
/// is the database misbehaving.
fn conflict(error: rusqlite::Error) -> StoreError {
    match error {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::ConstraintViolation,
                ..
            },
            message,
        ) => StoreError::Conflict(message.unwrap_or_else(|| "constraint violated".into())),
        other => StoreError::Backend(other.to_string()),
    }
}

/// SQLite has no date type; instants are stored as RFC 3339 text.
fn format_ts(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339()
}

fn parse_ts(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// `When` as the two columns that hold it.
fn when_to_parts(when: When) -> (&'static str, Option<DateTime<Utc>>) {
    match when {
        When::Anytime => ("anytime", None),
        When::DueBy(deadline) => ("due_by", Some(deadline)),
        When::At(start) => ("at", Some(start)),
    }
}

fn when_from_parts(kind: &str, time: Option<&str>) -> When {
    let instant = time.and_then(parse_ts);
    match (kind, instant) {
        ("due_by", Some(deadline)) => When::DueBy(deadline),
        ("at", Some(start)) => When::At(start),
        // An unknown kind, or a kind whose instant will not parse, is read as
        // "anytime": a hand-edited row must not stop the server starting, and
        // an unscheduled task is a safe thing to show.
        _ => When::Anytime,
    }
}

fn label_to_stored(label: Label) -> &'static str {
    match label {
        Label::School => "school",
        Label::Work => "work",
        Label::Errands => "errands",
        Label::Health => "health",
        Label::Personal => "personal",
        Label::Other => "other",
    }
}

fn label_from_stored(text: &str) -> Label {
    match text {
        "school" => Label::School,
        "work" => Label::Work,
        "errands" => Label::Errands,
        "health" => Label::Health,
        "personal" => Label::Personal,
        _ => Label::Other,
    }
}

fn priority_to_stored(priority: Priority) -> &'static str {
    match priority {
        Priority::Low => "low",
        Priority::Medium => "medium",
        Priority::High => "high",
        Priority::Critical => "critical",
    }
}

fn priority_from_stored(text: &str) -> Priority {
    match text {
        "low" => Priority::Low,
        "high" => Priority::High,
        "critical" => Priority::Critical,
        // "medium", and anything a hand-edit invented, is Medium.
        _ => Priority::Medium,
    }
}

fn source_to_stored(source: Source) -> &'static str {
    match source {
        Source::Voice => "voice",
        Source::Pdf => "pdf",
        Source::Email => "email",
        Source::Text => "text",
        Source::Calendar => "calendar",
    }
}

fn source_from_stored(text: &str) -> Source {
    match text {
        "voice" => Source::Voice,
        "pdf" => Source::Pdf,
        "email" => Source::Email,
        "calendar" => Source::Calendar,
        _ => Source::Text,
    }
}

/// Every spelling each enum can be stored as, for the round-trip test.
const LABELS: [Label; 6] = [
    Label::School,
    Label::Work,
    Label::Errands,
    Label::Health,
    Label::Personal,
    Label::Other,
];
const PRIORITIES: [Priority; 4] = [
    Priority::Low,
    Priority::Medium,
    Priority::High,
    Priority::Critical,
];
const SOURCES: [Source; 5] = [
    Source::Voice,
    Source::Pdf,
    Source::Email,
    Source::Text,
    Source::Calendar,
];

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// The user the contract suite writes as. Scoping is part of the contract now, so
    /// the tests have to name an owner like a real caller does.
    const USER: &str = "alice";
    const OTHER: &str = "bob";

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, hour, 0, 0).unwrap()
    }

    fn task(id: &str, order: i64) -> Task {
        Task {
            id: TaskId(id.into()),
            title: format!("Task {id}"),
            duration_min: 30,
            when: When::Anytime,
            done: false,
            order,
            label: Label::Work,
            priority: Priority::Medium,
            source: Source::Text,
            created: at(9),
            external_uid: None,
        }
    }

    /// One stored task, by id. Looked up by name so the checks read as claims
    /// about behaviour rather than about where a row happened to sort.
    fn read(store: &dyn Store, id: &TaskId) -> Task {
        store
            .tasks(USER)
            .expect("read tasks")
            .into_iter()
            .find(|task| &task.id == id)
            .unwrap_or_else(|| panic!("no stored task {id:?}"))
    }

    /// Two people must never see each other's tasks. This is the promise the whole
    /// user_id column exists for, and it is the one a missing filter would break.
    fn users_are_isolated(store: &dyn Store) {
        store.put(USER, task("mine", 0)).unwrap();
        store.put(OTHER, task("theirs", 0)).unwrap();

        let mine = store.tasks(USER).unwrap();
        assert_eq!(
            mine.len(),
            1,
            "alice should see only her own task, saw {:?}",
            mine.iter().map(|t| &t.title).collect::<Vec<_>>()
        );
        assert_eq!(mine[0].id, TaskId("mine".into()));

        let theirs = store.tasks(OTHER).unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].id, TaskId("theirs".into()));

        // Another person's task must not be reachable by its id, either.
        assert!(
            store.find_by_uid(USER, "theirs-uid").unwrap().is_none(),
            "alice reached bob's task by uid"
        );

        // And deleting only removes the caller's own.
        store.delete(OTHER, &TaskId("mine".into())).unwrap();
        assert_eq!(store.tasks(USER).unwrap().len(), 1, "alice lost her task");
    }
    /// The suite both stores must pass. Everything here goes through the
    /// [`Store`] trait, so a store that passes it behaves like the other one.
    fn store_contract(store: &dyn Store) {
        // -- an empty store has nothing in it
        assert_eq!(store.tasks(USER).unwrap().len(), 0, "a new store is empty");

        // -- put then read back
        store.put(USER, task("t1", 0)).unwrap();
        store.put(USER, task("t2", 1)).unwrap();
        let stored = store.tasks(USER).unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].id, TaskId("t1".into()));
        assert_eq!(stored[1].id, TaskId("t2".into()));

        // -- every field survives the round trip
        let mut original = task("t3", 2);
        original.title = "Read; notes, again".into();
        original.duration_min = 95;
        original.when = When::DueBy(at(17));
        original.done = true;
        original.order = 5;
        original.label = Label::School;
        original.priority = Priority::Critical;
        original.source = Source::Calendar;
        original.created = at(8);
        original.external_uid = Some("abc-123@rambl".into());
        store.put(USER, original.clone()).unwrap();
        assert_eq!(read(store, &original.id), original);

        // -- each When variant survives
        for (index, when) in [When::Anytime, When::DueBy(at(23)), When::At(at(14))]
            .into_iter()
            .enumerate()
        {
            let mut pinned = task(&format!("w{index}"), 100 + index as i64);
            pinned.when = when;
            store.put(USER, pinned.clone()).unwrap();
            let read = store
                .tasks(USER)
                .unwrap()
                .into_iter()
                .find(|t| t.id == pinned.id)
                .expect("task is stored");
            assert_eq!(read.when, when, "{when:?} did not survive");
        }

        // -- every enum spelling survives
        for (index, label) in LABELS.into_iter().enumerate() {
            let mut labelled = task(&format!("l{index}"), 200 + index as i64);
            labelled.label = label;
            store.put(USER, labelled.clone()).unwrap();
            assert_eq!(read(store, &labelled.id).label, label);
        }
        for (index, priority) in PRIORITIES.into_iter().enumerate() {
            let mut ranked = task(&format!("p{index}"), 300 + index as i64);
            ranked.priority = priority;
            store.put(USER, ranked.clone()).unwrap();
            assert_eq!(read(store, &ranked.id).priority, priority);
        }
        for (index, source) in SOURCES.into_iter().enumerate() {
            let mut sourced = task(&format!("s{index}"), 400 + index as i64);
            sourced.source = source;
            store.put(USER, sourced.clone()).unwrap();
            assert_eq!(read(store, &sourced.id).source, source);
        }

        // -- put on a known id replaces, and does not duplicate
        let before = store.tasks(USER).unwrap().len();
        let mut edited = original.clone();
        edited.title = "Renamed".into();
        edited.done = false;
        store.put(USER, edited.clone()).unwrap();
        let all = store.tasks(USER).unwrap();
        assert_eq!(all.len(), before, "put must replace, never duplicate");
        assert_eq!(read(store, &edited.id).title, "Renamed");
        assert!(!read(store, &edited.id).done);

        // -- tasks come back sorted by order
        let orders: Vec<i64> = store.tasks(USER).unwrap().iter().map(|t| t.order).collect();
        let mut ascending = orders.clone();
        ascending.sort_unstable();
        assert_eq!(orders, ascending, "tasks must come back in order");

        // -- delete reports whether there was something to delete
        let before_delete = store.tasks(USER).unwrap().len();
        assert!(store.delete(USER, &TaskId("t1".into())).unwrap());
        assert!(
            !store.delete(USER, &TaskId("t1".into())).unwrap(),
            "gone already"
        );
        assert!(!store.delete(USER, &TaskId("never-existed".into())).unwrap());
        assert_eq!(store.tasks(USER).unwrap().len(), before_delete - 1);
        assert!(
            !store.tasks(USER).unwrap().iter().any(|t| t.id.0 == "t1"),
            "the deleted task must not come back"
        );

        // -- set_order rewrites the order
        let before: Vec<String> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        let mut reversed = before.clone();
        reversed.reverse();
        let reversed_ids: Vec<TaskId> = reversed.iter().cloned().map(TaskId).collect();
        store.set_order(USER, &reversed_ids).unwrap();
        let after: Vec<String> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        assert_eq!(after, reversed, "set_order is the new order");

        // -- set_order makes the positions 0, 1, ... in the order given
        let positions: Vec<i64> = store.tasks(USER).unwrap().iter().map(|t| t.order).collect();
        assert_eq!(
            positions,
            (0..positions.len() as i64).collect::<Vec<_>>(),
            "positions must be gapless and start at zero"
        );

        // -- an id that is not stored is skipped, not invented
        let count = store.tasks(USER).unwrap().len();
        store.set_order(USER, &[TaskId("nope".into())]).unwrap();
        let after_unknown = store.tasks(USER).unwrap();
        assert_eq!(
            after_unknown.len(),
            count,
            "set_order must not create tasks"
        );
        assert!(
            !after_unknown.iter().any(|t| t.id.0 == "nope"),
            "an unknown id must not become a task"
        );

        // -- a full order is the normal case and must be exact
        let wanted: Vec<String> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        let wanted_ids: Vec<TaskId> = wanted.iter().cloned().map(TaskId).collect();
        store.set_order(USER, &wanted_ids).unwrap();
        let restored: Vec<String> = store
            .tasks(USER)
            .unwrap()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        assert_eq!(restored, wanted, "set_order is the new order");

        // -- find_by_uid
        assert!(
            store
                .find_by_uid(USER, "abc-123@rambl")
                .unwrap()
                .is_some_and(|t| t.id.0 == "t3")
        );
        assert!(store.find_by_uid(USER, "never-imported").unwrap().is_none());
        assert!(store.find_by_uid(USER, "").unwrap().is_none());

        // -- a task with no uid is not found by any uid
        assert!(
            store
                .tasks(USER)
                .unwrap()
                .iter()
                .filter(|t| t.external_uid.is_none())
                .count()
                > 0
        );
        assert!(store.find_by_uid(USER, "no-such-uid").unwrap().is_none());
    }

    #[test]
    fn memory_store_passes_the_contract() {
        users_are_isolated(&MemoryStore::new());
        store_contract(&MemoryStore::new());
    }

    #[test]
    fn sqlite_store_passes_the_contract() {
        users_are_isolated(&SqliteStore::open_in_memory().unwrap());
        store_contract(&SqliteStore::open_in_memory().unwrap());
    }

    // -- SQLite specifics the contract cannot cover through the trait.

    #[test]
    fn a_file_store_keeps_its_tasks_across_a_reopen() {
        let path = unique_temp_dir().join("nested").join("rambl.db");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());

        {
            let store = SqliteStore::open(&path).unwrap();
            store.put(USER, task("t1", 0)).unwrap();
            store.put(USER, task("t2", 1)).unwrap();
        }

        // Fresh handle, same file: this is the restart case.
        let store = SqliteStore::open(&path).unwrap();
        let stored = store.tasks(USER).unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].id, TaskId("t1".into()));
        assert_eq!(stored[1].id, TaskId("t2".into()));

        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn reopening_a_current_database_applies_nothing_and_loses_nothing() {
        let path = unique_temp_dir().join("rambl.db");
        let _ = std::fs::remove_dir_all(&path);

        let store = SqliteStore::open(&path).unwrap();
        store.put(USER, task("t1", 0)).unwrap();
        drop(store);

        // Second open runs migrate() again; it must be a no-op, not a rebuild.
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(store.tasks(USER).unwrap().len(), 1);
        assert_eq!(store.count(), 1);

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn the_schema_is_at_the_version_the_code_expects() {
        let store = SqliteStore::open_in_memory().unwrap();
        let conn = store.conn.lock().expect("db mutex");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn the_column_list_matches_the_table() {
        let store = SqliteStore::open_in_memory().unwrap();
        let conn = store.conn.lock().expect("db mutex");
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('v2_tasks')")
            .unwrap();
        let actual: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);

        let listed: Vec<&str> = TASK_COLUMNS
            .split(',')
            .map(|name| name.trim().trim_matches('"'))
            .collect();
        // `user_id` is the one column on the table that is not a task field: it
        // scopes every read and write rather than describing a task, so it is bound
        // separately rather than read into a `Task`.
        let mut expected = actual.clone();
        expected.retain(|name| name != "user_id");
        assert_eq!(
            listed, expected,
            "TASK_COLUMNS must list exactly the task columns, in order"
        );
        assert!(
            actual.contains(&"user_id".to_owned()),
            "tasks must be scoped to a user"
        );
    }

    #[test]
    fn two_tasks_cannot_claim_one_uid() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut first = task("t1", 0);
        first.external_uid = Some("shared".into());
        store.put(USER, first).unwrap();

        let mut second = task("t2", 1);
        second.external_uid = Some("shared".into());
        let error = store.put(USER, second).unwrap_err();
        assert!(
            matches!(error, StoreError::Conflict(_)),
            "expected a conflict, got {error:?}"
        );
        assert_eq!(store.count(), 1, "the clashing task was not stored");
    }

    #[test]
    fn memory_store_reports_the_same_uid_conflict() {
        let store = MemoryStore::new();
        let mut first = task("t1", 0);
        first.external_uid = Some("shared".into());
        store.put(USER, first).unwrap();

        let mut second = task("t2", 1);
        second.external_uid = Some("shared".into());
        assert!(matches!(
            store.put(USER, second),
            Err(StoreError::Conflict(_))
        ));
        assert_eq!(store.tasks(USER).unwrap().len(), 1);
    }

    #[test]
    fn re_importing_a_uid_updates_that_task_in_place() {
        let store = MemoryStore::new();
        let mut imported = task("t1", 0);
        imported.external_uid = Some("evt-1".into());
        imported.when = When::At(at(14));
        store.put(USER, imported).unwrap();

        // The event moved to Friday: same UID, new time.
        let existing = store.find_by_uid(USER, "evt-1").unwrap().expect("found");
        let mut moved = existing.clone();
        moved.when = When::At(at(17));
        store.put(USER, moved).unwrap();

        assert_eq!(
            store.tasks(USER).unwrap().len(),
            1,
            "a re-import must update, not duplicate"
        );
        assert_eq!(
            store.find_by_uid(USER, "evt-1").unwrap().unwrap().when,
            When::At(at(17))
        );
    }

    #[test]
    fn v2_never_touches_the_v1_tasks_table() {
        let store = SqliteStore::open_in_memory().unwrap();
        // A v1-shaped table in the same file must survive, untouched.
        store
            .conn
            .lock()
            .expect("db mutex")
            .execute_batch(
                "CREATE TABLE tasks (id TEXT PRIMARY KEY, title TEXT NOT NULL);
                 INSERT INTO tasks (id, title) VALUES ('v1-1', 'v1 task');",
            )
            .unwrap();

        store.put(USER, task("t1", 0)).unwrap();

        let conn = store.conn.lock().expect("db mutex");
        let v1: String = conn
            .query_row("SELECT title FROM tasks WHERE id = 'v1-1'", [], |row| {
                row.get(0)
            })
            .expect("the v1 row is still there");
        assert_eq!(v1, "v1 task");
    }

    #[test]
    fn a_hand_edited_row_degrades_instead_of_failing_the_load() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .conn
            .lock()
            .expect("db mutex")
            .execute_batch(
                "INSERT INTO v2_tasks (id, title, duration_min, when_kind, when_time, \
                                        done, \"order\", label, priority, source, created, external_uid, user_id) \
                 VALUES ('odd', 'Odd', 30, 'whenever', 'not-a-date', 0, 0, \
                         'mauve', 'enormous', 'telepathy', 'yesterday', NULL, 'alice');",
            )
            .unwrap();

        let stored = store.tasks(USER).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].when, When::Anytime);
        assert_eq!(stored[0].label, Label::Other);
        assert_eq!(stored[0].priority, Priority::Medium);
        assert_eq!(stored[0].source, Source::Text);
    }

    #[test]
    fn store_errors_read_as_something_a_user_can_see() {
        for error in [
            StoreError::NotFound,
            StoreError::Conflict("two tasks claim one uid".into()),
            StoreError::Backend("disk is full".into()),
        ] {
            let message = error.to_string();
            assert!(!message.is_empty(), "{error:?} rendered nothing");
            assert!(!message.contains('{'), "{error:?} leaked a format brace");
        }
    }

    #[test]
    fn when_round_trips_through_its_two_columns() {
        for when in [When::Anytime, When::DueBy(at(17)), When::At(at(14))] {
            let (kind, time) = when_to_parts(when);
            assert_eq!(when_from_parts(kind, time.map(format_ts).as_deref()), when);
        }
        assert_eq!(when_to_parts(When::Anytime), ("anytime", None));
    }

    /// A folder of its own, so two tests writing at once cannot collide.
    fn unique_temp_dir() -> std::path::PathBuf {
        let stem = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("rambl-store-{}-{stem}", std::process::id()))
    }
}
