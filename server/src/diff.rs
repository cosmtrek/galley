//! Revision diffs between two versions whose blocks already carry stable ids.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::doc::{Block, BlockId, BlockKind, Doc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeOp {
    Modified,
    Added,
    Deleted,
    Moved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegOp {
    Eq,
    Ins,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub op: SegOp,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellChange {
    pub row: u32,
    pub col: u32,
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockChange {
    pub op: ChangeOp,
    pub block_id: BlockId,
    pub kind: BlockKind,
    /// Section of the block in the new version (old version for deletions).
    pub section_id: BlockId,
    /// For deletions: the new-version block this one used to follow (None = document start).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<BlockId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    /// Word-level revision marks (modified, or moved with edits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<Segment>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<CellChange>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlockDiff {
    /// Only changed blocks, in new-document order with deletions placed where they used to be.
    pub changes: Vec<BlockChange>,
}

impl BlockDiff {
    pub fn changed_ids(&self) -> HashSet<&str> {
        self.changes.iter().map(|c| c.block_id.as_str()).collect()
    }

    pub fn get(&self, id: &str) -> Option<&BlockChange> {
        self.changes.iter().find(|c| c.block_id == id)
    }
}

pub fn compare(old: &Doc, new: &Doc) -> BlockDiff {
    let old_by_id: HashMap<&str, (usize, &Block)> =
        old.blocks.iter().enumerate().map(|(i, b)| (b.id.as_str(), (i, b))).collect();
    let new_ids: HashSet<&str> = new.blocks.iter().map(|b| b.id.as_str()).collect();

    // Common blocks off the longest increasing subsequence of old positions are "moved".
    let common: Vec<usize> = new
        .blocks
        .iter()
        .filter_map(|b| old_by_id.get(b.id.as_str()).map(|(i, _)| *i))
        .collect();
    let stay: HashSet<usize> = lis(&common).into_iter().map(|k| common[k]).collect();

    let mut changes = Vec::new();
    let mut deleted_emitted = HashSet::new();
    let mut emit_deleted_before = |old_limit: usize, after: Option<&str>, changes: &mut Vec<BlockChange>| {
        for (i, b) in old.blocks.iter().enumerate().take(old_limit) {
            if !new_ids.contains(b.id.as_str()) && deleted_emitted.insert(i) {
                changes.push(BlockChange {
                    op: ChangeOp::Deleted,
                    block_id: b.id.clone(),
                    kind: b.kind,
                    section_id: b.section_id.clone(),
                    after: after.map(str::to_string),
                    old_text: Some(b.text.clone()),
                    new_text: None,
                    segments: None,
                    cells: None,
                });
            }
        }
    };

    let mut prev_new: Option<&str> = None;
    for nb in &new.blocks {
        match old_by_id.get(nb.id.as_str()) {
            Some(&(oi, ob)) => {
                if stay.contains(&oi) {
                    emit_deleted_before(oi, prev_new, &mut changes);
                }
                let moved = !stay.contains(&oi);
                let edited = ob.text != nb.text || ob.sig != nb.sig || ob.kind != nb.kind;
                if moved || edited {
                    changes.push(BlockChange {
                        op: if moved { ChangeOp::Moved } else { ChangeOp::Modified },
                        block_id: nb.id.clone(),
                        kind: nb.kind,
                        section_id: nb.section_id.clone(),
                        after: None,
                        old_text: Some(ob.text.clone()),
                        new_text: Some(nb.text.clone()),
                        segments: edited.then(|| word_diff(&ob.text, &nb.text)),
                        cells: table_cells(ob, nb),
                    });
                }
            }
            None => changes.push(BlockChange {
                op: ChangeOp::Added,
                block_id: nb.id.clone(),
                kind: nb.kind,
                section_id: nb.section_id.clone(),
                after: None,
                old_text: None,
                new_text: Some(nb.text.clone()),
                segments: None,
                cells: None,
            }),
        }
        prev_new = Some(nb.id.as_str());
    }
    emit_deleted_before(old.blocks.len(), prev_new, &mut changes);
    BlockDiff { changes }
}

fn table_cells(old: &Block, new: &Block) -> Option<Vec<CellChange>> {
    let (oc, nc) = (old.cells.as_ref()?, new.cells.as_ref()?);
    let rows = oc.len().max(nc.len());
    let mut out = Vec::new();
    for r in 0..rows {
        let cols = oc.get(r).map_or(0, |x| x.len()).max(nc.get(r).map_or(0, |x| x.len()));
        for c in 0..cols {
            let o = oc.get(r).and_then(|x| x.get(c)).cloned();
            let n = nc.get(r).and_then(|x| x.get(c)).cloned();
            if o != n {
                out.push(CellChange { row: r as u32, col: c as u32, old: o, new: n });
            }
        }
    }
    Some(out)
}

/// Indices (into `seq`) of one longest strictly increasing subsequence.
fn lis(seq: &[usize]) -> Vec<usize> {
    let mut tails: Vec<usize> = Vec::new();
    let mut prev = vec![usize::MAX; seq.len()];
    for (i, &v) in seq.iter().enumerate() {
        let pos = tails.partition_point(|&t| seq[t] < v);
        if pos > 0 {
            prev[i] = tails[pos - 1];
        }
        if pos == tails.len() {
            tails.push(i);
        } else {
            tails[pos] = i;
        }
    }
    let mut out = Vec::new();
    let mut k = tails.last().copied().unwrap_or(usize::MAX);
    while k != usize::MAX {
        out.push(k);
        k = prev[k];
    }
    out.reverse();
    out
}

/// Tokens for word-level diffs: ASCII words and whitespace runs stay whole; every other
/// character (CJK, punctuation) is its own token, which gives character-level diffs for Chinese.
pub fn tokenize(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None::<(usize, u8)>;
    for (i, ch) in s.char_indices() {
        let class = if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' && start.is_some_and(|(_, c)| c == 1) {
            1
        } else if ch.is_whitespace() {
            2
        } else {
            0
        };
        match start {
            Some((_, c)) if c == class && class != 0 => {}
            Some((st, _)) => {
                out.push(&s[st..i]);
                start = Some((i, class));
            }
            None => start = Some((i, class)),
        }
    }
    if let Some((st, _)) = start {
        out.push(&s[st..]);
    }
    out
}

pub fn word_diff(old: &str, new: &str) -> Vec<Segment> {
    let a = tokenize(old);
    let b = tokenize(new);
    let ops = capture_diff_slices(Algorithm::Myers, &a, &b);
    let mut segs: Vec<Segment> = Vec::new();
    let mut push = |op: SegOp, text: String| {
        if text.is_empty() {
            return;
        }
        if let Some(last) = segs.last_mut() {
            if last.op == op {
                last.text.push_str(&text);
                return;
            }
        }
        segs.push(Segment { op, text });
    };
    for op in ops {
        match op {
            DiffOp::Equal { old_index, len, .. } => push(SegOp::Eq, a[old_index..old_index + len].concat()),
            DiffOp::Delete { old_index, old_len, .. } => {
                push(SegOp::Del, a[old_index..old_index + old_len].concat())
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                push(SegOp::Ins, b[new_index..new_index + new_len].concat())
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                push(SegOp::Del, a[old_index..old_index + old_len].concat());
                push(SegOp::Ins, b[new_index..new_index + new_len].concat());
            }
        }
    }
    segs
}
