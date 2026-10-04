//! Snapshot tests for the document pipeline: parse → align → diff → relocate.

use crate::align::{align, assign_fresh};
use crate::anchor::{Anchor, AnchorState, relocate};
use crate::diff::{ChangeOp, compare, word_diff};
use crate::doc::{BlockKind, Doc, parse};

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
    let mut d = parse(V1, "/a/r/");
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
    let d = parse("<script>alert(1)</script>\n\nhi <b onclick=x>there</b> [x](javascript:alert(1))\n", "/a/");
    let html = d.blocks.iter().map(|b| b.html.clone()).collect::<Vec<_>>().join("\n");
    assert!(!html.contains("<script"), "{html}");
    assert!(!html.contains("<b"), "{html}");
    assert!(!html.contains("javascript:"), "{html}");
}

#[test]
fn align_and_diff() {
    let old = v1();
    let mut new = parse(V2, "/a/r/");
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
    let mut old = parse(src_old, "/a/");
    assign_fresh(&mut old);
    let mut new = parse(src_new, "/a/");
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
    let mut new = parse(V1, "/a/r/");
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
    let mut new = parse(V2, "/a/r/");
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
    let mut d = parse("前缀😀生僻字𠀀之后的文字\n", "/a/");
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
        end: 2,
        quote: "错误".into(),
        prefix: String::new(),
        suffix: String::new(),
    };
    assert!(bad.validate(&d).is_err());
}

#[test]
fn deleted_block_quote_found_elsewhere() {
    let mut old = parse("## A\n\n第一段，包含关键句子：储能成本下降。\n\n## B\n\n别的内容。\n", "/a/");
    assign_fresh(&mut old);
    let mut new = parse("## A\n\n## B\n\n别的内容。储能成本下降。这是一段完全重写的文字，和原段落差别很大很大很大。\n", "/a/");
    align(&old, &mut new);
    let a = text_anchor(&old, "储能成本下降");
    let (na, st) = relocate(&a, &old, &new);
    assert_eq!(st, AnchorState::Moved);
    assert_eq!(na.block_id(), Some(new.blocks[2].id.as_str()));
}
