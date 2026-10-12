//! The administration web surface: today's usage, blocks and plan changes through
//! the HTTP API, and the small `/admin` page that drives it.
//!
//! This module holds only presentation. What a block, a plan or a usage row means
//! lives in `super::admin` (name resolution) and `super::store` (persistence),
//! which the console and this page share — the page never repeats their logic.

use chrono::NaiveDate;
use serde::Serialize;

use super::admin::{self, Target};
use super::auth::Plan;
use super::store::{Block, Store, StoreError};

/// Today's captures, by user and by client address, with the scans per plan.
/// One capture is one Jev call; past a plan's daily cap it is judged by `FakeJudge`.
#[derive(Serialize)]
pub struct UsageOut {
    pub day: String,
    pub users: Vec<UserUsage>,
    pub ips: Vec<IpUsage>,
    pub per_plan: PerPlan,
}

#[derive(Serialize)]
pub struct UserUsage {
    pub id: String,
    pub plan: &'static str,
    pub captures: u32,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct IpUsage {
    pub ip: String,
    pub captures: u32,
}

/// Captures today, split by the plan the user was on when asked. Anonymous users
/// count as `anon`; the account signs in before the page, so its plan is stored.
#[derive(Serialize, Default)]
pub struct PerPlan {
    pub anon: u32,
    pub free: u32,
    pub paid: u32,
}

#[derive(Serialize)]
pub struct BlockOut {
    pub kind: &'static str,
    pub value: String,
}

/// `GET /api/v1/admin/usage`, for one day.
pub fn usage(store: &dyn Store, day: NaiveDate) -> Result<UsageOut, StoreError> {
    let today = store.usage_today(day, admin::TOP)?;
    let mut per_plan = PerPlan::default();
    let mut users = Vec::with_capacity(today.users.len());
    for (id, captures) in &today.users {
        let plan = store.user_plan(id)?;
        match plan {
            Plan::Anon => per_plan.anon += captures,
            Plan::Free => per_plan.free += captures,
            Plan::Paid => per_plan.paid += captures,
        }
        users.push(UserUsage {
            id: id.clone(),
            plan: plan.as_str(),
            captures: *captures,
        });
    }
    Ok(UsageOut {
        day: day.to_string(),
        users,
        ips: today
            .ips
            .iter()
            .map(|(ip, captures)| IpUsage {
                ip: ip.clone(),
                captures: *captures,
            })
            .collect(),
        per_plan,
    })
}

/// `GET /api/v1/admin/blocks`: everything currently blocked.
pub fn blocks(store: &dyn Store) -> Result<Vec<BlockOut>, StoreError> {
    Ok(store
        .blocks()?
        .into_iter()
        .map(|block| match block {
            Block::User(id) => BlockOut {
                kind: "user",
                value: id,
            },
            Block::Ip(ip) => BlockOut {
                kind: "ip",
                value: ip,
            },
        })
        .collect())
}

/// `POST /api/v1/admin/block`. A user may be named by id or by email, exactly as
/// the console's `block user` command. Returns what was printed on the console.
pub fn block(store: &dyn Store, kind: &str, value: &str) -> Result<String, String> {
    let target = match kind {
        "user" => Target::User(value.to_owned()),
        "ip" => Target::Ip(value.to_owned()),
        _ => return Err(r#"kind must be "user" or "ip""#.into()),
    };
    let block = admin::resolve(store, target)?;
    store.block(&block).map_err(|error| error.to_string())?;
    Ok(format!("blocked {}", admin::describe(&block)))
}

/// `POST /api/v1/admin/unblock`, with the same acceptance as `block`.
pub fn unblock(store: &dyn Store, kind: &str, value: &str) -> Result<String, String> {
    let target = match kind {
        "user" => Target::User(value.to_owned()),
        "ip" => Target::Ip(value.to_owned()),
        _ => return Err(r#"kind must be "user" or "ip""#.into()),
    };
    let block = admin::resolve(store, target)?;
    let lifted = store.unblock(&block).map_err(|error| error.to_string())?;
    Ok(if lifted {
        format!("unblocked {}", admin::describe(&block))
    } else {
        format!("{} was not blocked", admin::describe(&block))
    })
}

/// `POST /api/v1/admin/plan`: move a user, by id or email, to another plan.
pub fn set_plan(store: &dyn Store, who: &str, plan: &str) -> Result<String, String> {
    let plan = admin::parse_plan(plan).ok_or("plan must be anon, free or paid")?;
    let id = admin::user_id(store, who)?;
    store
        .set_user_plan(&id, plan)
        .map_err(|error| error.to_string())?;
    Ok(format!("{id} is now on the {} plan", plan.as_str()))
}

/// The `/admin` page: one static document, no build step. It signs in the same way
/// the main page does (the token lives in `rambl_token`) and drives the API above.
pub const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Rambl admin</title>
<style>
  body { font: 15px/1.4 system-ui, sans-serif; margin: 0 auto; max-width: 46rem; padding: 1rem 1.5rem 3rem; color: #2b2418; }
  h1 { font-size: 1.4rem; } h2 { font-size: 1.05rem; margin-top: 1.8rem; }
  table { border-collapse: collapse; width: 100%; }
  th, td { text-align: left; padding: .3rem .6rem .3rem 0; border-bottom: 1px solid #e6dcc6; }
  input, select { padding: .3rem; margin: .2rem .4rem .2rem 0; }
  button { padding: .3rem .8rem; }
  #status { color: #7a6a48; min-height: 1.2em; }
  .error { color: #a33; }
</style>
</head>
<body>
<h1>Rambl admin</h1>
<p id="status">Checking access&hellip;</p>
<section>
  <h2>Today&rsquo;s usage (one capture is one Jev call)</h2>
  <table id="usage"></table>
  <p id="perplan"></p>
</section>
<section>
  <h2>Blocked</h2>
  <ul id="blocks"></ul>
  <form id="block-form">
    <select id="block-kind"><option value="user">user</option><option value="ip">ip</option></select>
    <input id="block-value" placeholder="email, id or ip address" required>
    <button>Block</button>
  </form>
</section>
<section>
  <h2>Plan</h2>
  <form id="plan-form">
    <input id="plan-who" placeholder="email or user id" required>
    <select id="plan-plan"><option value="anon">anon</option><option value="free">free</option><option value="paid">paid</option></select>
    <button>Set plan</button>
  </form>
</section>
<script>
const token = localStorage.getItem("rambl_token");
const status = (text, error) => { document.getElementById("status").textContent = text;
  document.getElementById("status").className = error ? "error" : ""; };
async function api(path, init) {
  const headers = { "content-type": "application/json" };
  if (token) headers["authorization"] = "Bearer " + token;
  const res = await fetch(path, Object.assign({ headers }, init || {}));
  if (!res.ok) {
    if (res.status === 404) throw new Error("not an admin: sign in on the main page, or ask the owner to add your email to RAMBL_ADMIN_EMAILS");
    try { const body = await res.json(); throw new Error(body.error && body.error.message || res.status); }
    catch (e) { throw e instanceof Error ? e : new Error(String(res.status)); }
  }
  return res.json();
}
async function refresh() {
  const [usage, blocks] = await Promise.all([api("/api/v1/admin/usage"), api("/api/v1/admin/blocks")]);
  const table = document.getElementById("usage");
  table.replaceChildren();
  const cell = (tag, text) => { const td = document.createElement(tag); td.textContent = text; return td; };
  const headers = table.createTHead();
  const headerRow = headers.insertRow();
  for (const title of ["user", "plan", "captures"]) headerRow.appendChild(cell("th", title));
  const bodyEl = table.createTBody();
  for (const u of usage.users) {
    const row = bodyEl.insertRow();
    row.appendChild(cell("td", u.id));
    row.appendChild(cell("td", u.plan));
    row.appendChild(cell("td", String(u.captures)));
  }
  const ipRow = bodyEl.insertRow();
  ipRow.appendChild(cell("th", "address"));
  ipRow.appendChild(cell("td", ""));
  ipRow.appendChild(cell("td", ""));
  for (const ip of usage.ips) {
    const row = bodyEl.insertRow();
    row.appendChild(cell("td", ip.ip));
    row.appendChild(cell("td", ""));
    row.appendChild(cell("td", String(ip.captures)));
  }
  const p = usage.per_plan;
  document.getElementById("perplan").textContent = "Jev calls today: anon " + p.anon + ", free " + p.free + ", paid " + p.paid;
  const list = document.getElementById("blocks");
  list.textContent = "";
  if (blocks.length === 0) {
    const empty = document.createElement("li");
    empty.textContent = "nothing is blocked";
    list.appendChild(empty);
  }
  for (const b of blocks) {
    const item = document.createElement("li");
    item.appendChild(cell("span", b.kind + " " + b.value));
    const btn = document.createElement("button");
    btn.textContent = "unblock";
    btn.dataset.kind = b.kind;
    btn.dataset.value = b.value;
    btn.onclick = () => run("unblock", { kind: btn.dataset.kind, value: btn.dataset.value }, btn);
    item.appendChild(btn);
    list.appendChild(item);
  }
}
async function run(action, body, btn) {
  btn.disabled = true;
  try {
    const out = await api(action === "block" ? "/api/v1/admin/block" : "/api/v1/admin/unblock",
      { method: "POST", body: JSON.stringify(body) });
    status(out.message);
    await refresh();
  } catch (e) { status(e.message, true); }
  btn.disabled = false;
}
document.getElementById("block-form").onsubmit = (e) => {
  e.preventDefault();
  run("block", { kind: document.getElementById("block-kind").value, value: document.getElementById("block-value").value }, e.submitter);
};
document.getElementById("plan-form").onsubmit = async (e) => {
  e.preventDefault();
  try {
    const out = await api("/api/v1/admin/plan", { method: "POST",
      body: JSON.stringify({ who: document.getElementById("plan-who").value, plan: document.getElementById("plan-plan").value }) });
    status(out.message);
  } catch (err) { status(err.message, true); }
};
refresh().then(() => status("")).catch((e) => status(e.message, true));
</script>
</body>
</html>"##;

#[cfg(test)]
mod tests {
    use super::super::auth::Me;
    use super::super::store::MemoryStore;
    use super::*;
    use chrono::{TimeZone, Utc};
    use todo_core::{Clock, SystemClock};

    fn signed_in(store: &MemoryStore, id: &str, email: &str, plan: Plan) {
        store
            .upsert_user_and_session(
                &Me {
                    id: id.into(),
                    email: email.into(),
                    name: "Sam".into(),
                    picture: None,
                    plan,
                },
                "token",
                SystemClock.now() + chrono::Duration::days(1),
            )
            .unwrap();
    }

    /// The #161 stored-XSS rule: the page never builds DOM from strings. Every cell
    /// is `createElement` + `textContent` and the tables are cleared with
    /// `replaceChildren`, so a value from the API can never become markup.
    #[test]
    fn the_admin_page_builds_every_row_without_inner_html() {
        assert!(
            !PAGE.contains("innerHTML"),
            "no innerHTML anywhere on the admin page"
        );
        assert!(PAGE.contains("createElement(tag)"));
        assert!(PAGE.contains("td.textContent = text"));
        assert!(PAGE.contains("replaceChildren"));
    }

    #[test]
    fn usage_lists_users_and_plans_and_totals_captures_per_plan() {
        let store = MemoryStore::default();
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap();
        let day = now.date_naive();
        signed_in(&store, "google-1", "a@example.com", Plan::Free);
        signed_in(&store, "google-2", "b@example.com", Plan::Paid);
        store
            .record_capture("google-1", "203.0.113.1", day)
            .unwrap();
        store
            .record_capture("google-1", "203.0.113.1", day)
            .unwrap();
        store
            .record_capture("google-2", "203.0.113.1", day)
            .unwrap();
        store
            .record_capture("anon_1234", "203.0.113.9", day)
            .unwrap();

        let out = usage(&store, day).unwrap();
        assert_eq!(out.day, "2026-10-12");
        assert_eq!(out.users.len(), 3);
        let free = out.users.iter().find(|u| u.id == "google-1").unwrap();
        assert_eq!((free.plan, free.captures), ("free", 2));
        let paid = out.users.iter().find(|u| u.id == "google-2").unwrap();
        assert_eq!((paid.plan, paid.captures), ("paid", 1));
        assert_eq!(out.per_plan.anon, 1);
        assert_eq!(out.per_plan.free, 2);
        assert_eq!(out.per_plan.paid, 1);
        assert_eq!(
            out.ips,
            vec![
                IpUsage {
                    ip: "203.0.113.1".into(),
                    captures: 3
                },
                IpUsage {
                    ip: "203.0.113.9".into(),
                    captures: 1
                },
            ]
        );
    }

    #[test]
    fn block_and_plan_reuse_the_console_resolution() {
        let store = MemoryStore::default();
        signed_in(&store, "google-1", "sam@example.com", Plan::Free);

        let message = block(&store, "user", "Sam@Example.com").unwrap();
        assert_eq!(message, "blocked user google-1");
        assert!(store.is_blocked("google-1", "203.0.113.1").unwrap());
        let listed = blocks(&store).unwrap();
        assert_eq!(listed[0].kind, "user");
        assert_eq!(listed[0].value, "google-1");

        assert_eq!(
            block(&store, "ip", "203.0.113.9").unwrap(),
            "blocked ip 203.0.113.9"
        );
        assert_eq!(
            unblock(&store, "ip", "203.0.113.9").unwrap(),
            "unblocked ip 203.0.113.9"
        );
        assert_eq!(
            unblock(&store, "ip", "203.0.113.9").unwrap(),
            "ip 203.0.113.9 was not blocked"
        );

        let message = set_plan(&store, "sam@example.com", "paid").unwrap();
        assert_eq!(message, "google-1 is now on the paid plan");
        assert_eq!(store.user_plan("google-1").unwrap(), Plan::Paid);
    }

    #[test]
    fn bad_kind_plan_or_unknown_email_is_refused_and_changes_nothing() {
        let store = MemoryStore::default();
        signed_in(&store, "google-1", "sam@example.com", Plan::Free);
        assert!(block(&store, "carrot", "x").is_err());
        assert!(set_plan(&store, "sam@example.com", "gold").is_err());
        assert!(set_plan(&store, "nobody@example.com", "paid").is_err());
        assert!(store.blocks().unwrap().is_empty());
        assert_eq!(store.user_plan("google-1").unwrap(), Plan::Free);
    }
}
