//! Where v2 tasks live. `SqliteStore` is for the server; `MemoryStore` is for tests.
//! Tasks are stored as JSON documents keyed by id, so the schema does not have to
//! change when a field is added to `Task`.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use todo_core::{Task, TaskId};

pub trait Store: Send {
    fn tasks(&self) -> Result<Vec<Task>>;
    fn get(&self, id: &TaskId) -> Result<Option<Task>>;
    /// Insert or replace the task with the same id.
    fn put(&mut self, task: &Task) -> Result<()>;
    /// Returns `true` when a task was removed.
    fn delete(&mut self, id: &TaskId) -> Result<bool>;
}

impl Store for Box<dyn Store> {
    fn tasks(&self) -> Result<Vec<Task>> {
        (**self).tasks()
    }
    fn get(&self, id: &TaskId) -> Result<Option<Task>> {
        (**self).get(id)
    }
    fn put(&mut self, task: &Task) -> Result<()> {
        (**self).put(task)
    }
    fn delete(&mut self, id: &TaskId) -> Result<bool> {
        (**self).delete(id)
    }
}

#[derive(Default)]
pub struct MemoryStore {
    tasks: BTreeMap<String, Task>,
}

impl Store for MemoryStore {
    fn tasks(&self) -> Result<Vec<Task>> {
        Ok(self.tasks.values().cloned().collect())
    }
    fn get(&self, id: &TaskId) -> Result<Option<Task>> {
        Ok(self.tasks.get(&id.0).cloned())
    }
    fn put(&mut self, task: &Task) -> Result<()> {
        self.tasks.insert(task.id.0.clone(), task.clone());
        Ok(())
    }
    fn delete(&mut self, id: &TaskId) -> Result<bool> {
        Ok(self.tasks.remove(&id.0).is_some())
    }
}

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Opens (and creates, if needed) the v2 table in the database at `path`.
    /// `:memory:` gives a private in-memory database.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open {path}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS v2_tasks (
                id TEXT PRIMARY KEY NOT NULL,
                body TEXT NOT NULL
            );",
        )?;
        Ok(Self { conn })
    }
}

impl Store for SqliteStore {
    fn tasks(&self) -> Result<Vec<Task>> {
        let mut statement = self.conn.prepare("SELECT body FROM v2_tasks")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            let body = row?;
            serde_json::from_str(&body).context("stored task is not valid JSON")
        })
        .collect()
    }
    fn get(&self, id: &TaskId) -> Result<Option<Task>> {
        let body: Option<String> = self
            .conn
            .query_row(
                "SELECT body FROM v2_tasks WHERE id = ?1",
                params![id.0],
                |row| row.get(0),
            )
            .optional()?;
        body.map(|body| serde_json::from_str(&body).context("stored task is not valid JSON"))
            .transpose()
    }
    fn put(&mut self, task: &Task) -> Result<()> {
        let body = serde_json::to_string(task)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO v2_tasks (id, body) VALUES (?1, ?2)",
            params![task.id.0, body],
        )?;
        Ok(())
    }
    fn delete(&mut self, id: &TaskId) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM v2_tasks WHERE id = ?1", params![id.0])?;
        Ok(removed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use todo_core::{Label, Priority, Source, When};

    fn task(id: &str) -> Task {
        Task {
            id: TaskId(id.into()),
            title: format!("Task {id}"),
            duration_min: 30,
            when: When::Anytime,
            done: false,
            order: 0,
            label: Label::Other,
            priority: Priority::Low,
            source: Source::Text,
            created: Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap(),
            external_uid: None,
        }
    }

    fn exercise(store: &mut dyn Store) {
        store.put(&task("a")).unwrap();
        store.put(&task("b")).unwrap();
        assert_eq!(store.tasks().unwrap().len(), 2);
        assert_eq!(store.get(&TaskId("a".into())).unwrap(), Some(task("a")));

        let mut changed = task("a");
        changed.done = true;
        store.put(&changed).unwrap();
        assert_eq!(store.get(&TaskId("a".into())).unwrap(), Some(changed));

        assert!(store.delete(&TaskId("a".into())).unwrap());
        assert!(!store.delete(&TaskId("a".into())).unwrap());
        assert_eq!(store.tasks().unwrap(), vec![task("b")]);
    }

    #[test]
    fn memory_store_round_trips() {
        exercise(&mut MemoryStore::default());
    }

    #[test]
    fn sqlite_store_round_trips() {
        exercise(&mut SqliteStore::open(":memory:").unwrap());
    }
}
