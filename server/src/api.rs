//! HTTP API for the workbench (owner) and agents.

use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::Role;
use crate::error::{AppError, AppResult};
use crate::packet_md;
use crate::store::{CommentPatch, NewComment, ResultReq, Store};

pub const SESSION_COOKIE: &str = "galley_session";

#[derive(Clone)]
pub struct Config {
    pub owner_password: String,
    pub agent_token: String,
    pub public_url: String,
    /// Adds `Secure` to the session cookie when served over HTTPS.
    pub secure_cookies: bool,
}

pub struct AppState {
    pub store: Mutex<Store>,
    pub config: Config,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ---------- auth ----------

pub struct Auth(pub Role);
pub struct Owner;

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

impl FromRequestParts<Shared> for Auth {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &Shared) -> Result<Self, Self::Rejection> {
        if let Some(h) = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
            if let Some(token) = h.strip_prefix("Bearer ") {
                if !state.config.agent_token.is_empty() && constant_time_eq(token.trim(), &state.config.agent_token) {
                    return Ok(Auth(Role::Agent));
                }
            }
            return Err(AppError::Unauthorized);
        }
        if let Some(tok) = cookie_value(&parts.headers, SESSION_COOKIE) {
            if state.store().session_valid(&tok)? {
                return Ok(Auth(Role::Owner));
            }
        }
        Err(AppError::Unauthorized)
    }
}

impl FromRequestParts<Shared> for Owner {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &Shared) -> Result<Self, Self::Rejection> {
        match Auth::from_request_parts(parts, state).await?.0 {
            Role::Owner => Ok(Owner),
            Role::Agent => Err(AppError::Forbidden("this action is reserved for the owner".into())),
        }
    }
}

// ---------- routes ----------

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/meta", get(meta))
        .route("/api/pending", get(pending))
        .route("/api/reports", get(list_reports).post(create_report))
        .route("/api/reports/{id}", get(get_report))
        .route("/api/reports/{id}/source", get(get_source))
        .route("/api/reports/{id}/versions", get(list_versions).post(push_version))
        .route("/api/reports/{id}/assets", post(upload_asset))
        .route("/api/reports/{id}/comments", get(list_comments).post(create_comment))
        .route("/api/reports/{id}/rounds", get(list_rounds).post(submit_round))
        .route("/api/reports/{id}/rounds/current", get(current_round))
        .route("/api/reports/{id}/compare", get(compare))
        .route("/api/reports/{id}/rollback", post(rollback))
        .route("/api/reports/{id}/publications", get(list_publications).post(publish))
        .route("/api/publications/{id}/revoke", post(revoke))
        .route("/api/versions/{id}", get(get_version))
        .route("/api/comments/{id}", patch(update_comment).delete(delete_comment))
        .route("/api/comments/{id}/messages", post(add_message))
        .route("/api/comments/{id}/resolve", post(resolve))
        .route("/api/comments/{id}/reopen", post(reopen))
        .route("/api/rounds/{id}", get(get_round))
        .route("/api/rounds/{id}/packet", get(get_packet))
        .route("/api/rounds/{id}/claim", post(claim))
        .route("/api/rounds/{id}/result", post(submit_result))
        .route("/api/rounds/{id}/review", get(review))
        .route("/api/rounds/{id}/extra/confirm", post(confirm_extra))
}

#[derive(Deserialize)]
struct LoginReq {
    password: String,
}

async fn login(State(s): State<Shared>, Json(req): Json<LoginReq>) -> AppResult<Response> {
    if s.config.owner_password.is_empty() || !constant_time_eq(&req.password, &s.config.owner_password) {
        return Err(AppError::Unauthorized);
    }
    let token = s.store().create_session()?;
    let secure = if s.config.secure_cookies { "; Secure" } else { "" };
    let cookie = format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000{secure}");
    let mut resp = Json(json!({ "ok": true })).into_response();
    resp.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    Ok(resp)
}

async fn logout(State(s): State<Shared>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(tok) = cookie_value(&headers, SESSION_COOKIE) {
        s.store().delete_session(&tok)?;
    }
    let mut resp = Json(json!({ "ok": true })).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("galley_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    Ok(resp)
}

async fn me(Auth(role): Auth) -> Json<Value> {
    Json(json!({ "role": role }))
}

async fn meta(State(s): State<Shared>, _: Owner) -> Json<Value> {
    Json(json!({ "public_url": s.config.public_url }))
}

async fn pending(State(s): State<Shared>, _: Auth) -> AppResult<Json<Value>> {
    let rows = s.store().pending_rounds()?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|(r, round)| json!({
            "report_id": r.id, "title": r.title, "round_id": round.id, "round_seq": round.seq,
            "status": round.status, "comments": round.comment_count,
        }))
        .collect::<Vec<_>>())))
}

async fn list_reports(State(s): State<Shared>, _: Auth) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().list_reports()?)))
}

#[derive(Deserialize)]
struct MarkdownReq {
    markdown: String,
    #[serde(default)]
    note: Option<String>,
}

async fn create_report(State(s): State<Shared>, _: Auth, Json(req): Json<MarkdownReq>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().create_report(&req.markdown)?)))
}

async fn get_report(State(s): State<Shared>, _: Auth, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let store = s.store();
    let info = store.report_info(&id)?;
    let v = store.current_version(&id)?;
    Ok(Json(json!({ "report": info, "version": v })))
}

#[derive(Deserialize)]
struct FormatQ {
    format: Option<String>,
}

async fn get_source(
    State(s): State<Shared>,
    _: Auth,
    Path(id): Path<String>,
    Query(q): Query<FormatQ>,
) -> AppResult<Response> {
    let text = s.store().source(&id, q.format.as_deref() != Some("raw"))?;
    Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], text).into_response())
}

async fn list_versions(State(s): State<Shared>, _: Auth, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().versions(&id)?)))
}

async fn push_version(
    State(s): State<Shared>,
    _: Auth,
    Path(id): Path<String>,
    Json(req): Json<MarkdownReq>,
) -> AppResult<Json<Value>> {
    let v = s.store().push_version(&id, &req.markdown, req.note.as_deref().unwrap_or("推送新版本"))?;
    Ok(Json(json!({ "id": v.id, "seq": v.seq, "diff": v.diff })))
}

#[derive(Deserialize)]
struct NameQ {
    name: String,
}

async fn upload_asset(
    State(s): State<Shared>,
    _: Auth,
    Path(id): Path<String>,
    Query(q): Query<NameQ>,
    body: Bytes,
) -> AppResult<Json<Value>> {
    let path = s.store().add_asset(&id, &q.name, &body)?;
    Ok(Json(json!({ "path": path })))
}

#[derive(Deserialize)]
struct VersionQ {
    version: Option<String>,
}

async fn list_comments(
    State(s): State<Shared>,
    _: Owner,
    Path(id): Path<String>,
    Query(q): Query<VersionQ>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().comments(&id, q.version.as_deref())?)))
}

async fn create_comment(
    State(s): State<Shared>,
    _: Owner,
    Path(id): Path<String>,
    Json(req): Json<NewComment>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().create_comment(&id, req)?)))
}

async fn update_comment(
    State(s): State<Shared>,
    _: Owner,
    Path(id): Path<String>,
    Json(req): Json<CommentPatch>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().update_comment(&id, req)?)))
}

async fn delete_comment(State(s): State<Shared>, _: Owner, Path(id): Path<String>) -> AppResult<StatusCode> {
    s.store().delete_comment(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct BodyReq {
    #[serde(default)]
    body: Option<String>,
}

async fn add_message(
    State(s): State<Shared>,
    _: Owner,
    Path(id): Path<String>,
    Json(req): Json<BodyReq>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().owner_message(&id, req.body.as_deref().unwrap_or(""))?)))
}

async fn resolve(State(s): State<Shared>, Auth(role): Auth, Path(id): Path<String>) -> AppResult<Json<Value>> {
    if role != Role::Owner {
        return Err(AppError::Forbidden("only the owner can resolve comments".into()));
    }
    Ok(Json(json!(s.store().resolve(&id)?)))
}

async fn reopen(
    State(s): State<Shared>,
    Auth(role): Auth,
    Path(id): Path<String>,
    body: Option<Json<BodyReq>>,
) -> AppResult<Json<Value>> {
    if role != Role::Owner {
        return Err(AppError::Forbidden("only the owner can reopen comments".into()));
    }
    let body = body.and_then(|Json(b)| b.body);
    Ok(Json(json!(s.store().reopen(&id, body.as_deref())?)))
}

async fn list_rounds(State(s): State<Shared>, _: Auth, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().rounds(&id)?)))
}

async fn submit_round(State(s): State<Shared>, _: Owner, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().submit_round(&id)?)))
}

fn packet_response(s: &Shared, round_id: &str, format: Option<&str>) -> AppResult<Response> {
    let packet = s.store().packet(round_id)?;
    if format == Some("md") {
        return Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], packet_md::render(&packet)).into_response());
    }
    Ok(Json(json!(packet)).into_response())
}

async fn current_round(
    State(s): State<Shared>,
    _: Auth,
    Path(id): Path<String>,
    Query(q): Query<FormatQ>,
) -> AppResult<Response> {
    let round = s.store().active_round(&id)?.ok_or(AppError::NotFound)?;
    packet_response(&s, &round.id, q.format.as_deref())
}

async fn get_round(State(s): State<Shared>, _: Auth, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().round(&id)?)))
}

async fn get_packet(
    State(s): State<Shared>,
    _: Auth,
    Path(id): Path<String>,
    Query(q): Query<FormatQ>,
) -> AppResult<Response> {
    packet_response(&s, &id, q.format.as_deref())
}

async fn claim(State(s): State<Shared>, Auth(role): Auth, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().claim(&id, role)?)))
}

async fn submit_result(
    State(s): State<Shared>,
    Auth(role): Auth,
    Path(id): Path<String>,
    Json(req): Json<ResultReq>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().submit_result(&id, role, req)?)))
}

async fn review(State(s): State<Shared>, _: Owner, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().review(&id)?)))
}

#[derive(Deserialize)]
struct ConfirmReq {
    #[serde(default)]
    block_id: Option<String>,
}

async fn confirm_extra(
    State(s): State<Shared>,
    _: Owner,
    Path(id): Path<String>,
    Json(req): Json<ConfirmReq>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().confirm_extra(&id, req.block_id.as_deref())?)))
}

#[derive(Deserialize)]
struct CompareQ {
    from: String,
    to: String,
}

async fn compare(
    State(s): State<Shared>,
    _: Owner,
    Path(_id): Path<String>,
    Query(q): Query<CompareQ>,
) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().compare(&q.from, &q.to)?)))
}

#[derive(Deserialize)]
struct VersionReq {
    #[serde(default)]
    version_id: Option<String>,
}

async fn rollback(
    State(s): State<Shared>,
    _: Owner,
    Path(id): Path<String>,
    Json(req): Json<VersionReq>,
) -> AppResult<Json<Value>> {
    let vid = req.version_id.ok_or_else(|| AppError::BadRequest("version_id is required".into()))?;
    let v = s.store().rollback(&id, &vid)?;
    Ok(Json(json!({ "id": v.id, "seq": v.seq })))
}

async fn get_version(State(s): State<Shared>, _: Owner, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().version(&id)?)))
}

async fn list_publications(State(s): State<Shared>, _: Owner, Path(id): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(json!(s.store().publications(&id)?)))
}

async fn publish(
    State(s): State<Shared>,
    _: Owner,
    Path(id): Path<String>,
    body: Option<Json<VersionReq>>,
) -> AppResult<Json<Value>> {
    let vid = body.and_then(|Json(b)| b.version_id);
    let mut store = s.store();
    let p = store.publish(&id, vid.as_deref())?;
    let counts = store.report_info(&id)?.counts;
    Ok(Json(json!({ "publication": p, "counts": counts })))
}

async fn revoke(State(s): State<Shared>, _: Owner, Path(id): Path<String>) -> AppResult<StatusCode> {
    s.store().revoke(&id)?;
    Ok(StatusCode::NO_CONTENT)
}
