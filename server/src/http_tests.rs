//! End-to-end tests through the HTTP router: a full round, permissions, and share-page privacy.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::api::{AppState, Config};
use crate::store::Store;

const AGENT: &str = "test-agent-token";
const PASSWORD: &str = "test-password";

struct T {
    app: axum::Router,
    cookie: String,
}

impl T {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("galley-test-{}", crate::db::random_token()));
        let state = Arc::new(AppState {
            store: Mutex::new(Store::new(crate::db::open_memory().unwrap(), dir)),
            config: Config {
                owner_password: PASSWORD.into(),
                agent_token: AGENT.into(),
                public_url: "http://test".into(),
                secure_cookies: false,
            },
        });
        let app = crate::app(state);
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "password": PASSWORD }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = resp.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string();
        T { app, cookie }
    }

    async fn req(&self, method: &str, path: &str, who: Who, body: Option<Value>) -> (StatusCode, Value, String) {
        let mut b = Request::builder().method(method).uri(path);
        match who {
            Who::Owner => b = b.header(header::COOKIE, &self.cookie),
            Who::Agent => b = b.header(header::AUTHORIZATION, format!("Bearer {AGENT}")),
            Who::Anon => {}
        }
        let body = match body {
            Some(v) => {
                b = b.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = self.app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let headers = format!("{:?}", resp.headers());
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes).to_string();
        let v = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, v, headers)
    }
}

#[derive(Clone, Copy)]
enum Who {
    Owner,
    Agent,
    Anon,
}

const V1: &str = "---\ntitle: 储能调研\nsummary: 摘要\n---\n\n## 一、市场\n\n2025 年装机 120 GW，同比增长 10%。\n\n## 二、风险\n\n原材料价格波动。\n";
const V2: &str = "---\ntitle: 储能调研\nsummary: 摘要\n---\n\n## 一、市场\n\n2025 年装机 120 GW，同比增长 12.3%（来源：BNEF）。\n\n## 二、风险\n\n原材料价格波动。另有新增一句。\n";

fn block_id(report: &Value, text: &str) -> String {
    report["version"]["doc"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["text"].as_str().unwrap().contains(text))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn full_round_and_privacy() {
    let t = T::new().await;

    let (st, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let rid = r["id"].as_str().unwrap().to_string();

    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let para = block_id(&report, "同比增长");
    let section = block_id(&report, "二、风险");

    // A text comment with a secret marker, a section comment, a document comment.
    let marker = "SECRET-MARKER-9f2c";
    let (st, c1, _) = t
        .req(
            "POST",
            &format!("/api/reports/{rid}/comments"),
            Who::Owner,
            // Older clients still send `kind`; unknown fields are ignored.
            Some(json!({ "kind": "verify", "body": format!("这个数字对吗 {marker}"),
                "anchor": { "type": "text", "block_id": para, "start": 21, "end": 24, "quote": "10%" } })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{c1}");
    let c1 = c1["id"].as_str().unwrap().to_string();
    let (_, c2, _) = t
        .req(
            "POST",
            &format!("/api/reports/{rid}/comments"),
            Who::Owner,
            Some(json!({ "body": "补充一点", "anchor": { "type": "section", "section_id": section } })),
        )
        .await;
    let c2 = c2["id"].as_str().unwrap().to_string();
    let (_, c3, _) = t
        .req(
            "POST",
            &format!("/api/reports/{rid}/comments"),
            Who::Owner,
            Some(json!({ "body": "全文语气更客观", "anchor": { "type": "document" } })),
        )
        .await;
    let c3 = c3["id"].as_str().unwrap().to_string();

    // Drafts are invisible to the agent.
    let (st, _, _) = t.req("GET", &format!("/api/reports/{rid}/rounds/current"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _, _) = t.req("GET", &format!("/api/reports/{rid}/comments"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Agent cannot submit a round; owner can.
    let (st, _, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::OK, "{round}");
    let round_id = round["id"].as_str().unwrap().to_string();

    let (st, md, _) = t.req("GET", &format!("/api/reports/{rid}/rounds/current?format=md"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::OK);
    let md = md.as_str().unwrap();
    assert!(md.contains(&c1) && md.contains("10%") && md.contains("源码第"), "{md}");
    // Reach is read from the comment's own wording, not from a separate field.
    assert!(md.contains("全文语气更客观") && md.contains("以评论原文为准"), "{md}");
    assert!(!md.contains("只改这一处") && !md.contains("核实 ·"), "{md}");

    let (st, _, _) = t.req("POST", &format!("/api/rounds/{round_id}/claim"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::OK);

    // Agent cannot resolve.
    let (st, _, _) = t.req("POST", &format!("/api/comments/{c1}/resolve"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Missing replies are rejected atomically.
    let (st, _, _) = t
        .req(
            "POST",
            &format!("/api/rounds/{round_id}/result"),
            Who::Agent,
            Some(json!({ "markdown": V2, "summary": "x", "replies": [
                { "comment_id": c1, "action": "changed", "body": "已改" } ] })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (_, versions, _) = t.req("GET", &format!("/api/reports/{rid}/versions"), Who::Owner, None).await;
    assert_eq!(versions.as_array().unwrap().len(), 1);

    let (st, res, _) = t
        .req(
            "POST",
            &format!("/api/rounds/{round_id}/result"),
            Who::Agent,
            Some(json!({ "markdown": V2, "summary": "处理 3 条", "replies": [
                { "comment_id": c1, "action": "changed", "body": "已改为 12.3%，来源 BNEF" },
                { "comment_id": c2, "action": "changed", "body": "已补充" },
                { "comment_id": c3, "action": "clarify", "body": "哪些段落语气不客观？" } ] })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{res}");
    assert_eq!(res["status"], "verifying");
    // Changes in the paragraph and the section are covered by comments.
    assert_eq!(res["extra_changes"].as_array().unwrap().len(), 0, "{res}");

    let (_, review, _) = t.req("GET", &format!("/api/rounds/{round_id}/review"), Who::Owner, None).await;
    let item = review["items"].as_array().unwrap().iter().find(|i| i["comment"]["id"] == c1.as_str()).unwrap();
    assert_eq!(item["change"]["op"], "modified");
    assert_eq!(item["comment"]["status"], "verify");
    assert!(item["comment"]["anchor"]["quote"].as_str().unwrap().contains("12.3%"));

    // Verify: resolve c1, reopen c2 → round done; c2 open for next round, c3 awaiting my reply.
    let (st, _, _) = t.req("POST", &format!("/api/comments/{c1}/resolve"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) =
        t.req("POST", &format!("/api/comments/{c2}/reopen"), Who::Owner, Some(json!({ "body": "还不够" }))).await;
    assert_eq!(st, StatusCode::OK);
    let (_, rd, _) = t.req("GET", &format!("/api/rounds/{round_id}"), Who::Owner, None).await;
    assert_eq!(rd["status"], "done");
    let (st, _, _) =
        t.req("POST", &format!("/api/comments/{c3}/messages"), Who::Owner, Some(json!({ "body": "第二节" }))).await;
    assert_eq!(st, StatusCode::OK);

    // Round 2 carries the reopened and answered comments.
    let (_, round2, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    assert_eq!(round2["seq"], 2);
    assert_eq!(round2["comment_count"], 2);
    let r2 = round2["id"].as_str().unwrap();
    let (st, _, _) = t
        .req(
            "POST",
            &format!("/api/rounds/{r2}/result"),
            Who::Agent,
            Some(json!({ "summary": "第二轮", "replies": [
                { "comment_id": c2, "action": "answered", "body": "补充了数据" },
                { "comment_id": c3, "action": "answered", "body": "已改第二节" } ] })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    for c in [&c2, &c3] {
        t.req("POST", &format!("/api/comments/{c}/resolve"), Who::Owner, None).await;
    }

    // Publish and check the share page contains no comment data.
    let (st, p, _) = t.req("POST", &format!("/api/reports/{rid}/publications"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::OK, "{p}");
    let path = p["publication"]["path"].as_str().unwrap().to_string();
    let pid = p["publication"]["id"].as_str().unwrap().to_string();
    let (st, html, headers) = t.req("GET", &path, Who::Anon, None).await;
    assert_eq!(st, StatusCode::OK);
    let html = html.as_str().unwrap();
    assert!(html.contains("12.3%"));
    for leaked in [marker, "已改为", "语气更客观", "<script", c1.as_str(), "data-block", "data-cell"] {
        assert!(!html.contains(leaked), "share page leaked {leaked}");
    }
    assert!(headers.contains("noindex"));
    assert!(headers.contains("script-src 'none'"));
    assert!(headers.contains("no-referrer"));

    // Republishing keeps the link; revoking returns 410; unknown tokens 404.
    let (_, p2, _) = t.req("POST", &format!("/api/reports/{rid}/publications"), Who::Owner, None).await;
    assert_eq!(p2["publication"]["path"].as_str().unwrap(), path);
    let (st, _, _) = t.req("POST", &format!("/api/publications/{pid}/revoke"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = t.req("GET", &path, Who::Anon, None).await;
    assert_eq!(st, StatusCode::GONE);
    let (st, _, _) = t.req("GET", "/s/doesnotexist", Who::Anon, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn auth_required() {
    let t = T::new().await;
    let (st, _, _) = t.req("GET", "/api/reports", Who::Anon, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = t
        .req("POST", "/api/login", Who::Anon, Some(json!({ "password": "wrong" })))
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_tools() {
    let t = T::new().await;
    let call = |id: i64, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let (st, init, _) = t.req("POST", "/mcp", Who::Agent, Some(call(1, "initialize", json!({ "protocolVersion": "2025-06-18" })))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(init["result"]["serverInfo"]["name"], "galley");
    let (st, _, _) = t.req("POST", "/mcp", Who::Agent, Some(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))).await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let (_, list, _) = t.req("POST", "/mcp", Who::Agent, Some(call(2, "tools/list", json!({})))).await;
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 6);
    let (_, created, _) = t
        .req("POST", "/mcp", Who::Agent, Some(call(3, "tools/call", json!({ "name": "galley_create_report", "arguments": { "markdown": V1 } }))))
        .await;
    let text = created["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("储能调研"), "{text}");
    let (_, round, _) = t
        .req("POST", "/mcp", Who::Agent, Some(call(4, "tools/call", json!({ "name": "galley_get_round", "arguments": {} }))))
        .await;
    assert!(round["result"]["content"][0]["text"].as_str().unwrap().contains("没有待处理"));
    let (st, _, _) = t.req("POST", "/mcp", Who::Owner, Some(call(5, "tools/list", json!({})))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}
