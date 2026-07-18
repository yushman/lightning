use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use rusqlite::{Connection, Result, params};

use crate::score::Verdict;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    Error,
    Skip,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Error => "error",
            Status::Skip => "skip",
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RunPayload {
    pub run_key: String,
    pub sha: String,
    pub branch: String,
    #[serde(default)]
    pub ci_url: Option<String>,
    pub results: Vec<ResultPayload>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ResultPayload {
    pub class_name: String,
    pub name: String,
    pub status: Status,
    #[serde(default)]
    pub time_ms: u64,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildOutcome {
    Success,
    Failed,
}

impl BuildOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildOutcome::Success => "success",
            BuildOutcome::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskOutcome {
    Success,
    UpToDate,
    FromCache,
    Failed,
    Skipped,
}

impl TaskOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskOutcome::Success => "success",
            TaskOutcome::UpToDate => "up-to-date",
            TaskOutcome::FromCache => "from-cache",
            TaskOutcome::Failed => "failed",
            TaskOutcome::Skipped => "skipped",
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct BuildPayload {
    pub build_key: String,
    #[serde(default)]
    pub sha: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub ci_url: Option<String>,
    pub outcome: BuildOutcome,
    pub requested_tasks: String,
    #[serde(default)]
    pub gradle_version: Option<String>,
    #[serde(default)]
    pub java_version: Option<String>,
    pub total_ms: u64,
    pub configuration_ms: u64,
    pub tasks: Vec<TaskPayload>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TaskPayload {
    pub path: String,
    pub outcome: TaskOutcome,
    pub duration_ms: u64,
}

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

pub fn init(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS runs (
            id INTEGER PRIMARY KEY,
            run_key TEXT UNIQUE NOT NULL,
            sha TEXT NOT NULL,
            branch TEXT NOT NULL,
            ci_url TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS tests (
            id INTEGER PRIMARY KEY,
            class_name TEXT NOT NULL,
            name TEXT NOT NULL,
            UNIQUE (class_name, name)
        );
        CREATE TABLE IF NOT EXISTS results (
            id INTEGER PRIMARY KEY,
            run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
            test_id INTEGER NOT NULL REFERENCES tests(id),
            status TEXT NOT NULL,
            time_ms INTEGER NOT NULL,
            message TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_results_test ON results(test_id);
        CREATE INDEX IF NOT EXISTS idx_results_run ON results(run_id);
        CREATE TABLE IF NOT EXISTS builds (
            id INTEGER PRIMARY KEY,
            build_key TEXT UNIQUE NOT NULL,
            sha TEXT,
            branch TEXT,
            ci_url TEXT,
            outcome TEXT NOT NULL,
            requested_tasks TEXT NOT NULL,
            gradle_version TEXT,
            java_version TEXT,
            total_ms INTEGER NOT NULL,
            configuration_ms INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS task_executions (
            id INTEGER PRIMARY KEY,
            build_id INTEGER NOT NULL REFERENCES builds(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            outcome TEXT NOT NULL,
            duration_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_task_executions_build ON task_executions(build_id);",
    )
}

/// Normalizes a test identity component: collapses whitespace and replaces
/// volatile tokens (hex object addresses, UUIDs) so identity is stable across runs.
pub fn normalize(s: &str) -> String {
    static WS: OnceLock<Regex> = OnceLock::new();
    static HEX: OnceLock<Regex> = OnceLock::new();
    static UUID: OnceLock<Regex> = OnceLock::new();
    let ws = WS.get_or_init(|| Regex::new(r"\s+").unwrap());
    let hex = HEX.get_or_init(|| Regex::new(r"@[0-9a-fA-F]{6,}").unwrap());
    let uuid = UUID.get_or_init(|| {
        Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
            .unwrap()
    });
    let s = ws.replace_all(s.trim(), " ");
    let s = hex.replace_all(&s, "_");
    uuid.replace_all(&s, "_").into_owned()
}

/// Inserts a run with its results. Returns (run_id, deduplicated).
pub fn ingest(conn: &mut Connection, run: &RunPayload) -> Result<(i64, bool)> {
    let tx = conn.transaction()?;
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM runs WHERE run_key = ?1",
            [&run.run_key],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| {
            if e == rusqlite::Error::QueryReturnedNoRows {
                Ok(None)
            } else {
                Err(e)
            }
        })?
    {
        return Ok((id, true));
    }
    tx.execute(
        "INSERT INTO runs (run_key, sha, branch, ci_url) VALUES (?1, ?2, ?3, ?4)",
        params![run.run_key, run.sha, run.branch, run.ci_url],
    )?;
    let run_id = tx.last_insert_rowid();
    for r in &run.results {
        let class_name = normalize(&r.class_name);
        let name = normalize(&r.name);
        tx.execute(
            "INSERT OR IGNORE INTO tests (class_name, name) VALUES (?1, ?2)",
            params![class_name, name],
        )?;
        let test_id: i64 = tx.query_row(
            "SELECT id FROM tests WHERE class_name = ?1 AND name = ?2",
            params![class_name, name],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO results (run_id, test_id, status, time_ms, message)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run_id,
                test_id,
                r.status.as_str(),
                r.time_ms as i64,
                r.message
            ],
        )?;
    }
    tx.commit()?;
    Ok((run_id, false))
}

/// Inserts a build with its task executions. Returns (build_id, deduplicated).
pub fn ingest_build(conn: &mut Connection, build: &BuildPayload) -> Result<(i64, bool)> {
    let tx = conn.transaction()?;
    if let Some(id) = tx
        .query_row(
            "SELECT id FROM builds WHERE build_key = ?1",
            [&build.build_key],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| {
            if e == rusqlite::Error::QueryReturnedNoRows {
                Ok(None)
            } else {
                Err(e)
            }
        })?
    {
        return Ok((id, true));
    }
    tx.execute(
        "INSERT INTO builds (build_key, sha, branch, ci_url, outcome, requested_tasks,
                             gradle_version, java_version, total_ms, configuration_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            build.build_key,
            build.sha,
            build.branch,
            build.ci_url,
            build.outcome.as_str(),
            build.requested_tasks,
            build.gradle_version,
            build.java_version,
            build.total_ms as i64,
            build.configuration_ms as i64,
        ],
    )?;
    let build_id = tx.last_insert_rowid();
    for t in &build.tasks {
        tx.execute(
            "INSERT INTO task_executions (build_id, path, outcome, duration_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![build_id, t.path, t.outcome.as_str(), t.duration_ms as i64],
        )?;
    }
    tx.commit()?;
    Ok((build_id, false))
}

/// Deletes runs and builds older than the retention window plus tests left without results.
pub fn prune(conn: &Connection, retention_days: u32) -> Result<()> {
    let cutoff = format!("-{retention_days} days");
    conn.execute(
        "DELETE FROM runs WHERE created_at < datetime('now', ?1)",
        [&cutoff],
    )?;
    conn.execute(
        "DELETE FROM tests WHERE id NOT IN (SELECT DISTINCT test_id FROM results)",
        [],
    )?;
    conn.execute(
        "DELETE FROM builds WHERE created_at < datetime('now', ?1)",
        [&cutoff],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct TestRow {
    pub id: i64,
    pub class_name: String,
    pub name: String,
}

pub fn all_tests(conn: &Connection) -> Result<Vec<TestRow>> {
    conn.prepare("SELECT id, class_name, name FROM tests ORDER BY class_name, name")?
        .query_map([], |r| {
            Ok(TestRow {
                id: r.get(0)?,
                class_name: r.get(1)?,
                name: r.get(2)?,
            })
        })?
        .collect()
}

pub fn test(conn: &Connection, id: i64) -> Result<Option<TestRow>> {
    conn.query_row(
        "SELECT id, class_name, name FROM tests WHERE id = ?1",
        [id],
        |r| {
            Ok(TestRow {
                id: r.get(0)?,
                class_name: r.get(1)?,
                name: r.get(2)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(e)
        }
    })
}

/// One run's verdict for a test, newest first.
#[derive(Debug, Clone)]
pub struct VerdictRow {
    pub run_id: i64,
    pub sha: String,
    pub branch: String,
    pub created_at: String,
    pub verdict: Option<Verdict>,
}

pub fn verdict_window(conn: &Connection, test_id: i64, limit: usize) -> Result<Vec<VerdictRow>> {
    conn.prepare(
        "SELECT runs.id, runs.sha, runs.branch, runs.created_at,
                SUM(results.status = 'pass') AS passes,
                SUM(results.status IN ('fail', 'error')) AS fails
         FROM results JOIN runs ON runs.id = results.run_id
         WHERE results.test_id = ?1
         GROUP BY runs.id
         ORDER BY runs.created_at DESC, runs.id DESC
         LIMIT ?2",
    )?
    .query_map(params![test_id, limit as i64], |r| {
        let passes: i64 = r.get(4)?;
        let fails: i64 = r.get(5)?;
        Ok(VerdictRow {
            run_id: r.get(0)?,
            sha: r.get(1)?,
            branch: r.get(2)?,
            created_at: r.get(3)?,
            verdict: Verdict::from_counts(passes, fails),
        })
    })?
    .collect()
}

#[derive(Debug, Clone)]
pub struct RunRow {
    pub run_key: String,
    pub sha: String,
    pub branch: String,
    pub ci_url: Option<String>,
    pub created_at: String,
}

pub fn run(conn: &Connection, id: i64) -> Result<Option<RunRow>> {
    conn.query_row(
        "SELECT run_key, sha, branch, ci_url, created_at FROM runs WHERE id = ?1",
        [id],
        |r| {
            Ok(RunRow {
                run_key: r.get(0)?,
                sha: r.get(1)?,
                branch: r.get(2)?,
                ci_url: r.get(3)?,
                created_at: r.get(4)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(e)
        }
    })
}

/// Per-test verdict within one run.
#[derive(Debug, Clone)]
pub struct RunTestRow {
    pub test_id: i64,
    pub class_name: String,
    pub name: String,
    pub verdict: Option<Verdict>,
}

pub fn run_tests(conn: &Connection, run_id: i64) -> Result<Vec<RunTestRow>> {
    conn.prepare(
        "SELECT tests.id, tests.class_name, tests.name,
                SUM(results.status = 'pass') AS passes,
                SUM(results.status IN ('fail', 'error')) AS fails
         FROM results JOIN tests ON tests.id = results.test_id
         WHERE results.run_id = ?1
         GROUP BY tests.id
         ORDER BY tests.class_name, tests.name",
    )?
    .query_map([run_id], |r| {
        let passes: i64 = r.get(3)?;
        let fails: i64 = r.get(4)?;
        Ok(RunTestRow {
            test_id: r.get(0)?,
            class_name: r.get(1)?,
            name: r.get(2)?,
            verdict: Verdict::from_counts(passes, fails),
        })
    })?
    .collect()
}

/// One build with aggregated task outcome counts.
#[derive(Debug, Clone)]
pub struct BuildRow {
    pub id: i64,
    pub build_key: String,
    pub sha: Option<String>,
    pub branch: Option<String>,
    pub ci_url: Option<String>,
    pub outcome: String,
    pub requested_tasks: String,
    pub gradle_version: Option<String>,
    pub java_version: Option<String>,
    pub total_ms: i64,
    pub configuration_ms: i64,
    pub created_at: String,
    pub tasks: i64,
    pub success: i64,
    pub up_to_date: i64,
    pub from_cache: i64,
    pub failed: i64,
    pub skipped: i64,
}

impl BuildRow {
    /// Tasks whose work was avoided (up-to-date or from-cache).
    pub fn avoided(&self) -> i64 {
        self.up_to_date + self.from_cache
    }
}

const BUILD_ROW_SQL: &str = "SELECT b.id, b.build_key, b.sha, b.branch, b.ci_url, b.outcome,
        b.requested_tasks, b.gradle_version, b.java_version, b.total_ms, b.configuration_ms,
        b.created_at,
        COUNT(t.id),
        COALESCE(SUM(t.outcome = 'success'), 0),
        COALESCE(SUM(t.outcome = 'up-to-date'), 0),
        COALESCE(SUM(t.outcome = 'from-cache'), 0),
        COALESCE(SUM(t.outcome = 'failed'), 0),
        COALESCE(SUM(t.outcome = 'skipped'), 0)
 FROM builds b LEFT JOIN task_executions t ON t.build_id = b.id";

fn build_row(r: &rusqlite::Row) -> rusqlite::Result<BuildRow> {
    Ok(BuildRow {
        id: r.get(0)?,
        build_key: r.get(1)?,
        sha: r.get(2)?,
        branch: r.get(3)?,
        ci_url: r.get(4)?,
        outcome: r.get(5)?,
        requested_tasks: r.get(6)?,
        gradle_version: r.get(7)?,
        java_version: r.get(8)?,
        total_ms: r.get(9)?,
        configuration_ms: r.get(10)?,
        created_at: r.get(11)?,
        tasks: r.get(12)?,
        success: r.get(13)?,
        up_to_date: r.get(14)?,
        from_cache: r.get(15)?,
        failed: r.get(16)?,
        skipped: r.get(17)?,
    })
}

/// Recent builds, newest first.
pub fn builds(conn: &Connection, limit: usize) -> Result<Vec<BuildRow>> {
    conn.prepare(&format!(
        "{BUILD_ROW_SQL} GROUP BY b.id ORDER BY b.created_at DESC, b.id DESC LIMIT ?1"
    ))?
    .query_map([limit as i64], build_row)?
    .collect()
}

pub fn build(conn: &Connection, id: i64) -> Result<Option<BuildRow>> {
    conn.query_row(
        &format!("{BUILD_ROW_SQL} WHERE b.id = ?1 GROUP BY b.id"),
        [id],
        build_row,
    )
    .map(Some)
    .or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(e)
        }
    })
}

#[derive(Debug, Clone)]
pub struct TaskExecutionRow {
    pub path: String,
    pub outcome: String,
    pub duration_ms: i64,
}

/// A build's task executions, slowest first.
pub fn build_tasks(
    conn: &Connection,
    build_id: i64,
    limit: usize,
) -> Result<Vec<TaskExecutionRow>> {
    conn.prepare(
        "SELECT path, outcome, duration_ms FROM task_executions
         WHERE build_id = ?1 ORDER BY duration_ms DESC, path LIMIT ?2",
    )?
    .query_map(params![build_id, limit as i64], |r| {
        Ok(TaskExecutionRow {
            path: r.get(0)?,
            outcome: r.get(1)?,
            duration_ms: r.get(2)?,
        })
    })?
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(run_key: &str, sha: &str, results: Vec<(&str, Status)>) -> RunPayload {
        RunPayload {
            run_key: run_key.into(),
            sha: sha.into(),
            branch: "main".into(),
            ci_url: None,
            results: results
                .into_iter()
                .map(|(name, status)| ResultPayload {
                    class_name: "com.example.FooTest".into(),
                    name: name.into(),
                    status,
                    time_ms: 10,
                    message: None,
                })
                .collect(),
        }
    }

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn
    }

    #[test]
    fn normalize_collapses_whitespace_and_volatile_tokens() {
        assert_eq!(normalize("  a   b\tc "), "a b c");
        assert_eq!(normalize("test[Foo@1a2b3c4d]"), "test[Foo_]");
        assert_eq!(
            normalize("test[id=123e4567-e89b-12d3-a456-426614174000]"),
            "test[id=_]"
        );
        assert_eq!(normalize("test[2]"), "test[2]");
    }

    #[test]
    fn ingest_dedupes_by_run_key() {
        let mut conn = mem();
        let p = payload("k1", "sha1", vec![("t", Status::Pass)]);
        let (id1, dedup1) = ingest(&mut conn, &p).unwrap();
        let (id2, dedup2) = ingest(&mut conn, &p).unwrap();
        assert!(!dedup1);
        assert!(dedup2);
        assert_eq!(id1, id2);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM results", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn same_identity_maps_to_one_test_row() {
        let mut conn = mem();
        ingest(&mut conn, &payload("k1", "s1", vec![("t ", Status::Pass)])).unwrap();
        ingest(&mut conn, &payload("k2", "s2", vec![(" t", Status::Fail)])).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM tests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn prune_removes_old_runs_and_orphan_tests() {
        let mut conn = mem();
        ingest(&mut conn, &payload("old", "s1", vec![("t", Status::Pass)])).unwrap();
        conn.execute(
            "UPDATE runs SET created_at = datetime('now', '-100 days')",
            [],
        )
        .unwrap();
        ingest(&mut conn, &payload("new", "s2", vec![("u", Status::Pass)])).unwrap();
        prune(&conn, 90).unwrap();
        let runs: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap();
        let tests: i64 = conn
            .query_row("SELECT COUNT(*) FROM tests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(runs, 1);
        assert_eq!(tests, 1);
    }

    fn build_payload(build_key: &str, tasks: Vec<(&str, TaskOutcome)>) -> BuildPayload {
        BuildPayload {
            build_key: build_key.into(),
            sha: Some("sha1".into()),
            branch: Some("main".into()),
            ci_url: None,
            outcome: BuildOutcome::Success,
            requested_tasks: "build".into(),
            gradle_version: Some("9.6.1".into()),
            java_version: Some("21".into()),
            total_ms: 1000,
            configuration_ms: 100,
            tasks: tasks
                .into_iter()
                .map(|(path, outcome)| TaskPayload {
                    path: path.into(),
                    outcome,
                    duration_ms: 10,
                })
                .collect(),
        }
    }

    #[test]
    fn ingest_build_dedupes_by_build_key() {
        let mut conn = mem();
        let p = build_payload(
            "b1",
            vec![
                (":a:compile", TaskOutcome::Success),
                (":a:test", TaskOutcome::FromCache),
            ],
        );
        let (id1, dedup1) = ingest_build(&mut conn, &p).unwrap();
        let (id2, dedup2) = ingest_build(&mut conn, &p).unwrap();
        assert!(!dedup1);
        assert!(dedup2);
        assert_eq!(id1, id2);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_executions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn build_rows_aggregate_outcome_counts() {
        let mut conn = mem();
        ingest_build(
            &mut conn,
            &build_payload(
                "b1",
                vec![
                    (":a", TaskOutcome::Success),
                    (":b", TaskOutcome::UpToDate),
                    (":c", TaskOutcome::FromCache),
                    (":d", TaskOutcome::Skipped),
                ],
            ),
        )
        .unwrap();
        ingest_build(&mut conn, &build_payload("b2", vec![])).unwrap();
        let rows = builds(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        // newest first: b2 has no tasks
        assert_eq!(rows[0].build_key, "b2");
        assert_eq!(rows[0].tasks, 0);
        let b1 = &rows[1];
        assert_eq!(b1.tasks, 4);
        assert_eq!(b1.success, 1);
        assert_eq!(b1.avoided(), 2);
        assert_eq!(b1.skipped, 1);
        let one = build(&conn, b1.id).unwrap().unwrap();
        assert_eq!(one.build_key, "b1");
        assert!(build(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn build_tasks_ordered_by_duration() {
        let mut conn = mem();
        let mut p = build_payload("b1", vec![(":fast", TaskOutcome::Success)]);
        p.tasks.push(TaskPayload {
            path: ":slow".into(),
            outcome: TaskOutcome::Success,
            duration_ms: 500,
        });
        let (id, _) = ingest_build(&mut conn, &p).unwrap();
        let tasks = build_tasks(&conn, id, 10).unwrap();
        assert_eq!(tasks[0].path, ":slow");
        assert_eq!(tasks[0].duration_ms, 500);
    }

    #[test]
    fn prune_removes_old_builds() {
        let mut conn = mem();
        ingest_build(
            &mut conn,
            &build_payload("old", vec![(":a", TaskOutcome::Success)]),
        )
        .unwrap();
        conn.execute(
            "UPDATE builds SET created_at = datetime('now', '-100 days')",
            [],
        )
        .unwrap();
        ingest_build(&mut conn, &build_payload("new", vec![])).unwrap();
        prune(&conn, 90).unwrap();
        let builds_left: i64 = conn
            .query_row("SELECT COUNT(*) FROM builds", [], |r| r.get(0))
            .unwrap();
        let tasks_left: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_executions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(builds_left, 1);
        assert_eq!(tasks_left, 0);
    }

    #[test]
    fn verdict_window_derives_mixed() {
        let mut conn = mem();
        ingest(
            &mut conn,
            &payload("k1", "s1", vec![("t", Status::Fail), ("t", Status::Pass)]),
        )
        .unwrap();
        let w = verdict_window(&conn, 1, 50).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].verdict, Some(Verdict::Mixed));
    }
}
