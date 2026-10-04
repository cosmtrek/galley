//! Snapshot tests for the document pipeline: parse → align → diff → relocate.

use crate::align::{align, assign_fresh};
use crate::anchor::{Anchor, AnchorState, relocate};
use crate::diff::{ChangeOp, compare, word_diff};
use crate::doc::{BlockKind, Doc, parse};

fn prefix(p: &str) -> impl Fn(&str) -> String + '_ {
    move |name| format!("{p}{name}")
}

const V1: &str = r#"---
title: 2026 储能行业调研
summary: 一句话摘要
---

## 一、市场规模

2025 年全球新增装机 120 GW，同比增长 10%，主要来自中国和美国。

| 指标 | 2025 | 2026E |
| --- | --- | --- |
| 装机量 | 120 GW | 160 GW |

- 中国：政策驱动
- 美国：IRA 补贴

## 二、竞争格局

宁德时代份额第一，比亚迪第二。

这一段将被删除，因为它和上一段重复。

## 三、风险

原材料价格波动是最大风险。

> 引用：碳酸锂价格在 2024 年下跌 80%。

![图 1](assets/fig1.png)
"#;

const V2: &str = r#"---
title: 2026 储能行业调研
summary: 一句话摘要
---

## 一、市场规模

2025 年全球新增装机 120 GW，同比增长 12.3%（来源：BNEF 2026 年报），主要来自中国和美国。

| 指标 | 2025 | 2026E |
| --- | --- | --- |
| 装机量 | 120 GW | 170 GW |

- 中国：政策驱动
- 美国：IRA 补贴
- 欧洲：电价套利

## 三、风险

原材料价格波动是最大风险。

> 引用：碳酸锂价格在 2024 年下跌 80%。

## 二、竞争格局

宁德时代份额第一，比亚迪第二。

![图 1](assets/fig1.png)
"#;

fn v1() -> Doc {
    let mut d = parse(V1, &prefix("/a/r/"));
    assign_fresh(&mut d);
    d
}

/// Replaces random ids with positional names so snapshots are stable.
fn normalize(json: String, docs: &[&Doc]) -> String {
    let mut out = json;
    let mut seen: Vec<String> = Vec::new();
    for d in docs {
        for b in &d.blocks {
            if !seen.contains(&b.id) {
                seen.push(b.id.clone());
            }
        }
    }
    for (i, id) in seen.iter().enumerate() {
        out = out.replace(id.as_str(), &format!("B{i}"));
    }
    out
}

#[test]
fn parse_blocks() {
    let d = v1();
    assert_eq!(d.title, "2026 储能行业调研");
    let summary: Vec<String> = d
        .blocks
        .iter()
        .map(|b| format!("{:?} L{}-{} {:?}", b.kind, b.src_lines.0, b.src_lines.1, b.text))
        .collect();
    insta::assert_snapshot!(summary.join("\n"));
    let html = normalize(d.render_html(), &[&d]);
    insta::assert_snapshot!(html);
}

#[test]
fn block_text_matches_html() {
    let d = v1();
    let table = d.blocks.iter().find(|b| b.kind == BlockKind::Table).unwrap();
    assert_eq!(table.text, "指标20252026E装机量120 GW160 GW");
    assert_eq!(table.cell_text(1, 2), Some("160 GW"));
    let img = d.blocks.iter().find(|b| b.kind == BlockKind::Image).unwrap();
    assert!(img.html.contains("/a/r/fig1.png"), "{}", img.html);
}

#[test]
fn raw_html_is_escaped() {
    let d = parse("<script>alert(1)</script>\n\nhi <b onclick=x>there</b> [x](javascript:alert(1))\n", &prefix("/a/"));
    let html = d.blocks.iter().map(|b| b.html.clone()).collect::<Vec<_>>().join("\n");
    assert!(!html.contains("<script"), "{html}");
    assert!(!html.contains("<b"), "{html}");
    assert!(!html.contains("javascript:"), "{html}");
}

#[test]
fn align_and_diff() {
    let old = v1();
    let mut new = parse(V2, &prefix("/a/r/"));
    align(&old, &mut new);
    let diff = compare(&old, &new);
    let lines: Vec<String> = diff
        .changes
        .iter()
        .map(|c| {
            format!(
                "{:?} {} {:?} after={:?} {:?} -> {:?}",
                c.op, c.block_id, c.kind, c.after, c.old_text, c.new_text
            )
        })
        .collect();
    insta::assert_snapshot!(normalize(lines.join("\n"), &[&old, &new]));

    let ops: Vec<ChangeOp> = diff.changes.iter().map(|c| c.op).collect();
    assert!(ops.contains(&ChangeOp::Modified));
    assert!(ops.contains(&ChangeOp::Added));
    assert!(ops.contains(&ChangeOp::Deleted));
    assert!(ops.contains(&ChangeOp::Moved));

    let table = diff.changes.iter().find(|c| c.kind == BlockKind::Table).unwrap();
    let cells = table.cells.as_ref().unwrap();
    assert_eq!(cells.len(), 1);
    assert_eq!((cells[0].row, cells[0].col), (1, 2));
}

#[test]
fn reordered_renumbered_sections_keep_ids() {
    let src_old = "# T\n\n## 3.1 装机\n\n装机量持续增长，同比提升三成。\n\n## 3.2 成本\n\n电芯成本下降明显。\n\n## 3.3 价格战\n\n价格战导致毛利率承压。\n";
    let src_new = "# T\n\n## 3.1 价格战\n\n价格战导致毛利率承压。\n\n## 3.2 装机\n\n装机量持续增长，同比提升三成。\n\n## 3.3 成本\n\n电芯成本下降明显。\n";
    let mut old = parse(src_old, &prefix("/a/"));
    assign_fresh(&mut old);
    let mut new = parse(src_new, &prefix("/a/"));
    align(&old, &mut new);
    for nb in &new.blocks {
        let ob = old.blocks.iter().find(|b| b.id == nb.id).expect("id inherited");
        if nb.kind == BlockKind::Heading && nb.text != "T" {
            assert_eq!(ob.text[4..], nb.text[4..], "heading paired with wrong section");
        } else {
            assert_eq!(ob.text, nb.text);
        }
    }
}

#[test]
fn word_diff_mixed_language() {
    let segs = word_diff("同比增长 10%，主要来自 China and US.", "同比增长 12.3%，主要来自 China and the US.");
    insta::assert_debug_snapshot!(segs);
}

#[test]
fn identical_versions_have_no_changes() {
    let old = v1();
    let mut new = parse(V1, &prefix("/a/r/"));
    align(&old, &mut new);
    assert!(compare(&old, &new).changes.is_empty());
    let ids: Vec<_> = old.blocks.iter().map(|b| &b.id).collect();
    let new_ids: Vec<_> = new.blocks.iter().map(|b| &b.id).collect();
    assert_eq!(ids, new_ids);
}

fn text_anchor(doc: &Doc, needle: &str) -> Anchor {
    let b = doc.blocks.iter().find(|b| b.text.contains(needle)).unwrap();
    let byte = b.text.find(needle).unwrap();
    let start = b.text[..byte].chars().count() as u32;
    let end = start + needle.chars().count() as u32;
    Anchor::Text {
        block_id: b.id.clone(),
        cell: None,
        start,
        end_block_id: None,
        end,
        quote: needle.into(),
        prefix: String::new(),
        suffix: String::new(),
    }
    .validate(doc)
    .unwrap()
}

#[test]
fn relocate_anchors() {
    let old = v1();
    let mut new = parse(V2, &prefix("/a/r/"));
    align(&old, &mut new);

    // Unchanged text in an unchanged block.
    let a = text_anchor(&old, "原材料价格波动");
    assert_eq!(relocate(&a, &old, &new).1, AnchorState::Exact);

    // Quote still present but shifted inside a modified block.
    let a = text_anchor(&old, "主要来自中国和美国");
    let (na, st) = relocate(&a, &old, &new);
    assert_eq!(st, AnchorState::Moved);
    if let Anchor::Text { quote, .. } = &na {
        assert_eq!(quote, "主要来自中国和美国");
    }

    // The quoted number itself was rewritten: falls back to text between prefix and suffix.
    let a = text_anchor(&old, "10%");
    let (na, st) = relocate(&a, &old, &new);
    assert_eq!(st, AnchorState::Fuzzy);
    match na {
        Anchor::Text { quote, .. } => assert!(quote.starts_with("12.3%"), "{quote}"),
        other => panic!("{other:?}"),
    }

    // Block moved to another position with its section.
    let a = text_anchor(&old, "宁德时代份额第一");
    assert_ne!(relocate(&a, &old, &new).1, AnchorState::Orphaned);

    // Block deleted and text gone.
    let a = text_anchor(&old, "这一段将被删除");
    assert_eq!(relocate(&a, &old, &new).1, AnchorState::Orphaned);

    // Section and document anchors.
    let sec = old.blocks.iter().find(|b| b.text == "三、风险").unwrap().id.clone();
    let a = Anchor::Section { section_id: sec };
    assert_eq!(relocate(&a, &old, &new).1, AnchorState::Exact);
    assert_eq!(relocate(&Anchor::Document, &old, &new).1, AnchorState::Exact);
}

#[test]
fn anchor_offsets_use_code_points() {
    let mut d = parse("前缀😀生僻字𠀀之后的文字\n", &prefix("/a/"));
    assign_fresh(&mut d);
    let a = text_anchor(&d, "𠀀之后");
    match a {
        Anchor::Text { start, end, prefix, .. } => {
            assert_eq!((start, end), (6, 9));
            assert_eq!(prefix, "前缀😀生僻字");
        }
        _ => unreachable!(),
    }
    let bad = Anchor::Text {
        block_id: d.blocks[0].id.clone(),
        cell: None,
        start: 0,
        end_block_id: None,
        end: 2,
        quote: "错误".into(),
        prefix: String::new(),
        suffix: String::new(),
    };
    assert!(bad.validate(&d).is_err());
}

#[test]
fn deleted_block_quote_found_elsewhere() {
    let mut old = parse("## A\n\n第一段，包含关键句子：储能成本下降。\n\n## B\n\n别的内容。\n", &prefix("/a/"));
    assign_fresh(&mut old);
    let mut new = parse("## A\n\n## B\n\n别的内容。储能成本下降。这是一段完全重写的文字，和原段落差别很大很大很大。\n", &prefix("/a/"));
    align(&old, &mut new);
    let a = text_anchor(&old, "储能成本下降");
    let (na, st) = relocate(&a, &old, &new);
    assert_eq!(st, AnchorState::Moved);
    assert_eq!(na.block_id(), Some(new.blocks[2].id.as_str()));
}

#[test]
fn cross_block_anchor() {
    let old = v1();
    let first = old.blocks.iter().find(|b| b.text.starts_with("宁德时代")).unwrap();
    let last = old.blocks.iter().find(|b| b.text.starts_with("这一段")).unwrap();
    let a = Anchor::Text {
        block_id: first.id.clone(),
        cell: None,
        start: 9,
        end_block_id: Some(last.id.clone()),
        end: 3,
        quote: String::new(),
        prefix: String::new(),
        suffix: String::new(),
    }
    .validate(&old)
    .unwrap();
    match &a {
        Anchor::Text { quote, prefix, suffix, .. } => {
            assert_eq!(quote, "比亚迪第二。\n这一段");
            assert_eq!(prefix, "宁德时代份额第一，");
            assert!(suffix.starts_with("将被删除"), "{suffix}");
        }
        _ => unreachable!(),
    }
    assert_eq!(a.block_ids(&old), vec![first.id.clone(), last.id.clone()]);

    // Unchanged document: stays a span.
    let mut same = parse(V1, &prefix("/a/r/"));
    align(&old, &mut same);
    let (na, st) = relocate(&a, &old, &same);
    assert_eq!(st, AnchorState::Exact);
    assert_eq!(na, a);

    // The last block is deleted in V2: the comment narrows to the surviving head.
    let mut new = parse(V2, &prefix("/a/r/"));
    align(&old, &mut new);
    let (na, st) = relocate(&a, &old, &new);
    assert_eq!(st, AnchorState::Fuzzy);
    match na {
        Anchor::Text { quote, end_block_id, .. } => {
            assert_eq!(quote, "比亚迪第二。");
            assert_eq!(end_block_id, None);
        }
        other => panic!("{other:?}"),
    }

    // Reversed or same-block spans are rejected.
    let bad = Anchor::Text {
        block_id: last.id.clone(),
        cell: None,
        start: 0,
        end_block_id: Some(first.id.clone()),
        end: 2,
        quote: String::new(),
        prefix: String::new(),
        suffix: String::new(),
    };
    assert!(bad.validate(&old).is_err());
}

#[test]
fn diagrams_render_as_images() {
    use base64::Engine;
    let md = "```mermaid\nflowchart LR\n  A[开始] --> B{判断}\n  B -->|是| C[结束]\n```\n\n```svg\n<svg viewBox=\"0 0 10 10\"><script>alert(1)</script><circle r=\"4\"/></svg>\n```\n\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><rect width=\"20\" height=\"20\"/></svg>\n\n```mermaid\nnot a diagram ][\n```\n\n<div onclick=\"x\">raw</div>\n";
    let d = parse(md, &prefix("/a/"));
    let kinds: Vec<BlockKind> = d.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(kinds, [BlockKind::Diagram, BlockKind::Diagram, BlockKind::Diagram, BlockKind::Code, BlockKind::Paragraph]);
    for b in &d.blocks[..3] {
        assert!(b.html.starts_with("<figure class=\"diagram\"><img src=\"data:image/svg+xml;base64,"), "{}", b.html);
        assert_eq!(b.text, "");
        assert!(!b.sig.is_empty());
    }
    // Inline SVG is only ever decoded as an image, with the namespace browsers require.
    let b64 = d.blocks[1].html.split("base64,").nth(1).unwrap().split('"').next().unwrap();
    let svg = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b64).unwrap()).unwrap();
    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox"), "{svg}");
    assert!(!d.render_html().contains("<svg"));
    assert!(d.blocks[4].html.contains("&lt;div"), "{}", d.blocks[4].html);
}
