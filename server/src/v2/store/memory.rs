//! The in-memory `Store` the HTTP and app tests run on (test builds only).

use super::*;

/// Tasks held in memory, with their owner. The HTTP tests use this so they never
/// touch a disk, and it has to model ownership exactly as the SQL store does or the
/// tests would not be testing the scoping that matters.
#[derive(Default)]
pub struct MemoryStore {
    tasks: Mutex<Vec<(String, Task)>>,
    series: Mutex<Vec<(String, Series)>>,
    exceptions: Mutex<Vec<(String, SeriesId, SeriesException)>>,
    users: Mutex<Vec<crate::v2::auth::Me>>,
    sessions: Mutex<Vec<(String, String, chrono::DateTime<Utc>)>>,
    captures: Mutex<Vec<(String, String, NaiveDate)>>,
    /// Every call each account has ever made, which the per-day captures cannot answer
    /// for once they are pruned away.
    usage_totals: Mutex<std::collections::HashMap<String, u32>>,
    anon_issued: Mutex<Vec<(String, NaiveDate)>>,
    blocks: Mutex<Vec<Block>>,
    /// When each anonymous user was last seen.
    activity: Mutex<std::collections::HashMap<String, DateTime<Utc>>>,
    day_settings: Mutex<std::collections::HashMap<String, DaySettings>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Forwards to whichever store was chosen. This is how `App` stays generic
/// over "SQLite or memory" without naming either.
impl Store for MemoryStore {
    fn tasks(&self, user: &str) -> Result<Vec<Task>, StoreError> {
        let mut stored: Vec<Task> = self
            .tasks
            .lock_recover()
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
        let mut stored = self.tasks.lock_recover();
        if stored
            .iter()
            .any(|(owner, other)| owner != user && other.id == task.id)
        {
            return Err(StoreError::Conflict(format!(
                "task id {} belongs to another user",
                task.id.0
            )));
        }
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

    fn user_plan(&self, user: &str) -> Result<crate::v2::auth::Plan, StoreError> {
        if user == crate::v2::auth::LOCAL_USER {
            return Ok(crate::v2::auth::Plan::Free);
        }
        Ok(self
            .users
            .lock_recover()
            .iter()
            .find(|held| held.id == user)
            .map(|held| held.plan)
            .unwrap_or(crate::v2::auth::Plan::Anon))
    }

    fn day_settings(&self, user: &str) -> Result<DaySettings, StoreError> {
        Ok(self
            .day_settings
            .lock_recover()
            .get(user)
            .copied()
            .unwrap_or_default())
    }

    fn set_day_settings(&self, user: &str, settings: DaySettings) -> Result<(), StoreError> {
        // As in SQLite, where the local user's row comes from a migration.
        if user != crate::v2::auth::LOCAL_USER && !self.account_exists(user)? {
            return Err(StoreError::NotFound);
        }
        self.day_settings
            .lock_recover()
            .insert(user.to_owned(), settings);
        Ok(())
    }

    fn set_user_plan(&self, user: &str, plan: crate::v2::auth::Plan) -> Result<(), StoreError> {
        let mut users = self.users.lock_recover();
        let Some(found) = users.iter_mut().find(|held| held.id == user) else {
            return Err(StoreError::NotFound);
        };
        found.plan = plan;
        Ok(())
    }

    fn anonymous_exists(&self, user: &str) -> Result<bool, StoreError> {
        Ok(self
            .users
            .lock_recover()
            .iter()
            .any(|held| held.id == user && held.plan == crate::v2::auth::Plan::Anon))
    }

    fn ensure_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        let mut users = self.users.lock_recover();
        if !users.iter().any(|held| held.id == user) {
            self.activity.lock_recover().insert(user.to_owned(), now);
            users.push(crate::v2::auth::Me {
                id: user.to_owned(),
                email: String::new(),
                name: String::new(),
                picture: None,
                plan: crate::v2::auth::Plan::Anon,
            });
        }
        Ok(())
    }

    fn touch_anonymous(&self, user: &str, now: DateTime<Utc>) -> Result<(), StoreError> {
        if let Some(seen) = self.activity.lock_recover().get_mut(user)
            && *seen < now - ACTIVITY_THROTTLE
        {
            *seen = now;
        }
        Ok(())
    }

    fn expire_anonymous(&self, cutoff: DateTime<Utc>) -> Result<usize, StoreError> {
        let mut activity = self.activity.lock_recover();
        let gone: Vec<String> = activity
            .iter()
            .filter(|(_, seen)| **seen <= cutoff)
            .map(|(user, _)| user.clone())
            .collect();
        for user in &gone {
            activity.remove(user);
        }
        self.tasks
            .lock_recover()
            .retain(|(owner, _)| !gone.contains(owner));
        // The series go with them: nothing cascades here either, so leaving them would
        // keep a dead visitor's routines with nobody left who could see them.
        self.series
            .lock_recover()
            .retain(|(owner, _)| !gone.contains(owner));
        self.exceptions
            .lock_recover()
            .retain(|(owner, _, _)| !gone.contains(owner));
        self.users
            .lock_recover()
            .retain(|held| !gone.contains(&held.id));
        self.day_settings
            .lock_recover()
            .retain(|owner, _| !gone.contains(owner));
        Ok(gone.len())
    }

    fn delete(&self, user: &str, id: &TaskId) -> Result<bool, StoreError> {
        let mut stored = self.tasks.lock_recover();
        let before = stored.len();
        stored.retain(|(owner, task)| !(owner == user && &task.id == id));
        Ok(stored.len() != before)
    }

    fn set_order(&self, user: &str, ids: &[TaskId]) -> Result<(), StoreError> {
        let mut stored = self.tasks.lock_recover();
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
            .lock_recover()
            .iter()
            .find(|(owner, task)| owner == user && task.external_uid.as_deref() == Some(uid))
            .map(|(_, task)| task.clone()))
    }

    fn series(&self, user: &str) -> Result<Vec<Series>, StoreError> {
        let mut held: Vec<Series> = self
            .series
            .lock_recover()
            .iter()
            .filter(|(owner, _)| owner == user)
            .map(|(_, series)| series.clone())
            .collect();
        held.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.created.cmp(&b.created))
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        Ok(held)
    }

    fn put_series(&self, user: &str, series: Series) -> Result<(), StoreError> {
        let mut held = self.series.lock_recover();
        if held
            .iter()
            .any(|(owner, other)| owner != user && other.id == series.id)
        {
            return Err(StoreError::Conflict(format!(
                "series id {} belongs to another user",
                series.id.0
            )));
        }
        match held
            .iter_mut()
            .find(|(owner, other)| owner == user && other.id == series.id)
        {
            Some((_, existing)) => *existing = series,
            None => held.push((user.to_owned(), series)),
        }
        Ok(())
    }

    fn delete_series(&self, user: &str, id: &SeriesId) -> Result<bool, StoreError> {
        let mut held = self.series.lock_recover();
        let before = held.len();
        held.retain(|(owner, series)| !(owner == user && &series.id == id));
        let removed = held.len() != before;
        if removed {
            self.exceptions
                .lock_recover()
                .retain(|(owner, series, _)| !(owner == user && series == id));
        }
        Ok(removed)
    }

    fn exceptions(&self, user: &str, id: &SeriesId) -> Result<Vec<SeriesException>, StoreError> {
        let mut held: Vec<SeriesException> = self
            .exceptions
            .lock_recover()
            .iter()
            .filter(|(owner, series, _)| owner == user && series == id)
            .map(|(_, _, exception)| exception.clone())
            .collect();
        held.sort_by_key(|exception| exception.date);
        Ok(held)
    }

    fn put_exception(
        &self,
        user: &str,
        id: &SeriesId,
        exception: SeriesException,
    ) -> Result<(), StoreError> {
        let mut held = self.exceptions.lock_recover();
        match held.iter_mut().find(|(owner, series, held)| {
            owner == user && series == id && held.date == exception.date
        }) {
            Some((_, _, existing)) => *existing = exception,
            None => held.push((user.to_owned(), id.clone(), exception)),
        }
        Ok(())
    }

    fn delete_exception(
        &self,
        user: &str,
        id: &SeriesId,
        date: NaiveDate,
    ) -> Result<bool, StoreError> {
        let mut held = self.exceptions.lock_recover();
        let before = held.len();
        held.retain(|(owner, series, exception)| {
            !(owner == user && series == id && exception.date == date)
        });
        Ok(held.len() != before)
    }

    fn upsert_user_and_session(
        &self,
        user: &crate::v2::auth::Me,
        token: &str,
        _now: chrono::DateTime<Utc>,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut users = self.users.lock_recover();
        match users.iter_mut().find(|held| held.id == user.id) {
            // A returning sign-in keeps the plan the database already holds.
            Some(existing) => {
                existing.email.clone_from(&user.email);
                existing.name.clone_from(&user.name);
                existing.picture.clone_from(&user.picture);
            }
            None => users.push(user.clone()),
        }
        drop(users);
        self.sessions
            .lock_recover()
            .push((token.to_owned(), user.id.clone(), expires_at));
        Ok(())
    }

    fn delete_session(&self, token: &str) -> Result<(), StoreError> {
        self.sessions
            .lock_recover()
            .retain(|(held, _, _)| held != token);
        Ok(())
    }

    fn session_user(
        &self,
        token: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<crate::v2::auth::Me>, StoreError> {
        let sessions = self.sessions.lock_recover();
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
            .lock_recover()
            .iter()
            .find(|user| user.id == user_id)
            .cloned())
    }

    fn delete_user(&self, user: &str) -> Result<(), StoreError> {
        self.tasks.lock_recover().retain(|(owner, _)| owner != user);
        self.series
            .lock_recover()
            .retain(|(owner, _)| owner != user);
        self.exceptions
            .lock_recover()
            .retain(|(owner, _, _)| owner != user);
        self.users.lock_recover().retain(|held| held.id != user);
        self.activity.lock_recover().remove(user);
        self.day_settings.lock_recover().remove(user);
        self.sessions
            .lock_recover()
            .retain(|(_, owner, _)| owner != user);
        Ok(())
    }

    fn captures_today(&self, user: &str, day: chrono::NaiveDate) -> Result<u32, StoreError> {
        Ok(self
            .captures
            .lock_recover()
            .iter()
            .filter(|(owner, _, when)| owner == user && *when == day)
            .count() as u32)
    }

    fn record_capture(&self, user: &str, ip: &str, day: NaiveDate) -> Result<(), StoreError> {
        self.captures
            .lock_recover()
            .push((user.to_owned(), ip.to_owned(), day));
        // The all-time total, which the per-day counts cannot answer for once they
        // are pruned away.
        *self
            .usage_totals
            .lock_recover()
            .entry(user.to_owned())
            .or_default() += 1;
        Ok(())
    }

    fn ip_captures_today(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        Ok(self
            .captures
            .lock_recover()
            .iter()
            .filter(|(_, from, when)| from == ip && *when == day)
            .count() as u32)
    }

    fn record_anon_issued(&self, ip: &str, day: NaiveDate) -> Result<u32, StoreError> {
        let mut issued = self.anon_issued.lock_recover();
        issued.push((ip.to_owned(), day));
        Ok(issued
            .iter()
            .filter(|(from, when)| from == ip && *when == day)
            .count() as u32)
    }

    fn block(&self, target: &Block) -> Result<(), StoreError> {
        let mut blocks = self.blocks.lock_recover();
        if !blocks.contains(target) {
            blocks.push(target.clone());
        }
        Ok(())
    }

    fn unblock(&self, target: &Block) -> Result<bool, StoreError> {
        let mut blocks = self.blocks.lock_recover();
        let before = blocks.len();
        blocks.retain(|held| held != target);
        Ok(blocks.len() != before)
    }

    fn is_blocked(&self, user: &str, ip: &str) -> Result<bool, StoreError> {
        Ok(self
            .blocks
            .lock_recover()
            .iter()
            .any(|held| *held == Block::User(user.into()) || *held == Block::Ip(ip.into())))
    }

    fn blocks(&self) -> Result<Vec<Block>, StoreError> {
        Ok(self.blocks.lock_recover().clone())
    }

    fn usage_today(&self, day: NaiveDate, top: usize) -> Result<Usage, StoreError> {
        let captures = self.captures.lock_recover();
        let today = captures.iter().filter(|(_, _, when)| *when == day);
        Ok(Usage {
            users: top_counts(today.clone().map(|(user, _, _)| user.as_str()), top),
            ips: top_counts(today.map(|(_, ip, _)| ip.as_str()), top),
        })
    }

    fn usage_ranking(&self, order: UsageOrder, top: usize) -> Result<Vec<UsageRow>, StoreError> {
        let mut calls: Vec<(String, u32)> = match order {
            UsageOrder::AllTime => self
                .usage_totals
                .lock_recover()
                .iter()
                .map(|(user, calls)| (user.clone(), *calls))
                .collect(),
            UsageOrder::Day(day) => {
                let captures = self.captures.lock_recover();
                let mut counted: std::collections::HashMap<&str, u32> =
                    std::collections::HashMap::new();
                for (user, _, _when) in captures.iter().filter(|(_, _, _when)| *_when == day) {
                    *counted.entry(user.as_str()).or_default() += 1;
                }
                counted
                    .into_iter()
                    .map(|(user, calls)| (user.to_owned(), calls))
                    .collect()
            }
        };
        // Most spend first, then by id, so equal spend has one order on every store.
        calls.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        calls.truncate(top);
        let users = self.users.lock_recover();
        calls
            .into_iter()
            .map(|(user, calls)| {
                let account = users.iter().find(|held| held.id == user);
                Ok(UsageRow {
                    user,
                    // An account with no email is an anonymous visitor, and says so.
                    email: account.map(|held| held.email.clone()).unwrap_or_default(),
                    plan: account.map_or(crate::v2::auth::Plan::Anon, |held| held.plan),
                    calls,
                })
            })
            .collect()
    }

    fn is_user_blocked(&self, user: &str) -> Result<bool, StoreError> {
        Ok(self
            .blocks
            .lock_recover()
            .contains(&Block::User(user.to_owned())))
    }

    fn account_exists(&self, user: &str) -> Result<bool, StoreError> {
        Ok(self.users.lock_recover().iter().any(|held| held.id == user))
    }

    fn user_id_for_email(&self, email: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .users
            .lock_recover()
            .iter()
            .find(|held| held.plan != crate::v2::auth::Plan::Anon && held.email == email)
            .map(|held| held.id.clone()))
    }

    fn adopt_anonymous(&self, anonymous: &str, user: &str) -> Result<(), StoreError> {
        if !self.anonymous_exists(anonymous)? {
            return Ok(());
        }
        self.users
            .lock_recover()
            .retain(|held| held.id != anonymous);
        self.activity.lock_recover().remove(anonymous);
        self.day_settings.lock_recover().remove(anonymous);
        let mut stored = self.tasks.lock_recover();
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
        drop(stored);
        let mut series = self.series.lock_recover();
        let existing_series_uids: std::collections::HashSet<_> = series
            .iter()
            .filter(|(owner, _)| owner == user)
            .filter_map(|(_, held)| held.external_uid.clone())
            .collect();
        // Like tasks: a series the user already imported keeps the signed-in copy.
        let (moved_series, dropped_series): (Vec<Series>, Vec<Series>) = series
            .iter()
            .filter(|(owner, _)| owner == anonymous)
            .map(|(_, held)| held.clone())
            .partition(|held| {
                !held
                    .external_uid
                    .as_ref()
                    .is_some_and(|uid| existing_series_uids.contains(uid))
            });
        series.retain(|(owner, _)| owner != anonymous);
        drop(series);
        let mut exceptions = self.exceptions.lock_recover();
        let moved_exceptions: Vec<(SeriesId, SeriesException)> = exceptions
            .iter()
            .filter(|(owner, id, _)| {
                owner == anonymous && !dropped_series.iter().any(|held| held.id == *id)
            })
            .map(|(_, series, exception)| (series.clone(), exception.clone()))
            .collect();
        exceptions.retain(|(owner, _, _)| owner != anonymous);
        drop(exceptions);
        for held in moved_series {
            self.put_series(user, held)?;
        }
        for (id, exception) in moved_exceptions {
            self.put_exception(user, &id, exception)?;
        }
        Ok(())
    }
}
