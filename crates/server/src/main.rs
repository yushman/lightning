mod db;
mod score;
mod web;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
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
}

pub struct App {
    pub db: Mutex<Connection>,
    pub retention_days: u32,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let conn = db::open(&args.db)
        .unwrap_or_else(|e| panic!("cannot open database {}: {e}", args.db.display()));
    db::prune(&conn, args.retention_days).expect("retention pruning failed");
    let app = Arc::new(App {
        db: Mutex::new(conn),
        retention_days: args.retention_days,
    });
    let router = Router::new()
        .route("/", get(web::flaky_page))
        .route("/tests/{id}", get(web::test_page))
        .route("/runs/{id}", get(web::run_page))
        .route("/builds", get(web::builds_page))
        .route("/builds/{id}", get(web::build_page))
        .route("/trends", get(web::trends_page))
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
