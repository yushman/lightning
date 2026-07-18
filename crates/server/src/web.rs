use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Html;
use serde_json::json;

use crate::App;
use crate::db::{self, BuildPayload, BuildRow, RunPayload, TestRow, VerdictRow};
use crate::score::{Score, Verdict, WindowEntry, score};

pub const WINDOW: usize = 50;
const TREND_LEN: usize = 20;

pub(crate) fn internal(e: rusqlite::Error) -> StatusCode {
    eprintln!("db error: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}

pub async fn ingest(
    State(app): State<Arc<App>>,
    payload: Result<Json<RunPayload>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    // The spec promises 400 for any malformed payload; axum's default is 422.
    let Ok(Json(payload)) = payload else {
        return Err(StatusCode::BAD_REQUEST);
    };
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
nav {{ margin-bottom: 1rem; }} nav a {{ margin-right: .8rem; }}
.outcome.success {{ color: #2da44e; }} .outcome.failed {{ color: #cf222e; }}
.bar {{ display: inline-block; height: .7em; background: #54aeff; border-radius: 2px; vertical-align: middle; }}
.num {{ text-align: right; }}
</style>
</head>
<body>
<h1><a href="/">lightning</a> · {title}</h1>
<nav><a href="/">flaky</a><a href="/builds">builds</a><a href="/trends">trends</a><a href="/cache">cache</a></nav>
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

const BUILDS_LIMIT: usize = 100;
const SLOWEST_TASKS: usize = 20;
const TREND_WINDOW: usize = 50;
const TREND_FETCH: usize = 500;

pub async fn ingest_build(
    State(app): State<Arc<App>>,
    payload: Result<Json<BuildPayload>, JsonRejection>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let Ok(Json(payload)) = payload else {
        return Err(StatusCode::BAD_REQUEST);
    };
    if payload.build_key.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut conn = app.db.lock().unwrap();
    let (build_id, deduplicated) = db::ingest_build(&mut conn, &payload).map_err(internal)?;
    db::prune(&conn, app.retention_days).map_err(internal)?;
    let status = if deduplicated {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(json!({ "build_id": build_id, "deduplicated": deduplicated })),
    ))
}

pub async fn builds_api(
    State(app): State<Arc<App>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let builds = db::builds(&conn, BUILDS_LIMIT).map_err(internal)?;
    let items: Vec<serde_json::Value> = builds
        .iter()
        .map(|b| {
            json!({
                "build_id": b.id,
                "build_key": b.build_key,
                "sha": b.sha,
                "branch": b.branch,
                "outcome": b.outcome,
                "total_ms": b.total_ms,
                "configuration_ms": b.configuration_ms,
                "created_at": b.created_at,
                "tasks": {
                    "total": b.tasks,
                    "success": b.success,
                    "up-to-date": b.up_to_date,
                    "from-cache": b.from_cache,
                    "failed": b.failed,
                    "skipped": b.skipped,
                },
            })
        })
        .collect();
    Ok(Json(json!(items)))
}

fn fmt_ms(ms: i64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

fn fmt_bytes(bytes: i64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < KIB * KIB {
        format!("{:.1} KiB", b / KIB)
    } else if b < KIB * KIB * KIB {
        format!("{:.1} MiB", b / KIB / KIB)
    } else {
        format!("{:.2} GiB", b / KIB / KIB / KIB)
    }
}

/// from-cache share of tasks that needed work; "—" when none did.
fn hit_rate_cell(from_cache: i64, work: i64) -> String {
    if work == 0 {
        "—".to_string()
    } else {
        format!("{}%", from_cache * 100 / work)
    }
}

fn avoided_cell(b: &BuildRow) -> String {
    if b.tasks == 0 {
        return "—".to_string();
    }
    format!(
        "{}/{} ({}%)",
        b.avoided(),
        b.tasks,
        b.avoided() * 100 / b.tasks
    )
}

fn short_sha(sha: Option<&str>) -> String {
    match sha {
        Some(s) => esc(&s[..s.len().min(9)]),
        None => "—".to_string(),
    }
}

pub async fn builds_page(State(app): State<Arc<App>>) -> Result<Html<String>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let builds = db::builds(&conn, BUILDS_LIMIT).map_err(internal)?;
    if builds.is_empty() {
        return Ok(page("builds", "<p>No builds recorded yet.</p>"));
    }
    let rows: String = builds
        .iter()
        .map(|b| {
            format!(
                r#"<tr><td class="muted">{time}</td><td>{branch}</td><td><code>{sha}</code></td>
<td class="outcome {outcome}">{outcome}</td><td><a href="/builds/{id}"><code>{tasks}</code></a></td>
<td class="num">{duration}</td><td class="num">{avoided}</td></tr>"#,
                time = esc(&b.created_at),
                branch = esc(b.branch.as_deref().unwrap_or("—")),
                sha = short_sha(b.sha.as_deref()),
                outcome = esc(&b.outcome),
                id = b.id,
                tasks = esc(if b.requested_tasks.is_empty() {
                    "(default)"
                } else {
                    &b.requested_tasks
                }),
                duration = fmt_ms(b.total_ms),
                avoided = avoided_cell(b),
            )
        })
        .collect();
    let body = format!(
        "<table><tr><th>Time (UTC)</th><th>Branch</th><th>SHA</th><th>Outcome</th>\
<th>Tasks</th><th class=\"num\">Duration</th><th class=\"num\">Avoided</th></tr>{rows}</table>"
    );
    Ok(page("builds", &body))
}

pub async fn build_page(
    State(app): State<Arc<App>>,
    Path(id): Path<i64>,
) -> Result<Html<String>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let b = db::build(&conn, id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let tasks = db::build_tasks(&conn, id, SLOWEST_TASKS).map_err(internal)?;
    let ci = b
        .ci_url
        .as_deref()
        .map(|u| format!(r#" · <a href="{0}">CI run</a>"#, esc(u)))
        .unwrap_or_default();
    let versions = format!(
        "Gradle {} · JDK {}",
        esc(b.gradle_version.as_deref().unwrap_or("?")),
        esc(b.java_version.as_deref().unwrap_or("?")),
    );
    let slowest: String = tasks
        .iter()
        .map(|t| {
            format!(
                r#"<tr><td><code>{path}</code></td><td>{outcome}</td><td class="num">{duration}</td></tr>"#,
                path = esc(&t.path),
                outcome = esc(&t.outcome),
                duration = fmt_ms(t.duration_ms),
            )
        })
        .collect();
    let slowest = if slowest.is_empty() {
        "<p>No tasks executed.</p>".to_string()
    } else {
        format!(
            "<h2>Slowest tasks (top {SLOWEST_TASKS})</h2>\
<table><tr><th>Task</th><th>Outcome</th><th class=\"num\">Duration</th></tr>{slowest}</table>"
        )
    };
    let body = format!(
        r#"<p><code>{sha}</code> on <b>{branch}</b> · {time} UTC{ci}</p>
<p class="muted">{versions} · build key <code>{key}</code></p>
<p>Outcome: <span class="outcome {outcome}">{outcome}</span> · tasks <code>{tasks}</code></p>
<p>Total {total} · configuration {config} · execution {exec}</p>
<p>{success} success · {up_to_date} up-to-date · {from_cache} from-cache · {failed} failed · {skipped} skipped · avoided {avoided} · cache hit rate {hit_rate}</p>
{slowest}"#,
        sha = short_sha(b.sha.as_deref()),
        branch = esc(b.branch.as_deref().unwrap_or("—")),
        time = esc(&b.created_at),
        versions = versions,
        key = esc(&b.build_key),
        outcome = esc(&b.outcome),
        tasks = esc(if b.requested_tasks.is_empty() {
            "(default)"
        } else {
            &b.requested_tasks
        }),
        total = fmt_ms(b.total_ms),
        config = fmt_ms(b.configuration_ms),
        exec = fmt_ms((b.total_ms - b.configuration_ms).max(0)),
        success = b.success,
        up_to_date = b.up_to_date,
        from_cache = b.from_cache,
        failed = b.failed,
        skipped = b.skipped,
        avoided = avoided_cell(&b),
        hit_rate = hit_rate_cell(b.from_cache, b.from_cache + b.success + b.failed),
    );
    Ok(page(&format!("build #{id}"), &body))
}

const TOP_ARTIFACTS: usize = 20;
const NEVER_CACHED_WINDOW: usize = 100;
const NEVER_CACHED_MIN_EXECUTIONS: i64 = 3;
const NEVER_CACHED_LIMIT: usize = 50;

pub async fn cache_page(State(app): State<Arc<App>>) -> Result<Html<String>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let (entries, total) = db::cache_totals(&conn).map_err(internal)?;
    let top = db::cache_top_entries(&conn, TOP_ARTIFACTS).map_err(internal)?;
    let (from_cache, work) = db::cache_hit_totals(&conn).map_err(internal)?;
    let never = db::never_cached_tasks(
        &conn,
        NEVER_CACHED_WINDOW,
        NEVER_CACHED_MIN_EXECUTIONS,
        NEVER_CACHED_LIMIT,
    )
    .map_err(internal)?;
    drop(conn);
    let auth = if app.cache.token.is_some() {
        " · writes token-protected"
    } else {
        " · writes open"
    };
    let storage = format!(
        "<p>{entries} entries · {used} used of {cap} · artifact limit {alim} · \
retention {days} days{auth}</p>",
        used = fmt_bytes(total),
        cap = fmt_bytes(app.cache.max_total_bytes),
        alim = fmt_bytes(app.cache.max_artifact_bytes as i64),
        days = app.cache.retention_days,
    );
    let hit_rate = if work == 0 {
        "<p>No task telemetry yet — overall hit rate unavailable.</p>".to_string()
    } else {
        format!(
            "<p>Overall task cache hit rate: <span class=\"score\">{rate}</span> \
({from_cache} from-cache of {work} tasks that needed work, all retained builds).
<span class=\"muted\">The denominator includes non-cacheable tasks, so the achievable rate is higher.</span></p>",
            rate = hit_rate_cell(from_cache, work),
        )
    };
    let artifacts = if top.is_empty() {
        "<p>Cache is empty.</p>".to_string()
    } else {
        let rows: String = top
            .iter()
            .map(|e| {
                format!(
                    r#"<tr><td><code>{key}</code></td><td class="num">{size}</td><td class="num">{hits}</td>
<td class="muted">{created}</td><td class="muted">{accessed}</td></tr>"#,
                    key = esc(&e.key),
                    size = fmt_bytes(e.size),
                    hits = e.hit_count,
                    created = esc(&e.created_at),
                    accessed = esc(&e.last_accessed_at),
                )
            })
            .collect();
        format!(
            "<h2>Top artifacts by hits (top {TOP_ARTIFACTS})</h2>\
<table><tr><th>Key</th><th class=\"num\">Size</th><th class=\"num\">Hits</th>\
<th>Created (UTC)</th><th>Last accessed (UTC)</th></tr>{rows}</table>"
        )
    };
    let never_cached = if never.is_empty() {
        "<p>No never-cached task paths detected.</p>".to_string()
    } else {
        let rows: String = never
            .iter()
            .map(|t| {
                format!(
                    r#"<tr><td><code>{path}</code></td><td class="num">{execs}</td><td class="num">{total}</td></tr>"#,
                    path = esc(&t.path),
                    execs = t.executions,
                    total = fmt_ms(t.total_ms),
                )
            })
            .collect();
        format!(
            "<table><tr><th>Task path</th><th class=\"num\">Executions</th>\
<th class=\"num\">Total execution time</th></tr>{rows}</table>"
        )
    };
    let body = format!(
        "{storage}{hit_rate}{artifacts}\
<h2>Never-cached task paths</h2>\
<p class=\"muted\">Task paths executed in at least {NEVER_CACHED_MIN_EXECUTIONS} of the last \
{NEVER_CACHED_WINDOW} builds and never from-cache or up-to-date, by total execution time. \
Weak signal: telemetry has no machine identity, so tasks whose inputs change every build are \
indistinguishable from uncacheable tasks — a starting point for investigation, not a verdict.</p>\
{never_cached}"
    );
    Ok(page("build cache", &body))
}

fn median(mut values: Vec<i64>) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[values.len() / 2])
}

pub async fn trends_page(State(app): State<Arc<App>>) -> Result<Html<String>, StatusCode> {
    let conn = app.db.lock().unwrap();
    let builds = db::builds(&conn, TREND_FETCH).map_err(internal)?;
    if builds.is_empty() {
        return Ok(page("trends", "<p>No builds recorded yet.</p>"));
    }
    // group newest-first builds per branch, keeping the recent window
    let mut branches: Vec<(String, Vec<&BuildRow>)> = Vec::new();
    for b in &builds {
        let name = b.branch.as_deref().unwrap_or("(unknown)").to_string();
        match branches.iter_mut().find(|(n, _)| *n == name) {
            Some((_, list)) if list.len() < TREND_WINDOW => list.push(b),
            Some(_) => {}
            None => branches.push((name, vec![b])),
        }
    }
    struct Trend {
        branch: String,
        count: usize,
        median_ms: Option<i64>,
        median_avoided_pct: Option<i64>,
    }
    let trends: Vec<Trend> = branches
        .iter()
        .map(|(name, list)| {
            let durations: Vec<i64> = list
                .iter()
                .filter(|b| b.outcome == "success")
                .map(|b| b.total_ms)
                .collect();
            let avoided: Vec<i64> = list
                .iter()
                .filter(|b| b.tasks > 0)
                .map(|b| b.avoided() * 100 / b.tasks)
                .collect();
            Trend {
                branch: name.clone(),
                count: list.len(),
                median_ms: median(durations),
                median_avoided_pct: median(avoided),
            }
        })
        .collect();
    let max_ms = trends.iter().filter_map(|t| t.median_ms).max().unwrap_or(0);
    let rows: String = trends
        .iter()
        .map(|t| {
            let (duration, bar) = match t.median_ms {
                Some(ms) => (
                    fmt_ms(ms),
                    format!(
                        r#"<span class="bar" style="width:{}px"></span>"#,
                        (ms * 200 / max_ms.max(1)).max(2)
                    ),
                ),
                None => ("—".to_string(), String::new()),
            };
            let avoided = t
                .median_avoided_pct
                .map(|p| format!("{p}%"))
                .unwrap_or_else(|| "—".to_string());
            format!(
                r#"<tr><td>{branch}</td><td class="num">{count}</td><td class="num">{duration}</td>
<td>{bar}</td><td class="num">{avoided}</td></tr>"#,
                branch = esc(&t.branch),
                count = t.count,
            )
        })
        .collect();
    let body = format!(
        "<p class=\"muted\">Per branch over its last {TREND_WINDOW} builds; median duration counts successful builds only.</p>\
<table><tr><th>Branch</th><th class=\"num\">Builds</th><th class=\"num\">Median duration</th>\
<th></th><th class=\"num\">Median avoided</th></tr>{rows}</table>"
    );
    Ok(page("build trends", &body))
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
