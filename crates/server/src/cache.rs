use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use rusqlite::Connection;

use crate::App;
use crate::db;
use crate::web::internal;

pub struct CacheConfig {
    pub dir: PathBuf,
    pub max_artifact_bytes: u64,
    pub max_total_bytes: i64,
    pub retention_days: u32,
    /// Shared token protecting writes (HTTP Basic password); None = open.
    pub token: Option<String>,
}

/// Gradle build cache keys: lowercase hex, 32–64 chars (Gradle 9 emits 32;
/// the range leaves room for wider hashes). Valid keys are safe filenames.
pub fn valid_key(key: &str) -> bool {
    (32..=64).contains(&key.len()) && key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Creates the cache directory and reconciles index and filesystem in both
/// directions: index rows without a file are dropped, files without an index
/// row (including leftover temp files) are deleted. The index is authoritative.
pub fn reconcile(conn: &Connection, dir: &Path) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| format!("cache dir {}: {e}", dir.display());
    std::fs::create_dir_all(dir).map_err(|e| err(&e))?;
    let mut on_disk = std::collections::HashSet::new();
    for entry in std::fs::read_dir(dir).map_err(|e| err(&e))? {
        let entry = entry.map_err(|e| err(&e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|t| t.is_file()) && valid_key(&name) {
            on_disk.insert(name);
        } else {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    for key in db::cache_keys(conn).map_err(|e| err(&e))? {
        if !on_disk.remove(&key) {
            db::cache_remove(conn, &key).map_err(|e| err(&e))?;
        }
    }
    for orphan in on_disk {
        let _ = std::fs::remove_file(dir.join(orphan));
    }
    Ok(())
}

fn basic_auth_ok(token: Option<&str>, headers: &HeaderMap) -> bool {
    let Some(token) = token else { return true };
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(encoded) = value.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let Ok(decoded) = String::from_utf8(decoded) else {
        return false;
    };
    // Gradle sends "username:password"; the username is ignored.
    decoded
        .split_once(':')
        .is_some_and(|(_, password)| password == token)
}

/// GET (and HEAD, which axum routes here too) /cache/{key}.
pub async fn get_entry(
    State(app): State<Arc<App>>,
    method: Method,
    UrlPath(key): UrlPath<String>,
) -> Result<Response, StatusCode> {
    if !valid_key(&key) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let indexed = {
        let conn = app.db.lock().unwrap();
        db::cache_touch(&conn, &key, method == Method::GET).map_err(internal)?
    };
    if !indexed {
        return Err(StatusCode::NOT_FOUND);
    }
    match tokio::fs::read(app.cache.dir.join(&key)).await {
        Ok(bytes) => Ok((
            [(header::CONTENT_TYPE, "application/octet-stream")],
            Bytes::from(bytes),
        )
            .into_response()),
        Err(_) => {
            // indexed but the file vanished: degrade to a miss
            let conn = app.db.lock().unwrap();
            db::cache_remove(&conn, &key).map_err(internal)?;
            Err(StatusCode::NOT_FOUND)
        }
    }
}

/// PUT /cache/{key}: opaque binary body, stored atomically (temp + rename).
pub async fn put_entry(
    State(app): State<Arc<App>>,
    UrlPath(key): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !valid_key(&key) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if !basic_auth_ok(app.cache.token.as_deref(), &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, r#"Basic realm="lightning""#)],
        )
            .into_response();
    }
    if body.len() as u64 > app.cache.max_artifact_bytes {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = app.cache.dir.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let stored = async {
        tokio::fs::write(&tmp, &body).await?;
        tokio::fs::rename(&tmp, app.cache.dir.join(&key)).await
    }
    .await;
    if let Err(e) = stored {
        eprintln!("cache write failed: {e}");
        let _ = tokio::fs::remove_file(&tmp).await;
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let evicted = {
        let conn = app.db.lock().unwrap();
        db::cache_upsert(&conn, &key, body.len() as i64)
            .and_then(|_| db::cache_prune_expired(&conn, app.cache.retention_days))
            .and_then(|mut keys| {
                keys.extend(db::cache_evict_lru(&conn, app.cache.max_total_bytes, &key)?);
                Ok(keys)
            })
    };
    match evicted {
        Ok(keys) => {
            for k in keys {
                let _ = tokio::fs::remove_file(app.cache.dir.join(&k)).await;
            }
            StatusCode::CREATED.into_response()
        }
        Err(e) => internal(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_key_accepts_gradle_hashes_only() {
        assert!(valid_key(&"a1".repeat(16))); // 32 hex (Gradle 9)
        assert!(valid_key(&"0f".repeat(32))); // 64 hex
        assert!(!valid_key(&"a1".repeat(15))); // too short
        assert!(!valid_key(&"a1".repeat(33))); // too long
        assert!(!valid_key(&"A1".repeat(16))); // uppercase
        assert!(!valid_key(&"g1".repeat(16))); // non-hex
        assert!(!valid_key("../../../../../../etc/passwd"));
        assert!(!valid_key(""));
    }

    fn headers_with_basic(user: &str, password: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
        headers.insert(
            header::AUTHORIZATION,
            format!("Basic {encoded}").parse().unwrap(),
        );
        headers
    }

    #[test]
    fn basic_auth_matches_password_ignores_username() {
        assert!(basic_auth_ok(None, &HeaderMap::new()));
        assert!(!basic_auth_ok(Some("secret"), &HeaderMap::new()));
        assert!(basic_auth_ok(
            Some("secret"),
            &headers_with_basic("anything", "secret")
        ));
        assert!(!basic_auth_ok(
            Some("secret"),
            &headers_with_basic("secret", "wrong")
        ));
        let mut bearer = HeaderMap::new();
        bearer.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(!basic_auth_ok(Some("secret"), &bearer));
    }

    #[test]
    fn reconcile_drops_orphans_on_both_sides() {
        let dir = std::env::temp_dir().join(format!("lightning-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        let kept = "ab".repeat(16);
        let indexed_only = "cd".repeat(16);
        let file_only = "ef".repeat(16);
        db::cache_upsert(&conn, &kept, 4).unwrap();
        db::cache_upsert(&conn, &indexed_only, 4).unwrap();
        reconcile(&conn, &dir).unwrap(); // also creates the dir
        std::fs::write(dir.join(&kept), b"data").unwrap();
        std::fs::write(dir.join(&file_only), b"data").unwrap();
        std::fs::write(dir.join(".tmp-1-1"), b"partial").unwrap();
        db::cache_upsert(&conn, &kept, 4).unwrap(); // re-index after first pass
        reconcile(&conn, &dir).unwrap();
        assert_eq!(db::cache_keys(&conn).unwrap(), vec![kept.clone()]);
        assert!(dir.join(&kept).exists());
        assert!(!dir.join(&file_only).exists());
        assert!(!dir.join(".tmp-1-1").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
