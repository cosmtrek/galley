//! Public share pages. Templates only ever see published HTML and the report title.

use askama::Template;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

use crate::api::Shared;
use crate::store::{ShareLookup, day_string};

#[derive(Template)]
#[template(path = "share.html")]
struct ShareTpl<'a> {
    title: &'a str,
    summary: &'a str,
    html: &'a str,
    updated: String,
    url: String,
    toc: &'a [(String, String, u8)],
}

#[derive(Template)]
#[template(path = "status.html")]
struct StatusTpl<'a> {
    heading: &'a str,
    message: &'a str,
}

fn with_headers(status: StatusCode, body: String) -> Response {
    let mut resp = (status, Html(body)).into_response();
    let h = resp.headers_mut();
    h.insert("x-robots-tag", HeaderValue::from_static("noindex, nofollow"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; img-src 'self' data:; script-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"),
    );
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    resp
}

fn status_page(status: StatusCode, heading: &str, message: &str) -> Response {
    let body = StatusTpl { heading, message }.render().unwrap_or_else(|_| heading.to_string());
    with_headers(status, body)
}

/// An image of a published report. Revalidated on every use so revoking the link takes effect.
pub async fn asset(State(s): State<Shared>, Path((token, rest)): Path<(String, String)>) -> Response {
    let lookup = s.store().share_asset(&token, &rest);
    let mut resp = match lookup {
        Ok(ShareLookup::Found((mime, bytes))) => crate::asset_response(&mime, bytes, "private, no-cache"),
        Ok(ShareLookup::Revoked) => StatusCode::GONE.into_response(),
        Ok(ShareLookup::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("share asset: {e}");
            StatusCode::NOT_FOUND.into_response()
        }
    };
    let h = resp.headers_mut();
    h.insert("x-robots-tag", HeaderValue::from_static("noindex, nofollow"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    resp
}

pub async fn page(State(s): State<Shared>, Path(token): Path<String>) -> Response {
    let lookup = s.store().share(&token);
    match lookup {
        Ok(ShareLookup::Found(p)) => {
            let tpl = ShareTpl {
                title: &p.title,
                summary: &p.summary,
                html: &p.html,
                updated: day_string(p.updated_at),
                url: format!("{}/s/{token}", s.config.public_url.trim_end_matches('/')),
                toc: &p.toc,
            };
            match tpl.render() {
                Ok(body) => with_headers(StatusCode::OK, body),
                Err(e) => {
                    eprintln!("share render: {e}");
                    status_page(StatusCode::INTERNAL_SERVER_ERROR, "出错了", "页面暂时无法显示。")
                }
            }
        }
        Ok(ShareLookup::Revoked) => status_page(StatusCode::GONE, "链接已失效", "分享者已撤销这个链接。"),
        Ok(ShareLookup::NotFound) => status_page(StatusCode::NOT_FOUND, "页面不存在", "请检查链接是否完整。"),
        Err(e) => {
            eprintln!("share lookup: {e}");
            status_page(StatusCode::INTERNAL_SERVER_ERROR, "出错了", "页面暂时无法显示。")
        }
    }
}
