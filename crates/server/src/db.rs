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
        CREATE INDEX IF NOT EXISTS idx_results_run ON results(run_id);",
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

/// Deletes runs older than the retention window plus tests left without results.
pub fn prune(conn: &Connection, retention_days: u32) -> Result<()> {
    conn.execute(
        "DELETE FROM runs WHERE created_at < datetime('now', ?1)",
        [format!("-{retention_days} days")],
    )?;
    conn.execute(
        "DELETE FROM tests WHERE id NOT IN (SELECT DISTINCT test_id FROM results)",
        [],
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
