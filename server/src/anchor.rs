//! Comment anchors and relocating them onto a new version.

use serde::{Deserialize, Serialize};

use crate::align::char_similarity;
use crate::doc::{Block, BlockId, Doc};

pub const CONTEXT_CHARS: usize = 32;
const FUZZY_THRESHOLD: f32 = 0.75;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Anchor {
    Document,
    Section {
        section_id: BlockId,
    },
    Block {
        block_id: BlockId,
    },
    Cell {
        block_id: BlockId,
        row: u32,
        col: u32,
    },
    /// Offsets are Unicode code points within the block (or cell) text. A selection that
    /// crosses blocks sets `end_block_id`; `start` is then an offset into `block_id` and `end`
    /// an offset into `end_block_id`, and `quote` joins the covered block texts with `\n`.
    Text {
        block_id: BlockId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell: Option<(u32, u32)>,
        start: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_block_id: Option<BlockId>,
        end: u32,
        quote: String,
        #[serde(default)]
        prefix: String,
        #[serde(default)]
        suffix: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorState {
    Exact,
    Moved,
    Fuzzy,
    Orphaned,
}

impl AnchorState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AnchorState::Exact => "exact",
            AnchorState::Moved => "moved",
            AnchorState::Fuzzy => "fuzzy",
            AnchorState::Orphaned => "orphaned",
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AnchorError {
    #[error("block {0} does not exist")]
    NoBlock(String),
    #[error("section {0} does not exist")]
    NoSection(String),
    #[error("cell ({0},{1}) does not exist")]
    NoCell(u32, u32),
    #[error("offsets out of range")]
    Range,
    #[error("quote does not match the text at the given offsets")]
    Quote,
}

impl Anchor {
    pub fn block_id(&self) -> Option<&str> {
        match self {
            Anchor::Block { block_id } | Anchor::Cell { block_id, .. } | Anchor::Text { block_id, .. } => {
                Some(block_id)
            }
            _ => None,
        }
    }

    /// Every block the anchor covers in `doc`, in document order (sections excluded).
    pub fn block_ids(&self, doc: &Doc) -> Vec<String> {
        match self {
            Anchor::Text { block_id, end_block_id: Some(last), .. } => {
                match (doc.index_of(block_id), doc.index_of(last)) {
                    (Some(i), Some(j)) if i <= j => doc.blocks[i..=j].iter().map(|b| b.id.clone()).collect(),
                    _ => vec![block_id.clone(), last.clone()],
                }
            }
            a => a.block_id().map(|b| vec![b.to_string()]).unwrap_or_default(),
        }
    }

    /// Validates a freshly created anchor against a version and normalizes its context.
    pub fn validate(mut self, doc: &Doc) -> Result<Anchor, AnchorError> {
        match &mut self {
            Anchor::Document => {}
            Anchor::Section { section_id } => {
                let b = doc.block(section_id).ok_or_else(|| AnchorError::NoSection(section_id.clone()))?;
                if b.kind != crate::doc::BlockKind::Heading {
                    return Err(AnchorError::NoSection(section_id.clone()));
                }
            }
            Anchor::Block { block_id } => {
                doc.block(block_id).ok_or_else(|| AnchorError::NoBlock(block_id.clone()))?;
            }
            Anchor::Cell { block_id, row, col } => {
                let b = doc.block(block_id).ok_or_else(|| AnchorError::NoBlock(block_id.clone()))?;
                b.cell_text(*row, *col).ok_or(AnchorError::NoCell(*row, *col))?;
            }
            Anchor::Text { block_id, cell, start, end_block_id: Some(last), end, quote, prefix, suffix } => {
                let i = doc.index_of(block_id).ok_or_else(|| AnchorError::NoBlock(block_id.clone()))?;
                let j = doc.index_of(last).ok_or_else(|| AnchorError::NoBlock(last.clone()))?;
                if cell.is_some() || i >= j {
                    return Err(AnchorError::Range);
                }
                let (s, e) = (*start as usize, *end as usize);
                if s >= doc.blocks[i].text.chars().count() || e == 0 || e > doc.blocks[j].text.chars().count() {
                    return Err(AnchorError::Range);
                }
                let (q, p, sfx) = span_context(doc, i, j, s, e);
                if !quote.is_empty() && q != *quote {
                    return Err(AnchorError::Quote);
                }
                (*quote, *prefix, *suffix) = (q, p, sfx);
            }
            Anchor::Text { block_id, cell, start, end, quote, prefix, suffix, .. } => {
                let b = doc.block(block_id).ok_or_else(|| AnchorError::NoBlock(block_id.clone()))?;
                let text = target_text(b, *cell).ok_or(AnchorError::NoCell(0, 0))?;
                let chars: Vec<char> = text.chars().collect();
                let (s, e) = (*start as usize, *end as usize);
                if s >= e || e > chars.len() {
                    return Err(AnchorError::Range);
                }
                let actual: String = chars[s..e].iter().collect();
                if !quote.is_empty() && actual != *quote {
                    return Err(AnchorError::Quote);
                }
                *quote = actual;
                (*prefix, *suffix) = context(&chars, s, e);
            }
        }
        Ok(self)
    }
}

fn target_text(b: &Block, cell: Option<(u32, u32)>) -> Option<&str> {
    match cell {
        Some((r, c)) => b.cell_text(r, c),
        None => Some(&b.text),
    }
}

fn context(chars: &[char], s: usize, e: usize) -> (String, String) {
    let p0 = s.saturating_sub(CONTEXT_CHARS);
    let s1 = (e + CONTEXT_CHARS).min(chars.len());
    (chars[p0..s].iter().collect(), chars[e..s1].iter().collect())
}

/// Relocates `anchor` (valid in `old`) onto `new`, whose blocks were aligned with `old`.
pub fn relocate(anchor: &Anchor, old: &Doc, new: &Doc) -> (Anchor, AnchorState) {
    match anchor {
        Anchor::Document => (anchor.clone(), AnchorState::Exact),
        Anchor::Section { section_id } => match new.block(section_id) {
            Some(_) => (anchor.clone(), AnchorState::Exact),
            None => (anchor.clone(), AnchorState::Orphaned),
        },
        Anchor::Block { block_id } => match new.block(block_id) {
            Some(_) => (anchor.clone(), moved_or_exact(old, new, block_id)),
            None => (anchor.clone(), AnchorState::Orphaned),
        },
        Anchor::Cell { block_id, row, col } => match new.block(block_id) {
            Some(b) if b.cell_text(*row, *col).is_some() => {
                (anchor.clone(), moved_or_exact(old, new, block_id))
            }
            Some(_) => (Anchor::Block { block_id: block_id.clone() }, AnchorState::Fuzzy),
            None => (anchor.clone(), AnchorState::Orphaned),
        },
        Anchor::Text { block_id, start, end_block_id: Some(last), end, prefix, suffix, .. } => {
            relocate_span(anchor, block_id, last, *start as usize, *end as usize, prefix, suffix, old, new)
        }
        Anchor::Text { block_id, cell, start, end, quote, prefix, suffix, .. } => {
            if let Some(b) = new.block(block_id) {
                if let Some(text) = target_text(b, *cell) {
                    let chars: Vec<char> = text.chars().collect();
                    if let Some((s, e, state)) = locate(&chars, *start as usize, *end as usize, quote, prefix, suffix) {
                        let state = if state == AnchorState::Exact {
                            moved_or_exact(old, new, block_id)
                        } else {
                            state
                        };
                        return (text_anchor(block_id, *cell, &chars, s, e), state);
                    }
                }
                // The quoted text was rewritten beyond recognition; the comment still concerns this block.
                return (Anchor::Block { block_id: block_id.clone() }, AnchorState::Fuzzy);
            }
            // Block deleted: look for the quote in the same section first, then anywhere.
            let section = old.block(block_id).map(|b| b.section_id.clone()).unwrap_or_default();
            let mut candidates: Vec<&Block> = new.blocks.iter().filter(|b| b.section_id == section).collect();
            candidates.extend(new.blocks.iter().filter(|b| b.section_id != section));
            for b in candidates {
                let chars: Vec<char> = b.text.chars().collect();
                if let Some((s, e, st)) = search(&chars, None, quote, prefix, suffix) {
                    let st = if st == AnchorState::Exact { AnchorState::Moved } else { st };
                    return (text_anchor(&b.id, None, &chars, s, e), st);
                }
            }
            (anchor.clone(), AnchorState::Orphaned)
        }
    }
}

/// Relocates a cross-block anchor by finding its head (start..end of the first block) and its
/// tail (beginning..end of the last block) separately. If only one end survives, the comment
/// narrows to that block.
#[allow(clippy::too_many_arguments)]
fn relocate_span(
    anchor: &Anchor,
    first: &str,
    last: &str,
    start: usize,
    end: usize,
    prefix: &str,
    suffix: &str,
    old: &Doc,
    new: &Doc,
) -> (Anchor, AnchorState) {
    let old_chars = |id: &str| old.block(id).map(|b| b.text.chars().collect::<Vec<char>>()).unwrap_or_default();
    let head_old = old_chars(first);
    let tail_old = old_chars(last);
    let head_q: String = head_old.get(start..).unwrap_or_default().iter().collect();
    let tail_q: String = tail_old.get(..end.min(tail_old.len())).unwrap_or_default().iter().collect();

    let head = new.index_of(first).and_then(|i| {
        let chars: Vec<char> = new.blocks[i].text.chars().collect();
        let qlen = head_q.chars().count();
        locate(&chars, start, start + qlen, &head_q, prefix, "").map(|(s, _, st)| (i, s, chars.len(), st))
    });
    let tail = new.index_of(last).and_then(|j| {
        let chars: Vec<char> = new.blocks[j].text.chars().collect();
        locate(&chars, 0, tail_q.chars().count(), &tail_q, "", suffix).map(|(_, e, st)| (j, e, st))
    });

    let worst = |a: AnchorState, b: AnchorState| {
        let rank = |s: AnchorState| match s {
            AnchorState::Exact => 0,
            AnchorState::Moved => 1,
            AnchorState::Fuzzy => 2,
            AnchorState::Orphaned => 3,
        };
        if rank(a) >= rank(b) { a } else { b }
    };
    match (head, tail) {
        (Some((i, s, _, hs)), Some((j, e, ts))) if i < j && e > 0 => {
            let (quote, prefix, suffix) = span_context(new, i, j, s, e);
            let mut st = worst(hs, ts);
            if st == AnchorState::Exact {
                st = worst(moved_or_exact(old, new, first), moved_or_exact(old, new, last));
            }
            let a = Anchor::Text {
                block_id: first.to_string(),
                cell: None,
                start: s as u32,
                end_block_id: Some(last.to_string()),
                end: e as u32,
                quote,
                prefix,
                suffix,
            };
            (a, st)
        }
        (Some((i, s, len, _)), _) => {
            let chars: Vec<char> = new.blocks[i].text.chars().collect();
            (text_anchor(first, None, &chars, s, len), AnchorState::Fuzzy)
        }
        (None, Some((j, e, _))) if e > 0 => {
            let chars: Vec<char> = new.blocks[j].text.chars().collect();
            (text_anchor(last, None, &chars, 0, e), AnchorState::Fuzzy)
        }
        _ => match [first, last].into_iter().find(|id| new.block(id).is_some()) {
            Some(id) => (Anchor::Block { block_id: id.to_string() }, AnchorState::Fuzzy),
            None => (anchor.clone(), AnchorState::Orphaned),
        },
    }
}

/// Quote, prefix and suffix for a selection from `s` in block `i` to `e` in block `j`.
fn span_context(doc: &Doc, i: usize, j: usize, s: usize, e: usize) -> (String, String, String) {
    let first: Vec<char> = doc.blocks[i].text.chars().collect();
    let last: Vec<char> = doc.blocks[j].text.chars().collect();
    let mut parts: Vec<String> = Vec::with_capacity(j - i + 1);
    parts.push(first[s..].iter().collect());
    parts.extend(doc.blocks[i + 1..j].iter().map(|b| b.text.clone()));
    parts.push(last[..e].iter().collect());
    let (prefix, _) = context(&first, s, first.len());
    let (_, suffix) = context(&last, 0, e);
    (parts.join("\n"), prefix, suffix)
}

fn moved_or_exact(old: &Doc, new: &Doc, id: &str) -> AnchorState {
    let section = |d: &Doc| d.block(id).map(|b| b.section_id.clone());
    if section(old) == section(new) { AnchorState::Exact } else { AnchorState::Moved }
}

fn text_anchor(block_id: &str, cell: Option<(u32, u32)>, chars: &[char], s: usize, e: usize) -> Anchor {
    let (prefix, suffix) = context(chars, s, e);
    Anchor::Text {
        block_id: block_id.to_string(),
        cell,
        start: s as u32,
        end_block_id: None,
        end: e as u32,
        quote: chars[s..e].iter().collect(),
        prefix,
        suffix,
    }
}

fn locate(
    chars: &[char],
    start: usize,
    end: usize,
    quote: &str,
    prefix: &str,
    suffix: &str,
) -> Option<(usize, usize, AnchorState)> {
    let q: Vec<char> = quote.chars().collect();
    if end <= chars.len() && end >= start && chars[start..end] == q[..] {
        return Some((start, end, AnchorState::Exact));
    }
    search(chars, Some(start), quote, prefix, suffix).map(|(s, e, st)| {
        (s, e, if st == AnchorState::Exact { AnchorState::Moved } else { st })
    })
}

/// Finds `quote` in `chars`: exact occurrences ranked by context, then the gap between the
/// surviving prefix and suffix, then a fuzzy window match.
fn search(
    chars: &[char],
    near: Option<usize>,
    quote: &str,
    prefix: &str,
    suffix: &str,
) -> Option<(usize, usize, AnchorState)> {
    let q: Vec<char> = quote.chars().collect();
    let p: Vec<char> = prefix.chars().collect();
    let sfx: Vec<char> = suffix.chars().collect();
    if q.is_empty() {
        return None;
    }

    let occurrences = find_all(chars, &q);
    if !occurrences.is_empty() {
        let best = occurrences
            .into_iter()
            .max_by_key(|&s| {
                let ctx = common_suffix(&chars[..s], &p) + common_prefix(&chars[s + q.len()..], &sfx);
                let dist = near.map_or(0, |n| n.abs_diff(s));
                (ctx, usize::MAX - dist)
            })
            .unwrap();
        return Some((best, best + q.len(), AnchorState::Exact));
    }

    // Text between intact context: e.g. "增长 10%" → "增长 12.3%" keeps prefix and suffix.
    let ctx_len = 8;
    // An empty prefix/suffix means the quote touched the start/end of the block.
    let pk = &p[p.len().saturating_sub(ctx_len)..];
    let sk = &sfx[..sfx.len().min(ctx_len)];
    if pk.len() + sk.len() >= 4 {
        let starts: Vec<usize> = if pk.is_empty() {
            vec![0]
        } else {
            find_all(chars, pk).into_iter().map(|s| s + pk.len()).collect()
        };
        let mut best: Option<(usize, usize)> = None;
        for gap_start in starts {
            let gap_end = if sk.is_empty() {
                Some(chars.len())
            } else {
                find_all(&chars[gap_start..], sk).first().map(|off| gap_start + off)
            };
            if let Some(gap_end) = gap_end {
                if gap_end > gap_start && gap_end - gap_start <= q.len() * 3 + 20 {
                    let better = best.is_none_or(|(bs, _)| {
                        near.map_or(false, |n| n.abs_diff(gap_start) < n.abs_diff(bs))
                    });
                    if better {
                        best = Some((gap_start, gap_end));
                    }
                }
            }
        }
        if let Some((s, e)) = best {
            return Some((s, e, AnchorState::Fuzzy));
        }
    }

    // Sliding window over lengths close to the quote length.
    if q.len() < 4 || chars.len() < q.len() / 2 {
        return None;
    }
    let mut best: Option<(usize, usize, f32)> = None;
    let lens = [q.len(), q.len() * 9 / 10, q.len() * 11 / 10];
    let step = (chars.len() / 400).max(1);
    for &len in &lens {
        if len == 0 || len > chars.len() {
            continue;
        }
        let mut s = 0;
        while s + len <= chars.len() {
            let r = char_similarity(&chars[s..s + len], &q);
            if r >= FUZZY_THRESHOLD && best.is_none_or(|(_, _, br)| r > br) {
                best = Some((s, s + len, r));
            }
            s += step;
        }
    }
    best.map(|(s, e, _)| (s, e, AnchorState::Fuzzy))
}

fn find_all(hay: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    (0..=hay.len() - needle.len()).filter(|&i| hay[i..i + needle.len()] == *needle).collect()
}

fn common_suffix(a: &[char], b: &[char]) -> usize {
    a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count()
}

fn common_prefix(a: &[char], b: &[char]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}
