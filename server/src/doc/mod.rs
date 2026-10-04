//! Markdown parsing, the block model, and rendering.

mod parse;
mod text;

pub use parse::parse;
pub use text::html_text_content;

use serde::{Deserialize, Serialize};

pub type BlockId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Heading,
    Paragraph,
    ListItem,
    Table,
    Code,
    Chart,
    /// Mermaid or SVG rendered to an image; `sig` carries the source.
    Diagram,
    Image,
    Quote,
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListInfo {
    /// Index of the list within the document; consecutive items with the same group share a `<ul>`/`<ol>`.
    pub group: u32,
    pub ordered: bool,
    pub start: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    /// Id of the nearest preceding heading (itself for headings); empty before the first heading.
    pub section_id: BlockId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// Plain text; equals the `textContent` of the rendered element, so offsets match the DOM.
    pub text: String,
    /// Cell texts for tables, row-major, header row first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<Vec<String>>>,
    /// 1-based inclusive line range in the full Markdown source.
    pub src_lines: (u32, u32),
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<ListInfo>,
    /// Sanitized HTML of the block without the `data-block` attribute.
    pub html: String,
    /// Extra signature used for alignment when text alone is not distinctive (e.g. image src).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sig: String,
}

impl Block {
    pub fn align_key(&self) -> String {
        format!("{:?}\u{0}{}\u{0}{}", self.kind, self.sig, self.text)
    }

    pub fn cell_text(&self, row: u32, col: u32) -> Option<&str> {
        self.cells
            .as_ref()?
            .get(row as usize)?
            .get(col as usize)
            .map(|s| s.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Frontmatter {
    pub title: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Doc {
    pub title: String,
    pub summary: String,
    pub blocks: Vec<Block>,
}

impl Doc {
    pub fn block(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.blocks.iter().position(|b| b.id == id)
    }

    /// Recomputes `section_id` after ids have been assigned.
    pub fn assign_sections(&mut self) {
        let mut current = String::new();
        for b in &mut self.blocks {
            if b.kind == BlockKind::Heading {
                current = b.id.clone();
            }
            b.section_id = current.clone();
        }
    }

    /// Full document HTML with `data-block` attributes on every block element.
    pub fn render_html(&self) -> String {
        let mut out = String::new();
        let mut open_list: Option<(u32, bool)> = None;
        for b in &self.blocks {
            let group = b.list.as_ref().map(|l| l.group);
            if let Some((g, ordered)) = open_list {
                if Some(g) != group {
                    out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
                    open_list = None;
                }
            }
            if let (Some(l), None) = (&b.list, open_list) {
                if l.ordered {
                    match l.start {
                        Some(s) if s != 1 => out.push_str(&format!("<ol start=\"{s}\">\n")),
                        _ => out.push_str("<ol>\n"),
                    }
                } else {
                    out.push_str("<ul>\n");
                }
                open_list = Some((l.group, l.ordered));
            }
            out.push_str(&inject_block_attr(&b.html, &b.id, b.kind == BlockKind::Heading));
            out.push('\n');
        }
        if let Some((_, ordered)) = open_list {
            out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
        }
        out
    }
}

/// Removes workbench-only `data-block` / `data-cell` attributes for public pages.
/// Safe on rendered HTML because text content always has `"` escaped.
pub fn strip_data_attrs(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let next = [" data-block=\"", " data-cell=\""].iter().filter_map(|p| rest.find(p).map(|i| (i, p.len()))).min();
        let Some((i, plen)) = next else { break };
        out.push_str(&rest[..i]);
        let after = &rest[i + plen..];
        rest = after.find('"').map_or("", |j| &after[j + 1..]);
    }
    out.push_str(rest);
    out
}

fn inject_block_attr(html: &str, id: &str, with_dom_id: bool) -> String {
    let Some(pos) = html.find('>') else {
        return html.to_string();
    };
    let (head, tail) = html.split_at(pos);
    let head = head.trim_end_matches('/');
    let attr = if with_dom_id {
        format!(" id=\"{id}\" data-block=\"{id}\"")
    } else {
        format!(" data-block=\"{id}\"")
    };
    format!("{head}{attr}{tail}")
}

pub fn new_block_id(taken: &std::collections::HashSet<String>) -> BlockId {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    loop {
        let n: u64 = rand::random();
        let mut s = String::from("b_");
        let mut v = n;
        for _ in 0..6 {
            s.push(ALPHABET[(v % 36) as usize] as char);
            v /= 36;
        }
        if !taken.contains(&s) {
            return s;
        }
    }
}
