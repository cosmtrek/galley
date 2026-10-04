//! Block alignment: gives blocks in a new version stable ids inherited from the previous version.

use std::collections::HashSet;

use similar::{Algorithm, DiffOp, capture_diff_slices, diff_ratio};

use crate::doc::{Block, BlockKind, Doc, new_block_id};

/// Paired "deleted + inserted" blocks inside the same hunk count as one modified block above this.
pub const MODIFY_THRESHOLD: f32 = 0.6;
/// Blocks matched across hunks count as moved above this.
pub const MOVE_THRESHOLD: f32 = 0.8;

/// Character-level similarity in `[0, 1]`.
pub fn similarity(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    char_similarity(&a, &b)
}

pub fn char_similarity(a: &[char], b: &[char]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let (min, max) = (a.len().min(b.len()), a.len().max(b.len()));
    if (2 * min) as f32 / ((min + max) as f32) < 0.3 {
        return 0.0;
    }
    let ops = capture_diff_slices(Algorithm::Myers, a, b);
    diff_ratio(&ops, a.len(), b.len())
}

fn block_similarity(a: &Block, b: &Block) -> f32 {
    if a.kind != b.kind {
        return 0.0;
    }
    if !a.sig.is_empty() || !b.sig.is_empty() {
        if a.sig == b.sig {
            return 1.0;
        }
        if a.text.is_empty() && b.text.is_empty() {
            return 0.0;
        }
    }
    if a.kind == BlockKind::Heading {
        // Renumbered headings ("3.3 价格战" → "3.1 价格战") should still pair.
        return similarity(strip_heading_number(&a.text), strip_heading_number(&b.text));
    }
    similarity(&a.text, &b.text)
}

fn strip_heading_number(s: &str) -> &str {
    let t = s.trim_start();
    let rest = t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '、');
    if rest.len() == t.len() { t } else { rest.trim_start() }
}

/// Assigns ids (and section ids) to `new` by aligning it with `old`.
/// Returns, for each new block, the index of the old block it inherits from.
pub fn align(old: &Doc, new: &mut Doc) -> Vec<Option<usize>> {
    let old_keys: Vec<String> = old.blocks.iter().map(|b| b.align_key()).collect();
    let new_keys: Vec<String> = new.blocks.iter().map(|b| b.align_key()).collect();
    let ops = capture_diff_slices(Algorithm::Myers, &old_keys, &new_keys);

    let mut matched: Vec<Option<usize>> = vec![None; new.blocks.len()];
    let mut old_used = vec![false; old.blocks.len()];

    // Identical blocks win over fuzzy pairing, even across hunks (e.g. moved sections),
    // so a similar-but-different sibling can't steal their id.
    for op in &ops {
        if let DiffOp::Equal { old_index, new_index, len } = *op {
            for k in 0..len {
                matched[new_index + k] = Some(old_index + k);
                old_used[old_index + k] = true;
            }
        }
    }
    for j in 0..new.blocks.len() {
        if matched[j].is_some() {
            continue;
        }
        if let Some(i) = (0..old.blocks.len()).find(|&i| !old_used[i] && old_keys[i] == new_keys[j]) {
            matched[j] = Some(i);
            old_used[i] = true;
        }
    }

    let mut hunk_old = usize::MAX..0;
    let mut hunk_new = usize::MAX..0;
    let flush = |ho: &mut std::ops::Range<usize>,
                     hn: &mut std::ops::Range<usize>,
                     matched: &mut Vec<Option<usize>>,
                     old_used: &mut Vec<bool>| {
        if ho.start < ho.end && hn.start < hn.end {
            let mut min_old = ho.start;
            for j in hn.clone() {
                if matched[j].is_some() {
                    continue;
                }
                let mut best: Option<(usize, f32)> = None;
                for i in min_old..ho.end {
                    if old_used[i] {
                        continue;
                    }
                    let s = block_similarity(&old.blocks[i], &new.blocks[j]);
                    if s > MODIFY_THRESHOLD && best.is_none_or(|(_, bs)| s > bs) {
                        best = Some((i, s));
                    }
                }
                if let Some((i, _)) = best {
                    matched[j] = Some(i);
                    old_used[i] = true;
                    min_old = i + 1;
                }
            }
        }
        *ho = usize::MAX..0;
        *hn = usize::MAX..0;
    };

    for op in &ops {
        match *op {
            DiffOp::Equal { .. } => {
                flush(&mut hunk_old, &mut hunk_new, &mut matched, &mut old_used);
            }
            _ => {
                let (_, or, nr) = op.as_tag_tuple();
                if !or.is_empty() {
                    hunk_old = hunk_old.start.min(or.start)..hunk_old.end.max(or.end);
                }
                if !nr.is_empty() {
                    hunk_new = hunk_new.start.min(nr.start)..hunk_new.end.max(nr.end);
                }
            }
        }
    }
    flush(&mut hunk_old, &mut hunk_new, &mut matched, &mut old_used);

    // Moves: unmatched new blocks against every unmatched old block.
    for j in 0..new.blocks.len() {
        if matched[j].is_some() {
            continue;
        }
        let mut best: Option<(usize, f32)> = None;
        for i in 0..old.blocks.len() {
            if old_used[i] {
                continue;
            }
            let s = block_similarity(&old.blocks[i], &new.blocks[j]);
            if s >= MOVE_THRESHOLD && best.is_none_or(|(_, bs)| s > bs) {
                best = Some((i, s));
            }
        }
        if let Some((i, _)) = best {
            matched[j] = Some(i);
            old_used[i] = true;
        }
    }

    let mut taken: HashSet<String> = old.blocks.iter().map(|b| b.id.clone()).collect();
    for (j, b) in new.blocks.iter_mut().enumerate() {
        b.id = match matched[j] {
            Some(i) => old.blocks[i].id.clone(),
            None => {
                let id = new_block_id(&taken);
                taken.insert(id.clone());
                id
            }
        };
    }
    new.assign_sections();
    matched
}

/// Assigns fresh ids to a first version.
pub fn assign_fresh(doc: &mut Doc) {
    let mut taken = HashSet::new();
    for b in &mut doc.blocks {
        let id = new_block_id(&taken);
        taken.insert(id.clone());
        b.id = id;
    }
    doc.assign_sections();
}
