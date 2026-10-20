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
    for leaked in [marker, "已改为", "语气更客观", c1.as_str(), "data-block", "data-cell"] {
        assert!(!html.contains(leaked), "share page leaked {leaked}");
    }
    assert!(headers.contains("noindex"));
    // The image viewer is the only script, admitted by a per-response nonce.
    let nonce = headers.split("script-src 'nonce-").nth(1).and_then(|s| s.split('\'').next()).expect("no script nonce");
    assert_eq!(html.matches("<script").count(), 1, "share page may only load its viewer");
    assert!(html.contains(&format!(r#"<script nonce="{nonce}" src="/static/share.js?v="#)), "{html}");
    assert!(!headers.contains("unsafe-"), "{headers}");
    let (_, _, again) = t.req("GET", &path, Who::Anon, None).await;
    assert!(!again.contains(&format!("'nonce-{nonce}'")), "nonce reused across responses");
    let (st, _, js) = t.req("GET", "/static/share.js", Who::Anon, None).await;
    assert_eq!(st, StatusCode::OK, "share.js missing: run `corepack pnpm build` in web/ first");
    assert!(js.contains("text/javascript"), "{js}");
    assert!(headers.contains("img-src 'self' data: http: https:;"));
    assert!(headers.contains("no-referrer"));
    assert!(html.contains(r#"<link rel="icon" href="/favicon.svg""#), "share page has no favicon");

    // Republishing keeps the link; revoking returns 410; unknown tokens 404.
    let (_, p2, _) = t.req("POST", &format!("/api/reports/{rid}/publications"), Who::Owner, None).await;
    assert_eq!(p2["publication"]["path"].as_str().unwrap(), path);
    let (st, _, _) = t.req("POST", &format!("/api/publications/{pid}/revoke"), Who::Owner, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, gone, gone_headers) = t.req("GET", &path, Who::Anon, None).await;
    assert_eq!(st, StatusCode::GONE);
    assert!(gone_headers.contains("script-src 'none'"), "{gone_headers}");
    assert!(gone.as_str().unwrap().contains(r#"<link rel="icon" href="/favicon.svg""#), "status page has no favicon");
    let (st, _, _) = t.req("GET", "/s/doesnotexist", Who::Anon, None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn external_images_show_on_the_share_page() {
    let t = T::new().await;
    let md = "---\ntitle: 外链\n---\n\n![图](https://img.example.com/a.png)\n";
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": md }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, p, _) = t.req("POST", &format!("/api/reports/{rid}/publications"), Who::Owner, None).await;
    let path = p["publication"]["path"].as_str().unwrap().to_string();
    let (st, html, headers) = t.req("GET", &path, Who::Anon, None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(html.as_str().unwrap().contains(r#"<img src="https://img.example.com/a.png" alt="图">"#), "{html}");
    assert!(headers.contains("https:"), "{headers}");
    assert!(headers.contains("no-referrer"), "{headers}");
}

#[tokio::test]
async fn site_icons_are_public() {
    let t = T::new().await;
    for (path, mime) in [
        ("/favicon.svg", "image/svg+xml"),
        ("/favicon.ico", "image/x-icon"),
        ("/apple-touch-icon.png", "image/png"),
    ] {
        let (st, _, headers) = t.req("GET", path, Who::Anon, None).await;
        assert_eq!(st, StatusCode::OK, "{path}");
        assert!(headers.contains(mime), "{path}: {headers}");
    }
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
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    for asked in [json!("1999-01-01"), Value::Null] {
        let (_, init, _) = t.req("POST", "/mcp", Who::Agent, Some(call(1, "initialize", json!({ "protocolVersion": asked })))).await;
        assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    }
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

#[tokio::test]
async fn mcp_does_not_offer_archived_rounds() {
    let t = T::new().await;
    let tool = |name: &str, args: Value| {
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } })
    };
    let text = |v: &Value| v["result"]["content"][0]["text"].as_str().unwrap().to_string();
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let comment = json!({ "body": "核实", "anchor": { "type": "block", "block_id": block_id(&report, "同比增长") } });
    t.req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(comment)).await;
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let round_id = round["id"].as_str().unwrap().to_string();

    let (_, list, _) = t.req("POST", "/mcp", Who::Agent, Some(tool("galley_list_reports", json!({})))).await;
    assert!(text(&list).contains("等 AI 处理") && text(&list).contains(&round_id), "{list}");

    // Archived while still submitted: listed as archived, not offered, and nothing is claimed.
    t.req("POST", &format!("/api/reports/{rid}/archive"), Who::Owner, None).await;
    let (_, list, _) = t.req("POST", "/mcp", Who::Agent, Some(tool("galley_list_reports", json!({})))).await;
    let list = text(&list);
    assert!(list.contains("已归档") && !list.contains(&round_id) && !list.contains("submitted"), "{list}");
    let (_, got, _) = t.req("POST", "/mcp", Who::Agent, Some(tool("galley_get_round", json!({ "report_id": rid })))).await;
    assert_eq!(text(&got), "报告已归档，没有待处理的轮次。", "{got}");
    let (_, got, _) = t.req("POST", "/mcp", Who::Agent, Some(tool("galley_get_round", json!({})))).await;
    assert!(text(&got).contains("没有待处理"), "{got}");
    let (_, rd, _) = t.req("GET", &format!("/api/rounds/{round_id}"), Who::Owner, None).await;
    assert_eq!(rd["status"], "submitted", "{rd}");

    // Archived mid-processing: the packet is not handed out again either.
    t.req("POST", &format!("/api/reports/{rid}/unarchive"), Who::Owner, None).await;
    let (_, got, _) = t.req("POST", "/mcp", Who::Agent, Some(tool("galley_get_round", json!({ "report_id": rid })))).await;
    assert!(text(&got).contains(&round_id), "{got}");
    t.req("POST", &format!("/api/reports/{rid}/archive"), Who::Owner, None).await;
    let (_, got, _) = t.req("POST", "/mcp", Who::Agent, Some(tool("galley_get_round", json!({ "report_id": rid })))).await;
    assert_eq!(text(&got), "报告已归档，没有待处理的轮次。", "{got}");
    let (_, rd, _) = t.req("GET", &format!("/api/rounds/{round_id}"), Who::Owner, None).await;
    assert_eq!(rd["status"], "processing", "{rd}");
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

/// Comments `commented` paragraphs of `before` (one block comment each), submits `after`, and returns the
/// texts of the extra changes plus the review.
async fn rewrite_round(before: &str, commented: &[&str], after: &str) -> (Vec<String>, Value) {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": before }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let mut replies = Vec::new();
    for text in commented {
        let anchor = json!({ "type": "block", "block_id": block_id(&report, text) });
        let (st, c, _) = t.req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "改", "anchor": anchor }))).await;
        assert_eq!(st, StatusCode::OK, "{c}");
        replies.push(json!({ "comment_id": c["id"], "action": "changed", "body": "已改" }));
    }
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let round_id = round["id"].as_str().unwrap().to_string();
    let (st, res, _) = t
        .req("POST", &format!("/api/rounds/{round_id}/result"), Who::Agent, Some(json!({ "markdown": after, "summary": "x", "replies": replies })))
        .await;
    assert_eq!(st, StatusCode::OK, "{res}");
    let extra = res["extra_changes"].as_array().unwrap();
    let texts = extra.iter().map(|e| e["new_text"].as_str().or(e["old_text"].as_str()).unwrap().to_string()).collect();
    let (_, review, _) = t.req("GET", &format!("/api/rounds/{round_id}/review"), Who::Owner, None).await;
    (texts, review)
}

const REWRITE_V1: &str = "---\ntitle: 介绍\n---\n\n## 一、简介\n\nGalley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。\n\n它的出发点是替代另一个产品里写报告的流程。\n\n## 二、安装\n\n用 Docker 启动。\n";

#[tokio::test]
async fn rewriting_commented_paragraphs_beyond_recognition_is_not_extra() {
    // Both paragraphs are rewritten too much to keep their block ids, so the diff sees "2 deleted + 2 added".
    let after = REWRITE_V1
        .replace("Galley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。", "像批改文稿一样审阅 AI 写的报告，核对无误后再发布。")
        .replace("它的出发点是替代另一个产品里写报告的流程。", "很难看出每一版到底改了哪里，也不知道有没有顺手动了别处。");
    let (extra, review) = rewrite_round(REWRITE_V1, &["自托管", "出发点"], &after).await;
    assert!(extra.is_empty(), "{extra:?}");

    // The new text is listed under the comments, since it is no longer listed as extra.
    let item = &review["items"][0];
    let shown: Vec<&str> = item["section_changes"].as_array().unwrap().iter().filter_map(|c| c["new_text"].as_str()).collect();
    assert!(shown.iter().any(|t| t.starts_with("像批改文稿")), "{item}");
}

#[tokio::test]
async fn splitting_a_commented_paragraph_into_several_is_not_extra() {
    let after = REWRITE_V1.replace(
        "它的出发点是替代另一个产品里写报告的流程。",
        "每次改完很难核对。\n\n- 不知道改了哪里；\n- 不知道有没有动别处。",
    );
    let (extra, _) = rewrite_round(REWRITE_V1, &["出发点"], &after).await;
    assert!(extra.is_empty(), "{extra:?}");
}

#[tokio::test]
async fn new_text_beside_a_commented_paragraph_is_still_extra() {
    // "Remove this" on the first paragraph; the agent also drops the uncommented second one and writes
    // something new in their place. The new paragraph replaces more than what was commented on.
    let after = REWRITE_V1.replace(
        "Galley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。\n\n它的出发点是替代另一个产品里写报告的流程。",
        "全新的一段，没有人要求写它。",
    );
    let (extra, review) = rewrite_round(REWRITE_V1, &["自托管"], &after).await;
    assert!(extra.iter().any(|t| t == "全新的一段，没有人要求写它。"), "{extra:?}");
    assert!(extra.iter().any(|t| t.starts_with("它的出发点")), "{extra:?}");
    // The new paragraph wasn't asked for, so the comment doesn't claim it.
    assert_eq!(review["items"][0]["comment"]["anchor_state"], "orphaned", "{review}");
}

/// Comments on `before` with the anchors `anchors` builds from the report, answers every comment with `action`,
/// submits `after` and returns the round's review.
async fn anchored_round(before: &str, anchors: impl Fn(&Value) -> Vec<Value>, action: &str, after: &str) -> Value {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": before }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let mut replies = Vec::new();
    for anchor in anchors(&report) {
        let (st, c, _) = t.req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "改", "anchor": anchor }))).await;
        assert_eq!(st, StatusCode::OK, "{c}");
        replies.push(json!({ "comment_id": c["id"], "action": action, "body": "已处理" }));
    }
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let round_id = round["id"].as_str().unwrap().to_string();
    let (st, res, _) = t
        .req("POST", &format!("/api/rounds/{round_id}/result"), Who::Agent, Some(json!({ "markdown": after, "summary": "x", "replies": replies })))
        .await;
    assert_eq!(st, StatusCode::OK, "{res}");
    let (_, review, _) = t.req("GET", &format!("/api/rounds/{round_id}/review"), Who::Owner, None).await;
    review
}

/// The text of the block a review item's comment is anchored on in the result version.
fn anchored_text<'a>(review: &'a Value, item: usize) -> &'a str {
    let c = &review["items"][item]["comment"];
    let id = c["anchor"]["block_id"].as_str().or(c["anchor"]["section_id"].as_str()).unwrap_or_default();
    review["items"][item]["section_changes"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(review["items"][item]["change"].as_object().map(|_| &review["items"][item]["change"]))
        .find(|ch| ch["block_id"] == id)
        .and_then(|ch| ch["new_text"].as_str())
        .unwrap_or_default()
}

#[tokio::test]
async fn comments_follow_paragraphs_rewritten_as_asked() {
    let after = REWRITE_V1
        .replace("Galley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。", "像批改文稿一样审阅 AI 写的报告，核对无误后再发布。")
        .replace("它的出发点是替代另一个产品里写报告的流程。", "很难看出每一版到底改了哪里，也不知道有没有顺手动了别处。");
    let (_, review) = rewrite_round(REWRITE_V1, &["自托管", "出发点"], &after).await;
    for i in 0..2 {
        assert_eq!(review["items"][i]["comment"]["anchor_state"], "moved", "{review}");
    }
    // Rewritten one for one, so each comment lands on the paragraph that replaced its own.
    let texts: Vec<&str> = (0..2).map(|i| anchored_text(&review, i)).collect();
    assert!(texts.iter().any(|t| t.starts_with("像批改文稿")) && texts.iter().any(|t| t.starts_with("很难看出")), "{texts:?}");
}

#[tokio::test]
async fn a_comment_follows_a_paragraph_split_into_several() {
    let after = REWRITE_V1.replace(
        "它的出发点是替代另一个产品里写报告的流程。",
        "每次改完很难核对。\n\n- 不知道改了哪里；\n- 不知道有没有动别处。",
    );
    let (_, review) = rewrite_round(REWRITE_V1, &["出发点"], &after).await;
    assert_eq!(review["items"][0]["comment"]["anchor_state"], "moved", "{review}");
    assert_eq!(anchored_text(&review, 0), "每次改完很难核对。", "{review}");
}

#[tokio::test]
async fn a_text_comment_with_a_question_follows_the_rewrite_and_stays_a_question() {
    let quote = "自托管的审阅工作台";
    let after = REWRITE_V1.replace("Galley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。", "像批改文稿一样审阅 AI 写的报告，核对无误后再发布。");
    let review = anchored_round(
        REWRITE_V1,
        |report| {
            let text = "Galley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。";
            let start = text[..text.find(quote).unwrap()].chars().count();
            vec![json!({ "type": "text", "block_id": block_id(report, "自托管"), "start": start, "end": start + quote.chars().count(),
                "quote": quote, "prefix": "", "suffix": "" })]
        },
        "clarify",
        &after,
    )
    .await;
    let c = &review["items"][0]["comment"];
    assert_eq!(c["anchor_state"], "moved", "{review}");
    assert_eq!(c["anchor"]["type"], "block", "{review}");
    assert_eq!(c["status"], "clarify", "a followed comment must not be marked orphaned: {review}");
}

#[tokio::test]
async fn a_redrawn_diagram_keeps_its_id_and_its_comment() {
    let before = "---\ntitle: 图\n---\n\n## 架构\n\n第一段说明。\n\n<svg width=\"200\" height=\"100\"><text x=\"20\" y=\"35\">入口</text></svg>\n\n第二段说明。\n";
    let after = before.replace("<text x=\"20\" y=\"35\">入口</text>", "<rect width=\"120\" height=\"40\"/><text x=\"20\" y=\"35\">入口和网关</text>");
    let review = anchored_round(
        before,
        |report| {
            let blocks = report["version"]["doc"]["blocks"].as_array().unwrap();
            let fig = blocks.iter().find(|b| b["kind"] != "heading" && b["kind"] != "paragraph").unwrap();
            vec![json!({ "type": "block", "block_id": fig["id"] })]
        },
        "changed",
        &after,
    )
    .await;
    // The only diagram on either side of the change is the same figure redrawn: modified, not deleted + added.
    let item = &review["items"][0];
    assert_eq!(item["comment"]["anchor_state"], "exact", "{review}");
    assert_eq!(item["base_anchor"]["block_id"], item["comment"]["anchor"]["block_id"], "{review}");
    assert_eq!(item["change"]["op"], "modified", "{review}");
}

#[tokio::test]
async fn a_section_comment_follows_its_rewritten_heading() {
    let after = REWRITE_V1.replace("## 一、简介", "## 一、为什么要有它");
    let review = anchored_round(
        REWRITE_V1,
        |report| vec![json!({ "type": "section", "section_id": block_id(report, "简介") })],
        "changed",
        &after,
    )
    .await;
    let c = &review["items"][0]["comment"];
    assert_eq!(c["anchor_state"], "moved", "{review}");
    assert_eq!(c["anchor"]["type"], "section", "{review}");
    assert_eq!(review["items"][0]["section_title"], "一、为什么要有它", "{review}");
}

#[tokio::test]
async fn a_deleted_paragraph_does_not_follow_its_rewritten_neighbour() {
    // Both paragraphs are commented: the first is rewritten, the second deleted as asked. The diff sees one run of
    // "2 deleted + 1 added" and can't tell which comment the new paragraph answers, so neither follows it rather
    // than the deletion request landing on the rewrite.
    let after = REWRITE_V1
        .replace("Galley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。", "像批改文稿一样审阅 AI 写的报告，核对无误后再发布。")
        .replace("它的出发点是替代另一个产品里写报告的流程。\n\n", "");
    let (_, review) = rewrite_round(REWRITE_V1, &["自托管", "出发点"], &after).await;
    for i in 0..2 {
        assert_eq!(review["items"][i]["comment"]["anchor_state"], "orphaned", "{review}");
    }
}

#[tokio::test]
async fn a_comment_on_a_deleted_paragraph_stays_orphaned() {
    // "Remove this" carried out with nothing written in its place: there is nothing to follow.
    let after = REWRITE_V1.replace("它的出发点是替代另一个产品里写报告的流程。\n\n", "");
    let (_, review) = rewrite_round(REWRITE_V1, &["出发点"], &after).await;
    assert_eq!(review["items"][0]["comment"]["anchor_state"], "orphaned", "{review}");
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

/// A report from V1 with a submitted round of one block comment on the growth figure: `(rid, round_id, comment_id)`.
async fn round_on_growth(t: &T) -> (String, String, String) {
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let anchor = json!({ "type": "block", "block_id": block_id(&report, "同比增长") });
    let (_, c, _) = t.req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "核实", "anchor": anchor }))).await;
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    (rid, round["id"].as_str().unwrap().to_string(), c["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn edits_give_the_same_result_as_the_full_markdown() {
    let full = T::new().await;
    let (rid_a, round_a, cid_a) = round_on_growth(&full).await;
    let (st, res_a, _) = full
        .req("POST", &format!("/api/rounds/{round_a}/result"), Who::Agent, Some(json!({ "markdown": V2, "summary": "x",
            "replies": [{ "comment_id": cid_a, "action": "changed", "body": "已改" }] })))
        .await;
    assert_eq!(st, StatusCode::OK, "{res_a}");

    let edits = T::new().await;
    let (rid_b, round_b, cid_b) = round_on_growth(&edits).await;
    let (st, res_b, _) = edits
        .req("POST", &format!("/api/rounds/{round_b}/result"), Who::Agent, Some(json!({ "summary": "x",
            "replies": [{ "comment_id": cid_b, "action": "changed", "body": "已改",
                "edits": [{ "old": "同比增长 10%", "new": "同比增长 12.3%（来源：BNEF）" }] }],
            "edits": [{ "old": "原材料价格波动。", "new": "原材料价格波动。另有新增一句。" }] })))
        .await;
    assert_eq!(st, StatusCode::OK, "{res_b}");

    let (_, src_a, _) = full.req("GET", &format!("/api/reports/{rid_a}/source?format=raw"), Who::Owner, None).await;
    let (_, src_b, _) = edits.req("GET", &format!("/api/reports/{rid_b}/source?format=raw"), Who::Owner, None).await;
    assert_eq!(src_b, Value::String(V2.into()));
    assert_eq!(src_a, src_b);
    let ops = |r: &Value| r["extra_changes"].as_array().unwrap().iter().map(|e| e["op"].clone()).collect::<Vec<_>>();
    assert_eq!(ops(&res_a), ops(&res_b));
    // Only the edits say the agent meant the change outside the comment.
    assert_eq!(res_a["extra_changes"][0].get("declared"), None, "{res_a}");
    assert_eq!(res_b["extra_changes"][0]["declared"], true, "{res_b}");
}

#[tokio::test]
async fn a_failing_edit_rejects_the_whole_result() {
    let t = T::new().await;
    let (rid, round_id, cid) = round_on_growth(&t).await;
    let path = format!("/api/rounds/{round_id}/result");
    let reply = |edits: Value| json!({ "comment_id": cid, "action": "changed", "body": "已改", "edits": edits });

    // The first edit is fine, the second can't be found: nothing is applied.
    let body = json!({ "summary": "x", "replies": [reply(json!([
        { "old": "同比增长 10%", "new": "同比增长 12.3%" },
        { "old": "不存在的句子", "new": "x" }
    ]))] });
    let (st, err, _) = t.req("POST", &path, Who::Agent, Some(body)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let msg = err["error"].as_str().unwrap();
    assert!(msg.contains(&format!("{cid} 的第 2 条 edit")) && msg.contains("找不到") && msg.contains("未生效"), "{msg}");

    let both = json!({ "summary": "x", "markdown": V2, "replies": [reply(json!([{ "old": "同比增长 10%", "new": "y" }]))] });
    let (st, err, _) = t.req("POST", &path, Who::Agent, Some(both)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("二选一"), "{err}");

    let answered = json!({ "summary": "x", "replies": [{ "comment_id": cid, "action": "answered", "body": "数字没错",
        "edits": [{ "old": "同比增长 10%", "new": "y" }] }] });
    let (st, err, _) = t.req("POST", &path, Who::Agent, Some(answered)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("不能带 edits"), "{err}");

    let (_, rd, _) = t.req("GET", &format!("/api/rounds/{round_id}"), Who::Owner, None).await;
    assert_eq!(rd["status"], "submitted", "{rd}");
    let (_, versions, _) = t.req("GET", &format!("/api/reports/{rid}/versions"), Who::Owner, None).await;
    assert_eq!(versions.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn agents_submit_and_push_edits_over_mcp() {
    let t = T::new().await;
    let tool = |name: &str, args: Value| {
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } })
    };
    let text = |v: &Value| v["result"]["content"][0]["text"].as_str().unwrap().to_string();
    let (rid, round_id, cid) = round_on_growth(&t).await;

    // The packet carries what a block edit needs: the block id and its source.
    let (_, packet, _) = t.req("POST", "/mcp", Who::Agent, Some(tool("galley_get_round", json!({ "report_id": rid })))).await;
    let packet = text(&packet);
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let block = block_id(&report, "同比增长");
    assert!(packet.contains(&format!("块 `{block}`")) && packet.contains("```markdown\n2025 年装机 120 GW，同比增长 10%。\n```"), "{packet}");

    let (_, res, _) = t
        .req("POST", "/mcp", Who::Agent, Some(tool("galley_submit_round", json!({ "round_id": round_id, "summary": "x",
            "replies": [{ "comment_id": cid, "action": "changed", "body": "重写了这一段",
                "edits": [{ "block": block, "new": "2025 年新增装机 120 GW，增速放缓到 10%。" }] }] })))).await;
    assert_eq!(res["result"]["isError"], false, "{res}");
    t.req("POST", &format!("/api/comments/{cid}/resolve"), Who::Owner, None).await;

    let (_, pushed, _) = t
        .req("POST", "/mcp", Who::Agent, Some(tool("galley_push_version", json!({ "report_id": rid,
            "edits": [{ "old": "原材料价格波动。", "new": "原材料价格波动较大。" }] })))).await;
    assert_eq!(pushed["result"]["isError"], false, "{pushed}");
    let (_, src, _) = t.req("GET", &format!("/api/reports/{rid}/source?format=raw"), Who::Owner, None).await;
    let src = src.as_str().unwrap();
    assert!(src.contains("增速放缓到 10%") && src.contains("原材料价格波动较大。") && !src.contains("同比增长"), "{src}");

    let (_, err, _) = t
        .req("POST", "/mcp", Who::Agent, Some(tool("galley_push_version", json!({ "report_id": rid,
            "edits": [{ "old": "原材料价格波动较大。", "new": "x" }], "markdown": V1 })))).await;
    assert!(err["result"]["isError"] == true && text(&err).contains("二选一"), "{err}");
}

#[tokio::test]
async fn pushing_edits_over_http() {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (st, v, _) = t
        .req("POST", &format!("/api/reports/{rid}/versions"), Who::Agent, Some(json!({ "edits": [{ "old": "10%", "new": "12.3%" }], "note": "改数字" })))
        .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["seq"], 2);
    let (st, _, _) = t.req("POST", &format!("/api/reports/{rid}/versions"), Who::Agent, Some(json!({ "note": "空" }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// Comments on `before` with the anchors `anchors` builds from the report and answers each with the edits `edits`
/// builds for it, plus top-level edits `outside`. Returns the result and the review.
async fn edits_round(
    before: &str,
    anchors: impl Fn(&Value) -> Vec<Value>,
    edits: impl Fn(&Value, usize) -> Value,
    outside: Value,
) -> (Value, Value) {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": before }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let (_, report, _) = t.req("GET", &format!("/api/reports/{rid}"), Who::Owner, None).await;
    let mut replies = Vec::new();
    for (i, anchor) in anchors(&report).into_iter().enumerate() {
        let (_, c, _) = t.req("POST", &format!("/api/reports/{rid}/comments"), Who::Owner, Some(json!({ "body": "改", "anchor": anchor }))).await;
        replies.push(json!({ "comment_id": c["id"], "action": "changed", "body": "已改", "edits": edits(&report, i) }));
    }
    let (_, round, _) = t.req("POST", &format!("/api/reports/{rid}/rounds"), Who::Owner, None).await;
    let round_id = round["id"].as_str().unwrap().to_string();
    let (st, res, _) = t
        .req("POST", &format!("/api/rounds/{round_id}/result"), Who::Agent, Some(json!({ "summary": "x", "replies": replies, "edits": outside })))
        .await;
    assert_eq!(st, StatusCode::OK, "{res}");
    let (_, review, _) = t.req("GET", &format!("/api/rounds/{round_id}/review"), Who::Owner, None).await;
    (res, review)
}

fn on_block(id: String) -> Value {
    json!({ "type": "block", "block_id": id })
}

const TERMS_V1: &str = "---\ntitle: 术语\n---\n\n## 一、简介\n\n大模型的成本在下降。\n\n## 二、应用\n\n大模型用于客服。\n\n## 三、风险\n\n原材料价格波动。\n";

#[tokio::test]
async fn edits_under_a_reply_belong_to_its_comment_wherever_they_land() {
    // "Call it LLM throughout" on the first paragraph: the agent changes the term in both sections.
    let (res, review) =
        edits_round(TERMS_V1, |r| vec![on_block(block_id(r, "成本"))], |_, _| json!([{ "old": "大模型", "new": "LLM", "all": true }]), json!([])).await;
    assert_eq!(res["extra_changes"], json!([]), "{res}");
    let item = &review["items"][0];
    let shown: Vec<&str> = item["section_changes"].as_array().unwrap().iter().filter_map(|c| c["new_text"].as_str()).collect();
    assert_eq!(shown, ["LLM的成本在下降。", "LLM用于客服。"], "{item}");
}

#[tokio::test]
async fn a_block_replaced_by_id_keeps_its_id() {
    let before = "---\ntitle: 图\n---\n\n## 一、架构\n\n<svg width=\"10\"><rect/></svg>\n\n说明文字。\n";
    let svg = |r: &Value| {
        let blocks = r["version"]["doc"]["blocks"].as_array().unwrap();
        blocks.iter().find(|b| b["kind"] == "diagram").unwrap()["id"].as_str().unwrap().to_string()
    };
    let (res, review) = edits_round(
        before,
        |r| vec![on_block(svg(r))],
        |r, _| json!([{ "block": svg(r), "new": "<svg width=\"300\" height=\"200\"><circle r=\"40\"/><text>全新</text></svg>" }]),
        json!([]),
    )
    .await;
    assert_eq!(res["extra_changes"], json!([]), "{res}");
    let item = &review["items"][0];
    // Redrawn beyond recognition, it is still the same block: an edit, with the comment where it was.
    assert_eq!(item["comment"]["anchor_state"], "exact", "{item}");
    assert_eq!(item["change"]["op"], "modified", "{item}");
    assert_eq!(item["change"]["block_id"], item["comment"]["anchor"]["block_id"], "{item}");
}

#[tokio::test]
async fn a_comment_follows_the_text_its_own_edits_wrote() {
    // The case position alone can't settle (see `a_deleted_paragraph_does_not_follow_its_rewritten_neighbour`):
    // the edits say which comment the new paragraph answers.
    let (res, review) = edits_round(
        REWRITE_V1,
        |r| vec![on_block(block_id(r, "自托管")), on_block(block_id(r, "出发点"))],
        |_, i| match i {
            0 => json!([{ "old": "Galley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。", "new": "像批改文稿一样审阅 AI 写的报告，核对无误后再发布。" }]),
            _ => json!([{ "old": "它的出发点是替代另一个产品里写报告的流程。\n\n", "new": "" }]),
        },
        json!([]),
    )
    .await;
    assert_eq!(res["extra_changes"], json!([]), "{res}");
    assert_eq!(review["items"][0]["comment"]["anchor_state"], "moved", "{review}");
    assert_eq!(anchored_text(&review, 0), "像批改文稿一样审阅 AI 写的报告，核对无误后再发布。");
    assert_eq!(review["items"][1]["comment"]["anchor_state"], "orphaned", "{review}");
}

#[tokio::test]
async fn a_whole_document_comment_covers_changes_anywhere() {
    // "Polish the whole report": every paragraph and heading changes, none of it extra.
    let after = REWRITE_V1
        .replace("## 一、简介", "## 一、概述")
        .replace("Galley 是一个自托管的审阅工作台，在报告上写评论再交给 AI 修改。", "像批改文稿一样审阅 AI 写的报告。")
        .replace("用 Docker 启动。", "用 Docker Compose 一条命令启动。");
    let review = anchored_round(REWRITE_V1, |_| vec![json!({ "type": "document" })], "changed", &after).await;
    assert_eq!(review["round"]["extra_changes"], json!([]), "{review}");
}

#[tokio::test]
async fn a_whole_document_comment_leaves_declared_edits_extra() {
    // The agent set the risk edit apart from the comment, so it is still for the owner to confirm.
    let (res, _) = edits_round(
        TERMS_V1,
        |_| vec![json!({ "type": "document" })],
        |_, _| json!([{ "old": "大模型", "new": "LLM", "all": true }]),
        json!([{ "old": "原材料价格波动。", "new": "原材料价格波动较大。" }]),
    )
    .await;
    let extra = res["extra_changes"].as_array().unwrap();
    assert_eq!(extra.len(), 1, "{res}");
    assert_eq!((extra[0]["new_text"].as_str(), &extra[0]["declared"]), (Some("原材料价格波动较大。"), &json!(true)), "{res}");
}

#[tokio::test]
async fn versions_record_who_pushed_them() {
    let t = T::new().await;
    let (_, r, _) = t.req("POST", "/api/reports", Who::Agent, Some(json!({ "markdown": V1 }))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    // The note is free text; only the author tells the owner's rollback from an agent claiming one.
    let (st, _, _) = t
        .req("POST", &format!("/api/reports/{rid}/versions"), Who::Agent, Some(json!({ "markdown": V2, "note": "回退到 v1" })))
        .await;
    assert_eq!(st, StatusCode::OK);
    let (_, versions, _) = t.req("GET", &format!("/api/reports/{rid}/versions"), Who::Owner, None).await;
    let v1 = versions.as_array().unwrap().iter().find(|v| v["seq"] == 1).unwrap()["id"].as_str().unwrap().to_string();
    let (st, _, _) = t.req("POST", &format!("/api/reports/{rid}/rollback"), Who::Owner, Some(json!({ "version_id": v1 }))).await;
    assert_eq!(st, StatusCode::OK);

    let (_, versions, _) = t.req("GET", &format!("/api/reports/{rid}/versions"), Who::Owner, None).await;
    let authors: Vec<&str> = versions.as_array().unwrap().iter().map(|v| v["author"].as_str().unwrap()).collect();
    assert_eq!(authors, ["owner", "agent", "agent"]);
    let (_, v, _) = t.req("GET", &format!("/api/versions/{v1}"), Who::Owner, None).await;
    assert_eq!(v["author"], "agent");
}
