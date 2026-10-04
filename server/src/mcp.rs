//! Minimal MCP server over Streamable HTTP (JSON responses, no SSE).
//!
//! Tools map one-to-one onto the agent HTTP API.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::api::{Auth, Shared};
use crate::domain::{Role, RoundStatus};
use crate::error::AppError;
use crate::packet_md;
use crate::store::ResultReq;

const PROTOCOL_VERSION: &str = "2025-06-18";

fn tools() -> Value {
    json!([
        {
            "name": "galley_list_reports",
            "description": "List Galley reports with their current version and any round waiting for the agent.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "galley_create_report",
            "description": "Create a new report from Markdown (GFM with `title`/`summary` frontmatter, `##` sections, no raw HTML, images as assets/<name>).",
            "inputSchema": {
                "type": "object",
                "properties": { "markdown": { "type": "string" } },
                "required": ["markdown"]
            }
        },
        {
            "name": "galley_get_source",
            "description": "Get the current Markdown source of a report. Use numbered=true to get line numbers matching the round packet.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "report_id": { "type": "string" },
                    "numbered": { "type": "boolean", "default": false }
                },
                "required": ["report_id"]
            }
        },
        {
            "name": "galley_get_round",
            "description": "Claim the pending review round and return its comment packet (Markdown). Without report_id, picks the oldest pending round across all reports.",
            "inputSchema": {
                "type": "object",
                "properties": { "report_id": { "type": "string" } }
            }
        },
        {
            "name": "galley_submit_round",
            "description": "Atomically submit a round result: the full new Markdown, one reply per pending comment, and a round summary. action: changed (I edited the report), answered (answer without edits), clarify (question or disagreement). The agent can never resolve comments.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "round_id": { "type": "string" },
                    "markdown": { "type": "string", "description": "Complete new Markdown. Omit if nothing changed." },
                    "replies": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "comment_id": { "type": "string" },
                                "action": { "type": "string", "enum": ["changed", "answered", "clarify"] },
                                "body": { "type": "string" }
                            },
                            "required": ["comment_id", "action", "body"]
                        }
                    },
                    "summary": { "type": "string", "description": "How many comments were handled, which sections changed, and any edits outside the comments." }
                },
                "required": ["round_id", "replies", "summary"]
            }
        },
        {
            "name": "galley_push_version",
            "description": "Push a new version of a report outside the review loop (not allowed while a round is pending).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "report_id": { "type": "string" },
                    "markdown": { "type": "string" },
                    "note": { "type": "string" }
                },
                "required": ["report_id", "markdown"]
            }
        }
    ])
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| AppError::BadRequest(format!("missing argument {key}")))
}

fn call_tool(s: &Shared, role: Role, name: &str, args: &Value) -> Result<String, AppError> {
    match name {
        "galley_list_reports" => {
            let reports = s.store().list_reports()?;
            let mut out = String::new();
            for r in reports {
                let round = match &r.active_round {
                    Some(rd) => format!("第 {} 轮 {}（round_id {}）", rd.seq, rd.status.as_str(), rd.id),
                    None => "无进行中的轮次".into(),
                };
                out.push_str(&format!("- {} `{}` v{} · {}\n", r.title, r.id, r.current_seq, round));
            }
            Ok(if out.is_empty() { "（还没有报告）".into() } else { out })
        }
        "galley_create_report" => {
            let r = s.store().create_report(arg(args, "markdown")?)?;
            Ok(format!("已创建报告「{}」，report_id: {}，版本 v{}", r.title, r.id, r.current_seq))
        }
        "galley_get_source" => {
            let numbered = args.get("numbered").and_then(Value::as_bool).unwrap_or(false);
            s.store().source(arg(args, "report_id")?, numbered)
        }
        "galley_get_round" => {
            let round = {
                let store = s.store();
                match args.get("report_id").and_then(Value::as_str) {
                    Some(id) => store.active_round(id)?,
                    None => store.pending_rounds()?.into_iter().map(|(_, r)| r).next(),
                }
            };
            let Some(round) = round else {
                return Ok("没有待处理的轮次。".into());
            };
            if round.status == RoundStatus::Submitted {
                s.store().claim(&round.id, role)?;
            } else if round.status != RoundStatus::Processing {
                return Ok(format!("第 {} 轮当前状态为 {}，无需处理。", round.seq, round.status.as_str()));
            }
            let packet = s.store().packet(&round.id)?;
            Ok(packet_md::render(&packet))
        }
        "galley_submit_round" => {
            let round_id = arg(args, "round_id")?.to_string();
            let req: ResultReq = serde_json::from_value(args.clone())
                .map_err(|e| AppError::BadRequest(format!("invalid arguments: {e}")))?;
            let r = s.store().submit_result(&round_id, role, req)?;
            Ok(format!(
                "已提交第 {} 轮结果，状态 {}；新版本 {}；评论之外的改动 {} 处。",
                r.seq,
                r.status.as_str(),
                r.result_version_id.as_deref().unwrap_or("-"),
                r.extra_changes.len()
            ))
        }
        "galley_push_version" => {
            let v = s.store().push_version(
                arg(args, "report_id")?,
                arg(args, "markdown")?,
                args.get("note").and_then(Value::as_str).unwrap_or("推送新版本"),
            )?;
            Ok(format!("已推送 v{}（{}），改动 {} 处", v.seq, v.id, v.diff.map_or(0, |d| d.changes.len())))
        }
        _ => Err(AppError::BadRequest(format!("unknown tool {name}"))),
    }
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn handle(s: &Shared, role: Role, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = id?; // notifications get no response
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
            rpc_result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "galley", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Galley 用于校对 AI 生成的长文报告。处理评论：galley_get_round 取评论包 → 修改 Markdown → galley_submit_round 一次性提交。"
                }),
            )
        }
        "ping" => rpc_result(&id, json!({})),
        "tools/list" => rpc_result(&id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(s, role, name, &args) {
                Ok(text) => rpc_result(&id, json!({ "content": [{ "type": "text", "text": text }], "isError": false })),
                Err(e) => rpc_result(&id, json!({ "content": [{ "type": "text", "text": e.client_message() }], "isError": true })),
            }
        }
        _ => rpc_error(&id, -32601, &format!("method not found: {method}")),
    })
}

pub async fn post(State(s): State<Shared>, Auth(role): Auth, Json(body): Json<Value>) -> Response {
    if role != Role::Agent {
        return AppError::Forbidden("MCP is for agents; use the agent token".into()).into_response();
    }
    let responses: Vec<Value> = match &body {
        Value::Array(batch) => batch.iter().filter_map(|m| handle(&s, role, m)).collect(),
        m => handle(&s, role, m).into_iter().collect(),
    };
    match (body.is_array(), responses.len()) {
        (_, 0) => StatusCode::ACCEPTED.into_response(),
        (false, _) => Json(responses.into_iter().next().unwrap()).into_response(),
        (true, _) => Json(Value::Array(responses)).into_response(),
    }
}

pub async fn get() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}
