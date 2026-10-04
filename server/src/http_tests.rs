//! End-to-end tests through the HTTP router: a full round, permissions, and share-page privacy.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::api::{AppState, Config, Shared};
use crate::store::Store;

const AGENT: &str = "test-agent-token";
const PASSWORD: &str = "test-password";

struct T {
    app: axum::Router,
    state: Shared,
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
        let app = crate::app(state.clone());
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
        T { app, state, cookie }
    }

    async fn upload(&self, rid: &str, name: &str, bytes: &'static [u8]) -> StatusCode {
        let req = Request::post(format!("/api/reports/{rid}/assets?name={name}"))
            .header(header::AUTHORIZATION, format!("Bearer {AGENT}"))
            .body(Body::from(bytes))
            .unwrap();
        self.app.clone().oneshot(req).await.unwrap().status()
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
    assert!(headers.contains("img-src 'self' data:"));
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

fn first_img_src(html: &str) -> String {
    html.split("<img src=\"").nth(1).unwrap().split('"').next().unwrap().to_string()
}

#[tokio::test]
async fn assets_are_private_and_published_snapshots_are_fixed() {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    assert_eq!(t.upload(&rid, "fig.png", b"IMAGE-ONE").await, StatusCode::OK);
    assert_eq!(t.upload(&rid, "other.png", b"UNPUBLISHED").await, StatusCode::OK);
    let md = format!("{V1}\n![图](assets/fig.png)\n");
    let (st, _, _) = t.req("POST", &format!("/api/reports/{rid}/versions"), Who::Agent, Some(json!({ "markdown": md }))).await;
    assert_eq!(st, StatusCode::OK);

    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let src = first_img_src(report["version"]["html"].as_str().unwrap());
    assert!(src.starts_with(&format!("/a/{rid}/")) && src.ends_with("/fig.png"), "{src}");

    // Workbench URLs need a login; content-addressed ones can be cached forever.
    let (st, _, _) = t.req("GET", &src, Who::Anon, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, body, headers) = t.req("GET", &src, Who::Owner, None).await;
    assert_eq!((st, body.as_str().unwrap()), (StatusCode::OK, "IMAGE-ONE"));
    assert!(headers.contains("immutable") && headers.contains("private"), "{headers}");

    let (_, p, _) = t.req("POST", &format!("/api/reports/{rid}/publications"), Who::Owner, None).await;
    let path = p["publication"]["path"].as_str().unwrap().to_string();
    let pid = p["publication"]["id"].as_str().unwrap().to_string();
    let (_, html, _) = t.req("GET", &path, Who::Anon, None).await;
    let html = html.as_str().unwrap();
    assert!(!html.contains(&rid), "share page leaked the report id");
    let shared = first_img_src(html);
    assert!(shared.starts_with(&format!("{path}/a/")), "{shared}");
    let (st, body, headers) = t.req("GET", &shared, Who::Anon, None).await;
    assert_eq!((st, body.as_str().unwrap()), (StatusCode::OK, "IMAGE-ONE"));
    assert!(headers.contains("no-cache") && !headers.contains("public"), "{headers}");

    // Only assets the published version references are reachable through the link.
    for other in [format!("{path}/a/other.png"), format!("{path}/a/fig.png")] {
        let (st, _, _) = t.req("GET", &other, Who::Anon, None).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{other}");
    }

    // Re-uploading under the same name does not change what was published.
    assert_eq!(t.upload(&rid, "fig.png", b"IMAGE-TWO").await, StatusCode::OK);
    let (_, body, _) = t.req("GET", &shared, Who::Anon, None).await;
    assert_eq!(body.as_str().unwrap(), "IMAGE-ONE");
    let (_, body, _) = t.req("GET", &format!("/a/{rid}/fig.png"), Who::Owner, None).await;
    assert_eq!(body.as_str().unwrap(), "IMAGE-TWO");

    // A new version picks up the new image; rolling back restores the old one.
    let (_, v, _) = t.req("POST", &format!("/api/reports/{rid}/versions"), Who::Agent, Some(json!({ "markdown": format!("{md}\n新增一段。\n") }))).await;
    let (_, cur, _) = t.req("GET", &format!("/api/versions/{}", v["id"].as_str().unwrap()), Who::Owner, None).await;
    let (_, body, _) = t.req("GET", &first_img_src(cur["html"].as_str().unwrap()), Who::Owner, None).await;
    assert_eq!(body.as_str().unwrap(), "IMAGE-TWO");
    let (_, versions, _) = t.req("GET", &format!("/api/reports/{rid}/versions"), Who::Owner, None).await;
    let v2 = versions.as_array().unwrap().iter().find(|v| v["seq"] == 2).unwrap()["id"].as_str().unwrap().to_string();
    let (st, rb, _) = t.req("POST", &format!("/api/reports/{rid}/rollback"), Who::Owner, Some(json!({ "version_id": v2 }))).await;
    assert_eq!(st, StatusCode::OK, "{rb}");
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let (_, body, _) = t.req("GET", &first_img_src(report["version"]["html"].as_str().unwrap()), Who::Owner, None).await;
    assert_eq!(body.as_str().unwrap(), "IMAGE-ONE");

    // Revoking the link revokes its images too.
    t.req("POST", &format!("/api/publications/{pid}/revoke"), Who::Owner, None).await;
    let (st, _, _) = t.req("GET", &shared, Who::Anon, None).await;
    assert_eq!(st, StatusCode::GONE);
}

#[tokio::test]
async fn extra_changes_must_be_confirmed_before_the_round_is_done() {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let para = block_id(&report, "同比增长");
    let (_, c, _) = t
        .req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "核实", "anchor": { "type": "block", "block_id": para } })))
        .await;
    let cid = c["id"].as_str().unwrap().to_string();
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let round_id = round["id"].as_str().unwrap().to_string();

    // V2 also edits the uncommented risk paragraph.
    let (_, res, _) = t
        .req("POST", &format!("/api/rounds/{round_id}/result"), Who::Agent, Some(json!({ "markdown": V2, "summary": "x",
            "replies": [{ "comment_id": cid, "action": "changed", "body": "已改" }] })))
        .await;
    let extra = res["extra_changes"].as_array().unwrap();
    assert_eq!(extra.len(), 1, "{res}");
    assert!(res["completed_at"].is_null(), "{res}");
    let extra_block = extra[0]["block_id"].as_str().unwrap().to_string();

    // Rolling back mid-verification would make the review diff describe a document that no longer exists.
    let base = round["base_version_id"].as_str().unwrap();
    let rollback = format!("/api/reports/{rid}/rollback");
    let (st, err, _) = t.req("POST", &rollback, Who::Owner, Some(json!({ "version_id": base }))).await;
    assert_eq!(st, StatusCode::CONFLICT, "{err}");
    assert!(err["error"].as_str().unwrap().contains("待验证"), "{err}");

    t.req("POST", &format!("/api/comments/{cid}/resolve"), Who::Owner, None).await;
    let (_, rd, _) = t.req("GET", &format!("/api/rounds/{round_id}"), Who::Owner, None).await;
    assert_eq!(rd["status"], "verifying");

    let confirm = format!("/api/rounds/{round_id}/extra/confirm");
    let (st, _, _) = t.req("POST", &confirm, Who::Agent, Some(json!({}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = t.req("POST", &confirm, Who::Owner, Some(json!({ "block_id": "b_nope" }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, rd, _) = t.req("POST", &confirm, Who::Owner, Some(json!({ "block_id": extra_block }))).await;
    assert_eq!(st, StatusCode::OK, "{rd}");
    assert_eq!(rd["status"], "done");
    assert!(rd["completed_at"].is_i64(), "{rd}");
    let (st, _, _) = t.req("POST", &confirm, Who::Owner, Some(json!({}))).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, rb, _) = t.req("POST", &rollback, Who::Owner, Some(json!({ "version_id": base }))).await;
    assert_eq!(st, StatusCode::OK, "{rb}");
}

#[tokio::test]
async fn a_comment_on_a_heading_covers_the_whole_nested_section() {
    const NESTED_V1: &str = "---\ntitle: 架构\n---\n\n## 三、架构设计\n\n总览一句。\n\n### 3.1 会话\n\n会话段落。\n\n### 3.2 记忆\n\n记忆段落。\n\n## 四、场景\n\n场景段落。\n";
    // The agent rewrites 3.1, drops 3.2, adds 3.3 — and also touches section 四, which no comment covers.
    const NESTED_V2: &str = "---\ntitle: 架构\n---\n\n## 三、架构设计\n\n总览一句。\n\n### 3.1 会话\n\n会话段落，重写过。\n\n### 3.3 技能\n\n技能段落。\n\n## 四、场景\n\n场景段落改了。\n";
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": NESTED_V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let heading = block_id(&report, "三、架构设计");
    let (st, c, _) = t
        .req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "只留最有特色的小节",
            "anchor": { "type": "text", "block_id": heading, "start": 2, "end": 6, "quote": "架构设计" } })))
        .await;
    assert_eq!(st, StatusCode::OK, "{c}");
    let cid = c["id"].as_str().unwrap().to_string();
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let round_id = round["id"].as_str().unwrap().to_string();
    let (st, res, _) = t
        .req("POST", &format!("/api/rounds/{round_id}/result"), Who::Agent, Some(json!({ "markdown": NESTED_V2, "summary": "x",
            "replies": [{ "comment_id": cid, "action": "changed", "body": "已精简" }] })))
        .await;
    assert_eq!(st, StatusCode::OK, "{res}");
    let extra = res["extra_changes"].as_array().unwrap();
    let extra_texts: Vec<&str> = extra.iter().filter_map(|e| e["new_text"].as_str()).collect();
    assert_eq!(extra_texts, vec!["场景段落改了。"], "only the uncommented section is extra: {res}");

    // The verify page lists the subsection changes under the heading comment itself.
    let (_, review, _) = t.req("GET", &format!("/api/rounds/{round_id}/review"), Who::Owner, None).await;
    let item = &review["items"][0];
    let section_changes = item["section_changes"].as_array().unwrap();
    assert!(section_changes.len() >= 3, "rewrite, deletion and addition should all be listed: {item}");
    assert!(section_changes.iter().all(|ch| ch["new_text"] != "场景段落改了。"), "{item}");
}

#[tokio::test]
async fn reverting_an_extra_change_drafts_a_comment() {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let para = block_id(&report, "同比增长");
    let (_, c, _) = t
        .req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "核实", "anchor": { "type": "block", "block_id": para } })))
        .await;
    let cid = c["id"].as_str().unwrap().to_string();
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let round_id = round["id"].as_str().unwrap().to_string();
    let (_, res, _) = t
        .req("POST", &format!("/api/rounds/{round_id}/result"), Who::Agent, Some(json!({ "markdown": V2, "summary": "x",
            "replies": [{ "comment_id": cid, "action": "changed", "body": "已改" }] })))
        .await;
    let extra_block = res["extra_changes"][0]["block_id"].as_str().unwrap().to_string();
    t.req("POST", &format!("/api/comments/{cid}/resolve"), Who::Owner, None).await;

    let revert = format!("/api/rounds/{round_id}/extra/revert");
    let req = json!({ "block_ids": [extra_block], "body": "请恢复为修改前的内容" });
    let (st, _, _) = t.req("POST", &revert, Who::Agent, Some(req.clone())).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, rd, _) = t.req("POST", &revert, Who::Owner, Some(req.clone())).await;
    assert_eq!(st, StatusCode::OK, "{rd}");
    assert_eq!(rd["status"], "done", "a reverted change no longer blocks the round");
    let draft_id = rd["extra_changes"][0]["revert_comment_id"].as_str().unwrap().to_string();

    let (_, all, _) = t.req("GET", &format!("/api/reports/{rid}/comments"), Who::Owner, None).await;
    let draft = all.as_array().unwrap().iter().find(|c| c["id"] == draft_id.as_str()).expect("draft comment").clone();
    assert_eq!(draft["status"], "draft", "{draft}");
    assert_eq!(draft["anchor"]["block_id"], extra_block.as_str(), "{draft}");
    let body = draft["body"].as_str().unwrap();
    assert!(body.starts_with("请恢复为修改前的内容") && body.ends_with("原材料价格波动。"), "{body}");
    assert!(!body.contains("另有新增一句"), "{body}");
    let (st, _, _) = t.req("POST", &revert, Who::Owner, Some(req)).await;
    assert_ne!(st, StatusCode::OK, "an extra change is handled only once");
}

#[tokio::test]
async fn results_with_broken_tables_are_rejected_whole() {
    let t = T::new().await;
    let table = "| 指标 | 2025 |\n| --- | --- |\n| 装机 | 120 GW |\n";
    // An imported report may already contain a broken table; that alone must not block the agent.
    let legacy = "| 旧表 | 2024 | 增速 |\n| --- | --- |\n| 装机 | 90 GW | - |\n";
    let md = format!("{V1}\n{table}\n{legacy}");
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": md }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, c, _) = t
        .req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "加一列增速", "anchor": { "type": "document" } })))
        .await;
    let cid = c["id"].as_str().unwrap().to_string();
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let result = format!("/api/rounds/{}/result", round["id"].as_str().unwrap());
    let reply = json!([{ "comment_id": cid, "action": "changed", "body": "已加" }]);

    // Header gained a column but the delimiter row did not.
    let broken = md.replace("| 指标 | 2025 |\n", "| 指标 | 2025 | 增速 |\n");
    let (st, err, _) = t.req("POST", &result, Who::Agent, Some(json!({ "markdown": broken, "summary": "x", "replies": reply }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{err}");
    assert!(err["error"].as_str().unwrap().contains("表格"), "{err}");
    let (_, versions, _) = t.req("GET", &format!("/api/reports/{rid}/versions"), Who::Owner, None).await;
    assert_eq!(versions.as_array().unwrap().len(), 1, "rejected result left a version behind");

    let fixed = broken.replace("| --- | --- |\n| 装机 | 120 GW |", "| --- | --- | --- |\n| 装机 | 120 GW | 10% |");
    let (st, rd, _) = t.req("POST", &result, Who::Agent, Some(json!({ "markdown": fixed, "summary": "x", "replies": reply }))).await;
    assert_eq!((st, rd["status"].as_str()), (StatusCode::OK, Some("verifying")), "{rd}");
}

#[tokio::test]
async fn expired_claims_are_reclaimed() {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    t.req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "全文", "anchor": { "type": "document" } })))
        .await;
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let claim = format!("/api/rounds/{}/claim", round["id"].as_str().unwrap());
    let (st, _, _) = t.req("POST", &claim, Who::Agent, None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = t.req("POST", &claim, Who::Agent, None).await;
    assert_eq!(st, StatusCode::CONFLICT);
    t.state.store().conn.execute("UPDATE rounds SET claimed_at = 0", []).unwrap();
    let (_, rd, _) = t.req("GET", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    assert_eq!(rd[0]["status"], "submitted");
    let (st, rd, _) = t.req("POST", &claim, Who::Agent, None).await;
    assert_eq!(st, StatusCode::OK, "{rd}");
    assert_eq!(rd["status"], "processing");
}

#[tokio::test]
async fn only_the_owner_sees_the_agent_token() {
    let t = T::new().await;
    let (st, meta, _) = t.req("GET", "/api/meta", Who::Owner, None).await;
    assert_eq!((st, meta["agent_token"].as_str()), (StatusCode::OK, Some(AGENT)));
    assert!(meta["agent_last_seen"].is_null(), "no agent has connected yet: {meta}");
    for who in [Who::Agent, Who::Anon] {
        let (st, body, _) = t.req("GET", "/api/meta", who, None).await;
        assert!(st.is_client_error() && !body.to_string().contains(AGENT), "{st} {body}");
    }
    // The agent's (rejected) meta request above still authenticated, so the owner now sees it as connected.
    let (_, meta, _) = t.req("GET", "/api/meta", Who::Owner, None).await;
    assert!(meta["agent_last_seen"].as_i64().is_some(), "{meta}");
}

#[tokio::test]
async fn archive_freezes_a_report_and_delete_requires_it() {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let para = block_id(&report, "同比增长");
    let comment = json!({ "body": "核实", "anchor": { "type": "block", "block_id": para } });
    let (_, p, _) = t.req("POST", &format!("/api/reports/{rid}/publications"), Who::Owner, None).await;
    let share = p["publication"]["path"].as_str().unwrap().to_string();

    // Not archived yet: deleting is refused, and the agent can't archive.
    let (st, _, _) = t.req("DELETE", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, _, _) = t.req("POST", &format!("/api/reports/{rid}/archive"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Archive while the agent is mid-round: allowed, the round freezes and the agent's result is refused.
    let (_, c, _) = t.req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(comment.clone())).await;
    let cid = c["id"].as_str().unwrap().to_string();
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let round_id = round["id"].as_str().unwrap().to_string();
    let (st, _, _) = t.req("POST", &format!("/api/rounds/{round_id}/claim"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, info, _) = t.req("POST", &format!("/api/reports/{rid}/archive"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::OK, "{info}");
    assert!(info["archived_at"].is_i64(), "{info}");
    let result = json!({ "markdown": V2, "summary": "x", "replies": [{ "comment_id": cid, "action": "changed", "body": "已改" }] });
    let (st, e, _) = t.req("POST", &format!("/api/rounds/{round_id}/result"), Who::Agent, Some(result.clone())).await;
    assert_eq!(st, StatusCode::CONFLICT, "{e}");
    assert!(e["error"].as_str().unwrap().contains("未保存"), "{e}");
    let (_, pending, _) = t.req("GET", "/api/pending", Who::Agent, None).await;
    assert!(pending.as_array().unwrap().is_empty(), "archived rounds are hidden from the agent: {pending}");
    let (_, rd, _) = t.req("GET", &format!("/api/rounds/{round_id}"), Who::Owner, None).await;
    assert_eq!(rd["status"], "processing", "the round itself is untouched: {rd}");

    // Frozen for the owner too (comments, rounds, new versions)...
    let (st, e, _) = t.req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(comment.clone())).await;
    assert_eq!(st, StatusCode::CONFLICT, "{e}");
    assert!(e["error"].as_str().unwrap().contains("已归档"), "{e}");
    let (st, _, _) = t.req("POST", &format!("/api/comments/{cid}/resolve"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, _, _) = t.req("POST", &format!("/api/reports/{rid}/versions"), Who::Agent, Some(json!({ "markdown": V2 }))).await;
    assert_eq!(st, StatusCode::CONFLICT);
    // ...but still readable, and readers of the share link notice nothing.
    let (st, _, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = t.req("GET", &share, Who::Anon, None).await;
    assert_eq!(st, StatusCode::OK);

    // Restore: the round resumes where it was and the agent can hand in its result now.
    let (st, info, _) = t.req("POST", &format!("/api/reports/{rid}/unarchive"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::OK, "{info}");
    assert!(info["archived_at"].is_null(), "{info}");
    let (_, pending, _) = t.req("GET", "/api/pending", Who::Agent, None).await;
    assert_eq!(pending.as_array().unwrap().len(), 1, "{pending}");
    let (st, rd, _) = t.req("POST", &format!("/api/rounds/{round_id}/result"), Who::Agent, Some(result)).await;
    assert_eq!(st, StatusCode::OK, "{rd}");
    assert_eq!(rd["status"], "verifying");

    // Archive again (mid-verification is fine too), then delete: everything is gone, share link included.
    let (st, _, _) = t.req("POST", &format!("/api/reports/{rid}/archive"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = t.req("DELETE", &format!("/api/reports/{rid}"), Who::Agent, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, body, _) = t.req("DELETE", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let (st, _, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _, _) = t.req("GET", &share, Who::Anon, None).await;
    assert!(st.is_client_error(), "{st}");
    let (_, list, _) = t.req("GET", "/api/reports", Who::Owner, None).await;
    assert!(list.as_array().unwrap().is_empty(), "{list}");
}

#[tokio::test]
async fn sessions_are_stored_hashed() {
    let t = T::new().await;
    let raw = t.cookie.split_once('=').unwrap().1.to_string();
    let store = t.state.store();
    let n: i64 = store.conn.query_row("SELECT COUNT(*) FROM sessions WHERE token = ?1", [&raw], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
    assert!(store.session_valid(&raw).unwrap());
}
