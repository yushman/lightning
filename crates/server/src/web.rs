use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Html;
use serde_json::json;

use crate::App;
use crate::db::{self, RunPayload, TestRow, VerdictRow};
use crate::score::{Score, Verdict, WindowEntry, score};

pub const WINDOW: usize = 50;
const TREND_LEN: usize = 20;

fn internal(e: rusqlite::Error) -> StatusCode {
    eprintln!("db error: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}

pub async fn ingest(
    State(app): State<Arc<App>>,
    Json(payload): Json<RunPayload>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    if payload.run_key.is_empty() || payload.sha.is_empty() || payload.branch.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut conn = app.db.lock().unwrap();
    let (run_id, deduplicated) = db::ingest(&mut conn, &payload).map_err(internal)?;
    db::prune(&conn, app.retention_days).map_err(internal)?;
    let status = if deduplicated {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(json!({ "run_id": run_id, "deduplicated": deduplicated })),
    ))
}

struct FlakyTest {
    test: TestRow,
    score: Score,
    last_seen: String,
    /// Chronological verdicts, oldest first.
    window: Vec<VerdictRow>,
}

fn flaky_tests(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<FlakyTest>> {
    let mut out = Vec::new();
    for test in db::all_tests(conn)? {
        let mut window = db::verdict_window(conn, test.id, WINDOW)?;
        window.reverse();
        window.retain(|v| v.verdict.is_some());
        let entries: Vec<WindowEntry> = window
            .iter()
            .map(|v| WindowEntry {
                sha: v.sha.clone(),
                verdict: v.verdict.unwrap(),
            })
            .collect();
        let s = score(&entries);
        if s.score > 0 {
            let last_seen = window
                .last()
                .map(|v| v.created_at.clone())
                .unwrap_or_default();
            out.push(FlakyTest {
                test,
                score: s,
                last_seen,
                window,
            });
        }
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.score.score));
    Ok(out)
}

pub async fn flaky_api(State(app): State<Arc<App>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let flaky = flaky_tests(&conn).map_err(internal)?;
    let items: Vec<serde_json::Value> = flaky
        .iter()
        .map(|f| {
            json!({
                "test_id": f.test.id,
                "class_name": f.test.class_name,
                "name": f.test.name,
                "score": f.score.score,
                "flip_shas": f.score.flip_shas,
                "flips": f.score.flips,
                "last_seen": f.last_seen,
            })
        })
        .collect();
    Ok(Json(json!(items)))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — lightning</title>
<style>
body {{ font: 15px/1.5 -apple-system, "Segoe UI", sans-serif; margin: 2rem auto; max-width: 60rem; padding: 0 1rem; color: #1a1a1a; }}
h1 {{ font-size: 1.4rem; }} h1 a {{ color: inherit; text-decoration: none; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ text-align: left; padding: .4rem .6rem; border-bottom: 1px solid #e0e0e0; }}
code {{ font-size: .85em; background: #f2f2f2; padding: .1em .3em; border-radius: 3px; }}
.score {{ font-weight: 600; }}
.v {{ display: inline-block; width: .7em; height: 1em; margin-right: 2px; border-radius: 2px; vertical-align: middle; }}
.v.pass {{ background: #2da44e; }} .v.fail {{ background: #cf222e; }} .v.mixed {{ background: #d4a72c; }}
.verdict.pass {{ color: #2da44e; }} .verdict.fail {{ color: #cf222e; }} .verdict.mixed {{ color: #b08800; }}
.muted {{ color: #666; }}
</style>
</head>
<body>
<h1><a href="/">lightning</a> · {title}</h1>
{body}
</body>
</html>
"#,
        title = esc(title),
    ))
}

fn verdict_label(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "pass",
        Verdict::Fail => "fail",
        Verdict::Mixed => "mixed",
    }
}

fn trend(window: &[VerdictRow]) -> String {
    window
        .iter()
        .rev()
        .take(TREND_LEN)
        .rev()
        .filter_map(|v| {
            let label = verdict_label(v.verdict?);
            Some(format!(
                r#"<span class="v {label}" title="{} on {}"></span>"#,
                label,
                esc(&v.sha[..v.sha.len().min(9)]),
            ))
        })
        .collect()
}

pub async fn flaky_page(State(app): State<Arc<App>>) -> Result<Html<String>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let flaky = flaky_tests(&conn).map_err(internal)?;
    if flaky.is_empty() {
        return Ok(page("flaky tests", "<p>No flaky tests detected.</p>"));
    }
    let rows: String = flaky
        .iter()
        .map(|f| {
            format!(
                r#"<tr><td><a href="/tests/{id}">{class}<br><code>{name}</code></a></td>
<td class="score">{score}</td><td>{trend}</td><td class="muted">{seen}</td></tr>"#,
                id = f.test.id,
                class = esc(&f.test.class_name),
                name = esc(&f.test.name),
                score = f.score.score,
                trend = trend(&f.window),
                seen = esc(&f.last_seen),
            )
        })
        .collect();
    let body = format!(
        "<table><tr><th>Test</th><th>Score</th><th>Trend (old → new)</th><th>Last seen</th></tr>{rows}</table>"
    );
    Ok(page("flaky tests", &body))
}

pub async fn test_page(
    State(app): State<Arc<App>>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let test = db::test(&conn, id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let window = db::verdict_window(&conn, id, WINDOW).map_err(internal)?;
    let entries: Vec<WindowEntry> = window
        .iter()
        .rev()
        .filter_map(|v| {
            Some(WindowEntry {
                sha: v.sha.clone(),
                verdict: v.verdict?,
            })
        })
        .collect();
    let s = score(&entries);
    let rows: String = window
        .iter()
        .map(|v| {
            let (label, class) = match v.verdict {
                Some(verdict) => (verdict_label(verdict), verdict_label(verdict)),
                None => ("skip", "muted"),
            };
            format!(
                r#"<tr><td class="muted">{time}</td><td class="verdict {class}">{label}</td>
<td><code>{sha}</code></td><td>{branch}</td><td><a href="/runs/{run_id}">run #{run_id}</a></td></tr>"#,
                time = esc(&v.created_at),
                sha = esc(&v.sha[..v.sha.len().min(9)]),
                branch = esc(&v.branch),
                run_id = v.run_id,
            )
        })
        .collect();
    let body = format!(
        r#"<p><code>{class}</code> · <code>{name}</code></p>
<p>Flaky score: <span class="score">{score}</span> ({flip_shas} same-sha flip sha(s), {flips} cross-run flips in last {window_len} runs)</p>
<table><tr><th>Time (UTC)</th><th>Verdict</th><th>SHA</th><th>Branch</th><th>Run</th></tr>{rows}</table>"#,
        class = esc(&test.class_name),
        name = esc(&test.name),
        score = s.score,
        flip_shas = s.flip_shas,
        flips = s.flips,
        window_len = window.len(),
    );
    Ok(page("test history", &body))
}

pub async fn run_page(
    State(app): State<Arc<App>>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let run = db::run(&conn, id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tests = db::run_tests(&conn, id).map_err(internal)?;
    let count = |v: Option<Verdict>| tests.iter().filter(|t| t.verdict == v).count();
    let problems: String = tests
        .iter()
        .filter(|t| matches!(t.verdict, Some(Verdict::Fail | Verdict::Mixed)))
        .map(|t| {
            let label = verdict_label(t.verdict.unwrap());
            format!(
                r#"<tr><td class="verdict {label}">{label}</td>
<td><a href="/tests/{id}">{class}<br><code>{name}</code></a></td></tr>"#,
                id = t.test_id,
                class = esc(&t.class_name),
                name = esc(&t.name),
            )
        })
        .collect();
    let problems = if problems.is_empty() {
        "<p>No failed or mixed tests.</p>".to_string()
    } else {
        format!("<table><tr><th>Verdict</th><th>Test</th></tr>{problems}</table>")
    };
    let ci = run
        .ci_url
        .as_deref()
        .map(|u| format!(r#" · <a href="{0}">CI run</a>"#, esc(u)))
        .unwrap_or_default();
    let body = format!(
        r#"<p><code>{sha}</code> on <b>{branch}</b> · {time} UTC{ci}</p>
<p class="muted">run key <code>{key}</code></p>
<p>{passed} passed · {failed} failed · {mixed} mixed · {skipped} skipped</p>
{problems}"#,
        sha = esc(&run.sha),
        branch = esc(&run.branch),
        time = esc(&run.created_at),
        key = esc(&run.run_key),
        passed = count(Some(Verdict::Pass)),
        failed = count(Some(Verdict::Fail)),
        mixed = count(Some(Verdict::Mixed)),
        skipped = count(None),
    );
    Ok(page(&format!("run #{id}"), &body))
}
