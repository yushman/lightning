mod cache;
mod db;
mod score;
mod web;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use clap::Parser;
use rusqlite::Connection;

#[derive(Parser)]
#[command(
    name = "lightning-server",
    version,
    about = "lightning flaky-test radar server"
)]
struct Args {
    /// Address to listen on
    #[arg(long, default_value = "127.0.0.1:8080", env = "LIGHTNING_ADDR")]
    addr: String,
    /// Path to the SQLite database file
    #[arg(long, default_value = "lightning.db", env = "LIGHTNING_DB")]
    db: PathBuf,
    /// Delete runs older than this many days
    #[arg(long, default_value_t = 90, env = "LIGHTNING_RETENTION_DAYS")]
    retention_days: u32,
    /// Directory for remote build cache artifacts (default: lightning-cache next to the db)
    #[arg(long, env = "LIGHTNING_CACHE_DIR")]
    cache_dir: Option<PathBuf>,
    /// Maximum size of one cache artifact, MiB
    #[arg(long, default_value_t = 100, env = "LIGHTNING_CACHE_MAX_ARTIFACT_MB")]
    cache_max_artifact_mb: u64,
    /// Maximum total cache size, MiB
    #[arg(long, default_value_t = 10240, env = "LIGHTNING_CACHE_MAX_SIZE_MB")]
    cache_max_size_mb: u64,
    /// Delete cache entries not accessed for this many days
    #[arg(long, default_value_t = 30, env = "LIGHTNING_CACHE_RETENTION_DAYS")]
    cache_retention_days: u32,
    /// Shared token protecting cache writes (HTTP Basic password); writes are open when unset
    #[arg(long, env = "LIGHTNING_CACHE_TOKEN")]
    cache_token: Option<String>,
}

pub struct App {
    pub db: Mutex<Connection>,
    pub retention_days: u32,
    pub cache: cache::CacheConfig,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let conn = db::open(&args.db)
        .unwrap_or_else(|e| panic!("cannot open database {}: {e}", args.db.display()));
    db::prune(&conn, args.retention_days).expect("retention pruning failed");
    let cache = cache::CacheConfig {
        dir: args.cache_dir.unwrap_or_else(|| {
            args.db
                .parent()
                .unwrap_or(Path::new(""))
                .join("lightning-cache")
        }),
        max_artifact_bytes: args.cache_max_artifact_mb * 1024 * 1024,
        max_total_bytes: (args.cache_max_size_mb * 1024 * 1024) as i64,
        retention_days: args.cache_retention_days,
        token: args.cache_token,
    };
    cache::reconcile(&conn, &cache.dir)
        .unwrap_or_else(|e| panic!("cache reconciliation failed: {e}"));
    for key in db::cache_prune_expired(&conn, cache.retention_days).expect("cache retention failed")
    {
        let _ = std::fs::remove_file(cache.dir.join(&key));
    }
    let body_limit = cache.max_artifact_bytes as usize;
    let app = Arc::new(App {
        db: Mutex::new(conn),
        retention_days: args.retention_days,
        cache,
    });
    let router = Router::new()
        .route("/", get(web::flaky_page))
        .route("/tests/{id}", get(web::test_page))
        .route("/runs/{id}", get(web::run_page))
        .route("/builds", get(web::builds_page))
        .route("/builds/{id}", get(web::build_page))
        .route("/trends", get(web::trends_page))
        .route("/cache", get(web::cache_page))
        .route(
            "/cache/{key}",
            get(cache::get_entry)
                .put(cache::put_entry)
                .layer(DefaultBodyLimit::max(body_limit)),
        )
        .route("/api/runs", post(web::ingest))
        .route("/api/flaky", get(web::flaky_api))
        .route("/api/builds", post(web::ingest_build).get(web::builds_api))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(&args.addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {}: {e}", args.addr));
    println!("lightning-server listening on http://{}", args.addr);
    axum::serve(listener, router).await.expect("server failed");
}
