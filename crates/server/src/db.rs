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
        CREATE INDEX IF NOT EXISTS idx_task_executions_build ON task_executions(build_id);
        CREATE TABLE IF NOT EXISTS cache_entries (
            key TEXT PRIMARY KEY,
            size INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            last_accessed_at TEXT NOT NULL DEFAULT (datetime('now')),
            hit_count INTEGER NOT NULL DEFAULT 0
        );",
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

#[derive(Debug, Clone)]
pub struct CacheEntryRow {
    pub key: String,
    pub size: i64,
    pub created_at: String,
    pub last_accessed_at: String,
    pub hit_count: i64,
}

/// Inserts or overwrites a cache entry; an overwrite refreshes size and
/// timestamps but keeps the accumulated hit count.
pub fn cache_upsert(conn: &Connection, key: &str, size: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO cache_entries (key, size) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET size = excluded.size,
             created_at = datetime('now'), last_accessed_at = datetime('now')",
        params![key, size],
    )?;
    Ok(())
}

/// Returns whether the key is indexed; when it is, refreshes the last access
/// time and, if `count_hit`, increments the hit count.
pub fn cache_touch(conn: &Connection, key: &str, count_hit: bool) -> Result<bool> {
    let n = conn.execute(
        "UPDATE cache_entries SET last_accessed_at = datetime('now'),
             hit_count = hit_count + ?2 WHERE key = ?1",
        params![key, count_hit as i64],
    )?;
    Ok(n > 0)
}

pub fn cache_remove(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM cache_entries WHERE key = ?1", [key])?;
    Ok(())
}

pub fn cache_keys(conn: &Connection) -> Result<Vec<String>> {
    conn.prepare("SELECT key FROM cache_entries")?
        .query_map([], |r| r.get(0))?
        .collect()
}

/// (entry count, total indexed bytes)
pub fn cache_totals(conn: &Connection) -> Result<(i64, i64)> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM cache_entries",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Deletes least-recently-accessed entries (never `keep_key`) until the total
/// fits `max_total`. Returns the evicted keys; the caller deletes their files.
pub fn cache_evict_lru(conn: &Connection, max_total: i64, keep_key: &str) -> Result<Vec<String>> {
    let (_, mut total) = cache_totals(conn)?;
    if total <= max_total {
        return Ok(Vec::new());
    }
    let candidates: Vec<(String, i64)> = conn
        .prepare(
            "SELECT key, size FROM cache_entries WHERE key != ?1
             ORDER BY last_accessed_at, key",
        )?
        .query_map([keep_key], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_>>()?;
    let mut evicted = Vec::new();
    for (key, size) in candidates {
        if total <= max_total {
            break;
        }
        conn.execute("DELETE FROM cache_entries WHERE key = ?1", [&key])?;
        total -= size;
        evicted.push(key);
    }
    Ok(evicted)
}

/// Deletes entries not accessed within the retention window. Returns the
/// pruned keys; the caller deletes their files.
pub fn cache_prune_expired(conn: &Connection, retention_days: u32) -> Result<Vec<String>> {
    let cutoff = format!("-{retention_days} days");
    conn.prepare(
        "DELETE FROM cache_entries WHERE last_accessed_at < datetime('now', ?1) RETURNING key",
    )?
    .query_map([&cutoff], |r| r.get(0))?
    .collect()
}

/// Stored artifacts by hit count descending.
pub fn cache_top_entries(conn: &Connection, limit: usize) -> Result<Vec<CacheEntryRow>> {
    conn.prepare(
        "SELECT key, size, created_at, last_accessed_at, hit_count FROM cache_entries
         ORDER BY hit_count DESC, last_accessed_at DESC, key LIMIT ?1",
    )?
    .query_map([limit as i64], |r| {
        Ok(CacheEntryRow {
            key: r.get(0)?,
            size: r.get(1)?,
            created_at: r.get(2)?,
            last_accessed_at: r.get(3)?,
            hit_count: r.get(4)?,
        })
    })?
    .collect()
}

/// Overall task cache hit numbers over all retained builds:
/// (from-cache tasks, tasks that needed work = from-cache + success + failed).
pub fn cache_hit_totals(conn: &Connection) -> Result<(i64, i64)> {
    conn.query_row(
        "SELECT COALESCE(SUM(outcome = 'from-cache'), 0),
                COALESCE(SUM(outcome IN ('from-cache', 'success', 'failed')), 0)
         FROM task_executions",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

#[derive(Debug, Clone)]
pub struct NeverCachedRow {
    pub path: String,
    pub executions: i64,
    pub total_ms: i64,
}

/// Task paths executed (success/failed) at least `min_executions` times across
/// the last `builds_window` builds with no from-cache or up-to-date outcome,
/// ordered by total execution time descending (= potential savings).
pub fn never_cached_tasks(
    conn: &Connection,
    builds_window: usize,
    min_executions: i64,
    limit: usize,
) -> Result<Vec<NeverCachedRow>> {
    conn.prepare(
        "WITH recent AS (SELECT id FROM builds ORDER BY created_at DESC, id DESC LIMIT ?1)
         SELECT t.path,
                SUM(t.outcome IN ('success', 'failed')) AS executions,
                SUM(CASE WHEN t.outcome IN ('success', 'failed') THEN t.duration_ms ELSE 0 END)
                    AS total_ms
         FROM task_executions t JOIN recent ON t.build_id = recent.id
         GROUP BY t.path
         HAVING executions >= ?2 AND SUM(t.outcome IN ('from-cache', 'up-to-date')) = 0
         ORDER BY total_ms DESC, t.path
         LIMIT ?3",
    )?
    .query_map(
        params![builds_window as i64, min_executions, limit as i64],
        |r| {
            Ok(NeverCachedRow {
                path: r.get(0)?,
                executions: r.get(1)?,
                total_ms: r.get(2)?,
            })
        },
    )?
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
    fn cache_upsert_touch_and_totals() {
        let conn = mem();
        cache_upsert(&conn, "a".repeat(32).as_str(), 100).unwrap();
        cache_upsert(&conn, "b".repeat(32).as_str(), 200).unwrap();
        assert_eq!(cache_totals(&conn).unwrap(), (2, 300));
        // hit counted on GET, not on HEAD
        assert!(cache_touch(&conn, &"a".repeat(32), true).unwrap());
        assert!(cache_touch(&conn, &"a".repeat(32), false).unwrap());
        assert!(!cache_touch(&conn, &"c".repeat(32), true).unwrap());
        let top = cache_top_entries(&conn, 10).unwrap();
        assert_eq!(top[0].key, "a".repeat(32));
        assert_eq!(top[0].hit_count, 1);
        // overwrite keeps the hit count, refreshes size
        cache_upsert(&conn, "a".repeat(32).as_str(), 150).unwrap();
        let top = cache_top_entries(&conn, 10).unwrap();
        assert_eq!(top[0].hit_count, 1);
        assert_eq!(top[0].size, 150);
        cache_remove(&conn, &"a".repeat(32)).unwrap();
        assert_eq!(cache_totals(&conn).unwrap(), (1, 200));
    }

    #[test]
    fn cache_evicts_least_recently_accessed_first() {
        let conn = mem();
        for (key, age_days) in [("a", 3), ("b", 2), ("c", 1)] {
            cache_upsert(&conn, key.repeat(32).as_str(), 100).unwrap();
            conn.execute(
                "UPDATE cache_entries SET last_accessed_at = datetime('now', ?1) WHERE key = ?2",
                params![format!("-{age_days} days"), key.repeat(32)],
            )
            .unwrap();
        }
        cache_upsert(&conn, "d".repeat(32).as_str(), 100).unwrap();
        // fits: nothing evicted
        assert!(
            cache_evict_lru(&conn, 400, &"d".repeat(32))
                .unwrap()
                .is_empty()
        );
        // oldest-accessed go first, the just-written key is never evicted
        let evicted = cache_evict_lru(&conn, 200, &"d".repeat(32)).unwrap();
        assert_eq!(evicted, vec!["a".repeat(32), "b".repeat(32)]);
        let evicted = cache_evict_lru(&conn, 0, &"d".repeat(32)).unwrap();
        assert_eq!(evicted, vec!["c".repeat(32)]);
        assert_eq!(cache_keys(&conn).unwrap(), vec!["d".repeat(32)]);
    }

    #[test]
    fn cache_prune_expired_removes_only_stale_entries() {
        let conn = mem();
        cache_upsert(&conn, "a".repeat(32).as_str(), 100).unwrap();
        cache_upsert(&conn, "b".repeat(32).as_str(), 100).unwrap();
        conn.execute(
            "UPDATE cache_entries SET last_accessed_at = datetime('now', '-40 days')
             WHERE key = ?1",
            [&"a".repeat(32)],
        )
        .unwrap();
        let pruned = cache_prune_expired(&conn, 30).unwrap();
        assert_eq!(pruned, vec!["a".repeat(32)]);
        assert_eq!(cache_keys(&conn).unwrap(), vec!["b".repeat(32)]);
    }

    #[test]
    fn cache_hit_totals_count_work_needed_only() {
        let mut conn = mem();
        ingest_build(
            &mut conn,
            &build_payload(
                "b1",
                vec![
                    (":a", TaskOutcome::FromCache),
                    (":b", TaskOutcome::Success),
                    (":c", TaskOutcome::Failed),
                    (":d", TaskOutcome::UpToDate),
                    (":e", TaskOutcome::Skipped),
                ],
            ),
        )
        .unwrap();
        // up-to-date and skipped are excluded from the denominator
        assert_eq!(cache_hit_totals(&conn).unwrap(), (1, 3));
    }

    #[test]
    fn never_cached_requires_executions_and_no_cache_outcomes() {
        let mut conn = mem();
        for i in 0..3 {
            ingest_build(
                &mut conn,
                &build_payload(
                    &format!("b{i}"),
                    vec![
                        (":never", TaskOutcome::Success),
                        (
                            ":sometimes",
                            if i == 0 {
                                TaskOutcome::FromCache
                            } else {
                                TaskOutcome::Success
                            },
                        ),
                        (
                            ":upToDate",
                            if i == 0 {
                                TaskOutcome::UpToDate
                            } else {
                                TaskOutcome::Success
                            },
                        ),
                        (
                            ":rare",
                            if i == 0 {
                                TaskOutcome::Success
                            } else {
                                TaskOutcome::Skipped
                            },
                        ),
                    ],
                ),
            )
            .unwrap();
        }
        let rows = never_cached_tasks(&conn, 100, 3, 50).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, ":never");
        assert_eq!(rows[0].executions, 3);
        assert_eq!(rows[0].total_ms, 30);
        // shrinking the window below the qualifying executions drops the signal
        assert!(never_cached_tasks(&conn, 2, 3, 50).unwrap().is_empty());
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
