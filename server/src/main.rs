mod align;
mod anchor;
mod api;
mod db;
mod diff;
mod doc;
mod domain;
mod error;
mod mcp;
mod packet_md;
mod share;
mod store;

#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod pipeline_tests;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use rust_embed::RustEmbed;

use api::{AppState, Config, Shared};
use store::Store;

#[derive(RustEmbed)]
#[folder = "../web/dist"]
struct WebDist;

const REPORT_CSS: &str = include_str!("../../web/src/styles/report.css");
const SHARE_CSS: &str = include_str!("../static/share.css");

pub fn app(state: Shared) -> Router {
    Router::new()
        .merge(api::router())
        .route("/mcp", get(mcp::get).post(mcp::post))
        .route("/s/{token}", get(share::page))
        .route("/a/{report}/{name}", get(asset))
        .route("/static/report.css", get(|| async { css(REPORT_CSS) }))
        .route("/static/share.css", get(|| async { css(SHARE_CSS) }))
        .route("/", get(|| async { Redirect::temporary("/app") }))
        .route("/app", get(spa_index))
        .route("/app/", get(spa_index))
        .route("/app/{*path}", get(spa))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state)
}

fn css(body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=300")], body)
        .into_response()
}

async fn asset(State(s): State<Shared>, Path((report, name)): Path<(String, String)>) -> Response {
    match s.store().asset(&report, &name) {
        Ok((mime, bytes)) => {
            let mut resp = bytes.into_response();
            let h = resp.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&mime).unwrap_or(HeaderValue::from_static("application/octet-stream")));
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
            h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
            h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; sandbox"));
            resp
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn embedded(path: &str) -> Option<Response> {
    let file = WebDist::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    Some(
        (
            [
                (header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap()),
                (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
            ],
            file.data.into_owned(),
        )
            .into_response(),
    )
}

async fn spa_index() -> Response {
    embedded("index.html").unwrap_or_else(|| {
        (StatusCode::SERVICE_UNAVAILABLE, "workbench not built: run `npm run build` in web/").into_response()
    })
}

async fn spa(Path(path): Path<String>) -> Response {
    match embedded(&path) {
        Some(r) => r,
        None if path.starts_with("assets/") => StatusCode::NOT_FOUND.into_response(),
        None => spa_index().await,
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

/// Missing secrets are generated once and kept in `<data>/secrets.json` so first runs need no setup.
fn load_secret(data_dir: &std::path::Path, key: &str, env: &str) -> String {
    if let Ok(v) = std::env::var(env) {
        if !v.is_empty() {
            return v;
        }
    }
    let path = data_dir.join("secrets.json");
    let mut secrets: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if let Some(v) = secrets.get(key).and_then(|v| v.as_str()) {
        return v.to_string();
    }
    let value = db::random_token();
    secrets.insert(key.to_string(), value.clone().into());
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&secrets).unwrap_or_default());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    value
}

#[tokio::main]
async fn main() {
    let addr: SocketAddr = env_or("GALLEY_ADDR", "127.0.0.1:7860").parse().expect("GALLEY_ADDR must be host:port");
    let data_dir = PathBuf::from(env_or("GALLEY_DATA", "./data"));
    std::fs::create_dir_all(data_dir.join("assets")).expect("cannot create data directory");

    let config = Config {
        owner_password: load_secret(&data_dir, "owner_password", "GALLEY_PASSWORD"),
        agent_token: load_secret(&data_dir, "agent_token", "GALLEY_AGENT_TOKEN"),
        public_url: env_or("GALLEY_PUBLIC_URL", &format!("http://{addr}")),
        secure_cookies: env_or("GALLEY_PUBLIC_URL", "").starts_with("https://"),
    };
    let conn = db::open(&data_dir.join("galley.db")).expect("cannot open database");
    let state = Arc::new(AppState {
        store: Mutex::new(Store::new(conn, data_dir.join("assets"))),
        config: config.clone(),
    });

    let listener = tokio::net::TcpListener::bind(addr).await.expect("cannot bind");
    println!("galley listening on {}", config.public_url);
    println!("  workbench: {}/app", config.public_url);
    println!("  MCP:       {}/mcp", config.public_url);
    if std::env::var("GALLEY_PASSWORD").is_err() || std::env::var("GALLEY_AGENT_TOKEN").is_err() {
        println!("  secrets:   {} (owner_password, agent_token)", data_dir.join("secrets.json").display());
    }
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}
