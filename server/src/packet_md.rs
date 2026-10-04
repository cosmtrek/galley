//! Renders a round packet as Markdown meant to be pasted straight into an agent prompt.

use std::fmt::Write;

use crate::store::{Packet, PacketComment};

fn quote_block(out: &mut String, label: &str, text: &str) {
    let _ = writeln!(out, "{label}:");
    for line in text.lines() {
        let _ = writeln!(out, "> {line}");
    }
}

fn comment(out: &mut String, c: &PacketComment, detailed: bool) {
    let _ = writeln!(out, "### {}", c.id);
    let mut loc = String::new();
    if let Some(s) = &c.section {
        let _ = write!(loc, "章节「{s}」");
    }
    let target = match c.anchor_type.as_str() {
        "document" => "整篇报告",
        "section" => "整个章节",
        "block" => "整个段落/块",
        "cell" => "表格单元格",
        _ => "选中文字",
    };
    let _ = write!(loc, "{}{target}", if loc.is_empty() { "" } else { " · " });
    if let Some((a, b)) = c.src_lines {
        let _ = write!(loc, " · 源码第 {a}-{b} 行");
    }
    let _ = writeln!(out, "位置：{loc}\n");
    if let Some(q) = &c.quote {
        quote_block(out, "原文", q);
        out.push('\n');
    }
    let _ = writeln!(out, "评论：{}\n", c.body);
    if detailed {
        if let Some(t) = &c.block_text {
            if c.quote.as_deref() != Some(t.as_str()) {
                quote_block(out, "所在段落", t);
                out.push('\n');
            }
        }
        if let Some(t) = &c.context_before {
            quote_block(out, "前一块", t);
            out.push('\n');
        }
        if let Some(t) = &c.context_after {
            quote_block(out, "后一块", t);
            out.push('\n');
        }
    }
    let msgs: Vec<_> = c.messages.iter().filter(|m| !m.body.is_empty()).collect();
    if !msgs.is_empty() {
        let _ = writeln!(out, "对话记录：");
        for m in msgs {
            let who = match m.author {
                crate::domain::Role::Owner => "我",
                crate::domain::Role::Agent => "AI",
            };
            let _ = writeln!(out, "- {who}：{}", m.body.replace('\n', " "));
        }
        out.push('\n');
    }
}

pub fn render(p: &Packet) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Galley 第 {} 轮评论：{}\n", p.round.seq, p.report_title);
    let _ = writeln!(
        out,
        "- report_id: `{}`\n- round_id: `{}`\n- 基于版本: v{}（`{}`）\n- 状态: {}\n- 待处理评论: {} 条\n",
        p.report_id,
        p.round.id,
        p.base_seq,
        p.base_version_id,
        p.round.status.as_str(),
        p.comments.len()
    );
    out.push_str(
        "## 处理要求\n\n\
         1. 用 `GET /api/reports/{report_id}/source?format=raw`（或 MCP `galley_get_source`）取得 v{base} 的完整 Markdown，按下列评论修改。\n\
         2. 评论是一句自然语言，意图和范围都以评论原文为准：提到「全文」「所有类似的地方」等就在全文查找并处理，否则只改评论所在的位置。\n\
         3. 评论涉及事实、数据或来源时要查证，并在回复中附来源；意图不明确、无法确认或有不同意见时用 `clarify` 提问，不要硬改。\n\
         4. 不要改动已解决评论涉及的内容，避免把修好的问题改回去。\n\
         5. 评论之外尽量不做改动；如有，请在摘要中说明。\n\
         6. 一次性提交：`POST /api/rounds/{round_id}/result`（或 MCP `galley_submit_round`），包含完整新 Markdown、每条评论的回复（action 为 changed / answered / clarify）和本轮摘要。每条待处理评论都必须回复。\n\n"
            .replace("{base}", &p.base_seq.to_string())
            .as_str(),
    );
    let _ = writeln!(out, "## 待处理评论\n");
    if p.comments.is_empty() {
        let _ = writeln!(out, "（无）\n");
    }
    for c in &p.comments {
        comment(&mut out, c, true);
    }
    if !p.resolved.is_empty() {
        let _ = writeln!(out, "## 已解决评论（约束：不要改回去）\n");
        for c in &p.resolved {
            comment(&mut out, c, false);
        }
    }
    out
}
