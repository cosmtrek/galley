//! Partial results: an agent sends the edits it made instead of the whole new Markdown.
//!
//! Every edit is located in the base version and all of them are applied at once, so one edit never shifts
//! where another lands. Any edit that can't be located exactly makes the whole set fail.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::doc::Doc;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Edit {
    /// Markdown source to replace; must occur exactly once in the base unless `all` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    /// Replace every occurrence of `old`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
    /// Replace the whole source of this block instead of matching text; an empty `new` deletes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    pub new: String,
}

/// An edit together with where it came from, used to word error messages.
pub struct Sourced<'a> {
    pub edit: &'a Edit,
    /// The comment whose reply carries the edit; `None` for edits outside the comments.
    pub comment_id: Option<&'a str>,
    /// Position within its list, 1-based.
    pub index: usize,
}

impl Sourced<'_> {
    fn label(&self) -> String {
        match self.comment_id {
            Some(c) => format!("{c} 的第 {} 条 edit", self.index),
            None => format!("评论之外的第 {} 条 edit", self.index),
        }
    }
}

/// Applies `edits` to `base` (whose parsed form is `doc`) and returns the new Markdown. The error is a
/// message for the agent naming the edit that failed.
pub fn apply(base: &str, doc: &Doc, edits: &[Sourced]) -> Result<String, String> {
    let mut spans: Vec<(Range<usize>, &str, usize)> = Vec::new();
    for (i, s) in edits.iter().enumerate() {
        let e = s.edit;
        match (&e.old, &e.block) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(format!("{}：old 和 block 必须且只能填一个", s.label()));
            }
            (Some(old), None) => {
                if old.is_empty() {
                    return Err(format!("{}：old 不能为空", s.label()));
                }
                let found: Vec<usize> = base.match_indices(old.as_str()).map(|(at, _)| at).collect();
                match (found.len(), e.all) {
                    (0, _) => {
                        return Err(format!(
                            "{}：old 在基准版本里找不到。old 要逐字取自 Markdown 源码（评论包里的源码片段或 galley_get_source），不是渲染后的文字",
                            s.label()
                        ));
                    }
                    (n, false) if n > 1 => {
                        return Err(format!(
                            "{}：old 在基准版本里出现了 {n} 次。多带一些上下文让它只出现一次，或者设置 all: true 替换全部",
                            s.label()
                        ));
                    }
                    _ => spans.extend(found.into_iter().map(|at| (at..at + old.len(), e.new.as_str(), i))),
                }
            }
            (None, Some(id)) => {
                let b = doc.block(id).ok_or_else(|| format!("{}：基准版本里没有块 {id}", s.label()))?;
                spans.push((line_range(base, b.src_lines), e.new.trim_end_matches(['\n', '\r']), i));
            }
        }
    }
    spans.sort_by_key(|(r, ..)| (r.start, r.end));
    for w in spans.windows(2) {
        if w[1].0.start < w[0].0.end {
            let (a, b) = (&edits[w[0].2], &edits[w[1].2]);
            return Err(format!("{} 和{}改到了同一段文字，把它们合并成一条", a.label(), b.label()));
        }
    }
    let mut out = String::with_capacity(base.len());
    let mut at = 0;
    for (r, new, _) in &spans {
        out.push_str(&base[at..r.start]);
        out.push_str(new);
        at = r.end;
    }
    out.push_str(&base[at..]);
    Ok(out)
}

/// Byte range of 1-based inclusive lines, without the final line break.
fn line_range(src: &str, (first, last): (u32, u32)) -> Range<usize> {
    let starts: Vec<usize> = std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let start = starts.get(first.saturating_sub(1) as usize).copied().unwrap_or(src.len());
    let mut end = starts.get(last as usize).map_or(src.len(), |&next| next - 1);
    if end > start && src.as_bytes()[end - 1] == b'\r' {
        end -= 1;
    }
    start..end.max(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::align::assign_fresh;
    use crate::doc::parse;

    const MD: &str = "---\ntitle: t\n---\n\n## 一、市场\n\n2025 年装机 120 GW，同比增长 10%。大模型很热。\n\n<svg width=\"10\"><rect/></svg>\n\n- 大模型\n- 第二项\n\n## 二、风险\n\n原材料价格波动。\n";

    fn doc() -> Doc {
        let mut d = parse(MD, &|n: &str| n.to_string());
        assign_fresh(&mut d);
        d
    }

    fn text(old: &str, new: &str) -> Edit {
        Edit { old: Some(old.into()), all: false, block: None, new: new.into() }
    }

    fn block(id: &str, new: &str) -> Edit {
        Edit { old: None, all: false, block: Some(id.into()), new: new.into() }
    }

    fn run_on(d: &Doc, edits: &[Edit]) -> Result<String, String> {
        let sourced: Vec<Sourced> =
            edits.iter().enumerate().map(|(i, e)| Sourced { edit: e, comment_id: Some("c_1"), index: i + 1 }).collect();
        apply(MD, d, &sourced)
    }

    fn run(edits: &[Edit]) -> Result<String, String> {
        run_on(&doc(), edits)
    }

    fn id_of(d: &Doc, prefix: &str) -> String {
        d.blocks.iter().find(|b| b.text.starts_with(prefix)).unwrap().id.clone()
    }

    fn diagram(d: &Doc) -> String {
        d.blocks.iter().find(|b| b.kind == crate::doc::BlockKind::Diagram).unwrap().id.clone()
    }

    #[test]
    fn replaces_text_found_once() {
        let out = run(&[text("同比增长 10%", "同比增长 12.3%")]).unwrap();
        assert_eq!(out, MD.replace("同比增长 10%", "同比增长 12.3%"));
    }

    #[test]
    fn edits_apply_against_the_base_not_each_other() {
        // The second edit matches text the first one leaves alone; order in the list doesn't matter.
        let a = run(&[text("120 GW", "130 GW"), text("原材料", "上游原材料")]).unwrap();
        let b = run(&[text("原材料", "上游原材料"), text("120 GW", "130 GW")]).unwrap();
        assert_eq!(a, b);
        assert!(a.contains("130 GW") && a.contains("上游原材料"));
    }

    #[test]
    fn ambiguous_or_missing_text_is_rejected() {
        let err = run(&[text("大模型", "LLM")]).unwrap_err();
        assert!(err.contains("c_1 的第 1 条 edit") && err.contains("出现了 2 次"), "{err}");
        let err = run(&[text("一万吉瓦", "x")]).unwrap_err();
        assert!(err.contains("找不到"), "{err}");
        assert!(run(&[text("", "x")]).unwrap_err().contains("不能为空"));
    }

    #[test]
    fn all_replaces_every_occurrence() {
        let out = run(&[Edit { all: true, ..text("大模型", "LLM") }]).unwrap();
        assert_eq!(out, MD.replace("大模型", "LLM"));
    }

    #[test]
    fn a_block_is_replaced_or_deleted_without_quoting_it() {
        let d = doc();
        let svg = diagram(&d);
        let out = run_on(&d, &[block(&svg, "<svg width=\"20\"><circle/></svg>\n")]).unwrap();
        assert_eq!(out, MD.replace("<svg width=\"10\"><rect/></svg>", "<svg width=\"20\"><circle/></svg>"));
        let para = id_of(&d, "原材料");
        let out = run_on(&d, &[block(&para, "")]).unwrap();
        assert!(!out.contains("原材料") && out.ends_with("## 二、风险\n\n\n"), "{out:?}");
    }

    #[test]
    fn a_list_item_block_keeps_its_neighbours() {
        let d = doc();
        let item = d.blocks.iter().find(|b| b.text == "第二项").unwrap().id.clone();
        let out = run_on(&d, &[block(&item, "- 第二项，改过")]).unwrap();
        assert!(out.contains("- 大模型\n- 第二项，改过\n"), "{out:?}");
    }

    #[test]
    fn overlapping_or_malformed_edits_are_rejected() {
        let d = doc();
        let para = id_of(&d, "2025");
        let err = run_on(&d, &[block(&para, "新段落"), text("同比增长 10%", "x")]).unwrap_err();
        assert!(err.contains("改到了同一段文字"), "{err}");
        assert!(run_on(&d, &[block("b_nope", "x")]).unwrap_err().contains("没有块 b_nope"));
        let both = Edit { old: Some("x".into()), all: false, block: Some(para.clone()), new: "y".into() };
        assert!(run_on(&d, &[both]).unwrap_err().contains("只能填一个"));
        let neither = Edit { old: None, all: false, block: None, new: "y".into() };
        assert!(run_on(&d, &[neither]).unwrap_err().contains("只能填一个"));
    }
}
