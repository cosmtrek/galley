//! Application services on top of SQLite. Status changes always go through `domain`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::align::{align, assign_fresh};
use crate::anchor::{Anchor, AnchorState, relocate};
use crate::db::{now_ms, random_token, short_id};
use crate::diff::{BlockChange, BlockDiff, ChangeOp, compare};
use crate::doc::{BlockKind, Doc, parse};
use crate::domain::{
    CommentAction, CommentStatus, Role, RoundAction, RoundStatus, comment_transition, round_transition,
};
use crate::error::{AppError, AppResult};

pub const LEASE_MS: i64 = 30 * 60 * 1000;

pub fn asset_prefix(report_id: &str) -> String {
    format!("/a/{report_id}/")
}

pub fn share_asset_prefix(token: &str) -> String {
    format!("/s/{token}/a/")
}

// ---------- API types ----------

#[derive(Debug, Serialize, Default, Clone)]
pub struct Counts {
    pub draft: i64,
    pub open: i64,
    pub clarify: i64,
    pub verify: i64,
    pub resolved: i64,
    pub orphaned: i64,
}

#[derive(Debug, Serialize)]
pub struct ReportInfo {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub current_version_id: String,
    pub current_seq: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub counts: Counts,
    pub round_count: i64,
    pub active_round: Option<Round>,
    pub publication: Option<Publication>,
    pub archived_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Version {
    pub id: String,
    pub report_id: String,
    pub seq: i64,
    pub markdown: String,
    pub html: String,
    pub doc: Doc,
    pub diff: Option<BlockDiff>,
    pub round_id: Option<String>,
    pub note: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct VersionMeta {
    pub id: String,
    pub seq: i64,
    pub round_id: Option<String>,
    pub round_seq: Option<i64>,
    pub note: String,
    pub created_at: i64,
    pub changes: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExtraChange {
    #[serde(flatten)]
    pub change: BlockChange,
    /// Handled: either accepted, or turned into a draft comment asking the agent to undo it.
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_comment_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Round {
    pub id: String,
    pub report_id: String,
    pub seq: i64,
    pub status: RoundStatus,
    pub base_version_id: String,
    pub result_version_id: Option<String>,
    pub summary: String,
    pub extra_changes: Vec<ExtraChange>,
    pub submitted_at: i64,
    pub claimed_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub comment_count: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct Message {
    pub id: i64,
    pub author: Role,
    pub action: Option<String>,
    pub body: String,
    pub round_id: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct Comment {
    pub id: String,
    pub report_id: String,
    pub round_id: Option<String>,
    pub status: CommentStatus,
    pub body: String,
    pub created_version_id: String,
    pub resolved_version_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub messages: Vec<Message>,
    pub anchor: Anchor,
    pub anchor_state: AnchorState,
    pub anchor_version_id: String,
    /// The quote as written when the comment was created.
    pub original_quote: Option<String>,
    pub section_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Publication {
    pub id: String,
    pub report_id: String,
    pub version_id: String,
    pub version_seq: i64,
    pub token: String,
    pub path: String,
    pub revoked_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub views: i64,
    pub last_viewed_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct NewComment {
    pub body: String,
    pub anchor: Anchor,
}

#[derive(Debug, Deserialize)]
pub struct CommentPatch {
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ReplyReq {
    pub comment_id: String,
    pub action: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResultReq {
    #[serde(default)]
    pub markdown: Option<String>,
    #[serde(default)]
    pub replies: Vec<ReplyReq>,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Serialize)]
pub struct PacketComment {
    pub id: String,
    pub status: CommentStatus,
    pub body: String,
    pub anchor_type: String,
    pub section: Option<String>,
    pub quote: Option<String>,
    pub block_text: Option<String>,
    pub context_before: Option<String>,
    pub context_after: Option<String>,
    pub src_lines: Option<(u32, u32)>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
pub struct Packet {
    pub report_id: String,
    pub report_title: String,
    pub round: Round,
    pub base_version_id: String,
    pub base_seq: i64,
    pub comments: Vec<PacketComment>,
    pub resolved: Vec<PacketComment>,
}

#[derive(Debug, Serialize)]
pub struct ReviewItem {
    pub comment: Comment,
    pub base_anchor: Option<Anchor>,
    pub section_title: Option<String>,
    pub change: Option<BlockChange>,
    pub section_changes: Vec<BlockChange>,
}

#[derive(Debug, Serialize)]
pub struct Review {
    pub round: Round,
    pub base_seq: i64,
    pub result_seq: Option<i64>,
    pub items: Vec<ReviewItem>,
    pub section_titles: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub from: VersionMeta,
    pub to: VersionMeta,
    pub changes: Vec<BlockChange>,
    pub section_titles: HashMap<String, String>,
}

pub struct SharePage {
    pub title: String,
    pub summary: String,
    pub html: String,
    pub updated_at: i64,
    pub toc: Vec<(String, String, u8)>,
}

pub enum ShareLookup<T> {
    Found(T),
    Revoked,
    NotFound,
}

// ---------- Store ----------

pub struct Store {
    pub conn: Connection,
    pub assets_dir: PathBuf,
}

impl Store {
    pub fn new(conn: Connection, assets_dir: PathBuf) -> Self {
        Self { conn, assets_dir }
    }

    // ----- reports & versions -----

    pub fn create_report(&mut self, markdown: &str) -> AppResult<ReportInfo> {
        let id = format!("r_{}", random_token());
        let now = now_ms();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO reports (id, title, summary, created_at, updated_at) VALUES (?1, '', '', ?2, ?2)",
            params![id, now],
        )?;
        insert_version(&tx, &id, markdown, None, "初稿", &HashMap::new())?;
        tx.commit()?;
        self.report_info(&id)
    }

    pub fn list_reports(&self) -> AppResult<Vec<ReportInfo>> {
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM reports ORDER BY updated_at DESC")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        ids.iter().map(|id| self.report_info(id)).collect()
    }

    pub fn report_info(&self, id: &str) -> AppResult<ReportInfo> {
        let (title, summary, vid, created_at, updated_at, archived_at): (String, String, String, i64, i64, Option<i64>) =
            self.conn.query_row(
                "SELECT title, summary, current_version_id, created_at, updated_at, archived_at FROM reports WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )?;
        let seq: i64 = self.conn.query_row("SELECT seq FROM versions WHERE id = ?1", [&vid], |r| r.get(0))?;
        let mut counts = Counts::default();
        let mut stmt = self.conn.prepare("SELECT status, COUNT(*) FROM comments WHERE report_id = ?1 GROUP BY status")?;
        for row in stmt.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (s, n) = row?;
            match CommentStatus::parse(&s) {
                Some(CommentStatus::Draft) => counts.draft = n,
                Some(CommentStatus::Open) => counts.open = n,
                Some(CommentStatus::Clarify) => counts.clarify = n,
                Some(CommentStatus::Verify) => counts.verify = n,
                Some(CommentStatus::Resolved) => counts.resolved = n,
                Some(CommentStatus::Orphaned) => counts.orphaned = n,
                None => {}
            }
        }
        Ok(ReportInfo {
            id: id.to_string(),
            title,
            summary,
            current_version_id: vid,
            current_seq: seq,
            created_at,
            updated_at,
            counts,
            round_count: self.conn.query_row("SELECT COUNT(*) FROM rounds WHERE report_id = ?1", [id], |r| r.get(0))?,
            active_round: self.active_round(id)?,
            publication: self.publications(id)?.into_iter().find(|p| p.revoked_at.is_none()),
            archived_at,
        })
    }

    /// Archiving is the owner's call at any moment; a round in flight simply freezes, and whatever the agent
    /// hands back while archived is refused (see `ensure_not_archived`).
    pub fn archive(&mut self, report_id: &str) -> AppResult<ReportInfo> {
        self.conn.execute("UPDATE reports SET archived_at = ?2 WHERE id = ?1 AND archived_at IS NULL", params![report_id, now_ms()])?;
        self.report_info(report_id)
    }

    pub fn unarchive(&mut self, report_id: &str) -> AppResult<ReportInfo> {
        self.conn.execute("UPDATE reports SET archived_at = NULL WHERE id = ?1", [report_id])?;
        self.report_info(report_id)
    }

    /// Permanently removes a report and everything under it, share links included. Only archived reports can
    /// be deleted, so a stray click has to get past archiving first.
    pub fn delete_report(&mut self, report_id: &str) -> AppResult<()> {
        if self.report_info(report_id)?.archived_at.is_none() {
            return Err(AppError::Conflict("只能删除已归档的报告，请先归档".into()));
        }
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM views WHERE publication_id IN (SELECT id FROM publications WHERE report_id = ?1)", [report_id])?;
        tx.execute("DELETE FROM publications WHERE report_id = ?1", [report_id])?;
        // messages and comment_anchors cascade from comments.
        tx.execute("DELETE FROM comments WHERE report_id = ?1", [report_id])?;
        tx.execute("DELETE FROM rounds WHERE report_id = ?1", [report_id])?;
        tx.execute("DELETE FROM versions WHERE report_id = ?1", [report_id])?;
        tx.execute("DELETE FROM assets WHERE report_id = ?1", [report_id])?;
        tx.execute("DELETE FROM asset_blobs WHERE report_id = ?1", [report_id])?;
        tx.execute("DELETE FROM reports WHERE id = ?1", [report_id])?;
        tx.commit()?;
        // Blobs are only reachable through the rows just removed; a leftover directory is harmless, so don't fail on it.
        let dir = self.assets_dir.join(report_id);
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                eprintln!("delete report {report_id}: could not remove assets: {e}");
            }
        }
        Ok(())
    }

    pub fn version(&self, id: &str) -> AppResult<Version> {
        load_version(&self.conn, id)
    }

    pub fn current_version(&self, report_id: &str) -> AppResult<Version> {
        let vid: String =
            self.conn.query_row("SELECT current_version_id FROM reports WHERE id = ?1", [report_id], |r| r.get(0))?;
        self.version(&vid)
    }

    pub fn versions(&self, report_id: &str) -> AppResult<Vec<VersionMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.id, v.seq, v.round_id, r.seq, v.note, v.created_at, v.block_diff
             FROM versions v LEFT JOIN rounds r ON r.id = v.round_id
             WHERE v.report_id = ?1 ORDER BY v.seq DESC",
        )?;
        let rows = stmt.query_map([report_id], |r| {
            let diff: Option<String> = r.get(6)?;
            Ok(VersionMeta {
                id: r.get(0)?,
                seq: r.get(1)?,
                round_id: r.get(2)?,
                round_seq: r.get(3)?,
                note: r.get(4)?,
                created_at: r.get(5)?,
                changes: diff
                    .and_then(|d| serde_json::from_str::<BlockDiff>(&d).ok())
                    .map_or(0, |d| d.changes.len()),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn source(&self, report_id: &str, numbered: bool) -> AppResult<String> {
        let v = self.current_version(report_id)?;
        if !numbered {
            return Ok(v.markdown);
        }
        let width = v.markdown.lines().count().to_string().len();
        Ok(v.markdown.lines().enumerate().map(|(i, l)| format!("{:>width$}| {l}\n", i + 1)).collect())
    }

    /// Pushes a new version outside the round flow (agent revision or owner rollback).
    pub fn push_version(&mut self, report_id: &str, markdown: &str, note: &str, role: Role) -> AppResult<Version> {
        self.push_version_pinned(report_id, markdown, note, &HashMap::new(), role)
    }

    fn push_version_pinned(
        &mut self,
        report_id: &str,
        markdown: &str,
        note: &str,
        pinned: &HashMap<String, String>,
        role: Role,
    ) -> AppResult<Version> {
        ensure_not_archived(&self.conn, report_id, role)?;
        self.ensure_no_pending_round(report_id)?;
        let tx = self.conn.transaction()?;
        let (vid, relocs) = insert_version(&tx, report_id, markdown, None, note, pinned)?;
        for (cid, state) in relocs {
            if state == AnchorState::Orphaned {
                transition_comment(&tx, &cid, CommentAction::Orphan, Role::Agent)?;
            }
        }
        tx.commit()?;
        self.version(&vid)
    }

    pub fn rollback(&mut self, report_id: &str, version_id: &str) -> AppResult<Version> {
        let target = self.version(version_id)?;
        if target.report_id != report_id {
            return Err(AppError::NotFound);
        }
        // A rollback during verification would leave the review diff describing a document that is gone.
        if let Some(r) = self.active_round(report_id)? {
            return Err(round_busy(&r, "回退版本"));
        }
        // Restore the images the target was rendered with, not whatever was uploaded since.
        let pinned = asset_refs(&target.html, &asset_prefix(report_id));
        self.push_version_pinned(report_id, &target.markdown, &format!("回退到 v{}", target.seq), &pinned, Role::Owner)
    }

    pub fn compare(&self, from: &str, to: &str) -> AppResult<Comparison> {
        let a = self.version(from)?;
        let b = self.version(to)?;
        if a.report_id != b.report_id {
            return Err(AppError::BadRequest("这两个版本不属于同一份报告".into()));
        }
        let metas = self.versions(&a.report_id)?;
        let meta = |id: &str| metas.iter().find(|m| m.id == id).map(|m| VersionMeta {
            id: m.id.clone(),
            seq: m.seq,
            round_id: m.round_id.clone(),
            round_seq: m.round_seq,
            note: m.note.clone(),
            created_at: m.created_at,
            changes: m.changes,
        });
        let mut titles = section_titles(&a.doc);
        titles.extend(section_titles(&b.doc));
        Ok(Comparison {
            from: meta(from).ok_or(AppError::NotFound)?,
            to: meta(to).ok_or(AppError::NotFound)?,
            changes: compare(&a.doc, &b.doc).changes,
            section_titles: titles,
        })
    }

    pub fn add_asset(&mut self, report_id: &str, name: &str, bytes: &[u8]) -> AppResult<String> {
        ensure_not_archived(&self.conn, report_id, Role::Agent)?;
        self.conn.query_row("SELECT 1 FROM reports WHERE id = ?1", [report_id], |_| Ok(()))?;
        let valid = !name.is_empty()
            && name.len() <= 128
            && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            && !name.starts_with('.');
        if !valid {
            return Err(AppError::BadRequest("文件名只能包含字母、数字、点、下划线和连字符".into()));
        }
        let mime = mime_guess::from_path(name).first_or_octet_stream();
        if mime.type_() != "image" {
            return Err(AppError::BadRequest("只能上传图片".into()));
        }
        let sha = hex::encode(Sha256::digest(bytes));
        self.write_blob(report_id, &sha, bytes)?;
        let now = now_ms();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO asset_blobs (report_id, sha256, mime, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![report_id, sha, mime.to_string(), now],
        )?;
        tx.execute(
            "INSERT INTO assets (id, report_id, name, sha256, mime, path, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (report_id, name) DO UPDATE SET sha256 = excluded.sha256, mime = excluded.mime, path = excluded.path",
            params![short_id("a_", 10), report_id, name, sha, mime.to_string(), blob_key(report_id, &sha), now],
        )?;
        tx.commit()?;
        Ok(format!("assets/{name}"))
    }

    /// Blobs are immutable: content that is already stored is never rewritten.
    fn write_blob(&self, report_id: &str, sha: &str, bytes: &[u8]) -> AppResult<()> {
        let path = self.assets_dir.join(blob_key(report_id, sha));
        if path.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(path.parent().unwrap_or(&self.assets_dir))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Serves `<sha256>/<name>` (exact content, cacheable forever) or a bare `<name>` (the latest
    /// upload under that name, used by versions rendered before the asset existed).
    /// Returns `(mime, bytes, immutable)`.
    pub fn asset(&self, report_id: &str, rest: &str) -> AppResult<(String, Vec<u8>, bool)> {
        let (sha, immutable) = match rest.split_once('/') {
            Some((sha, _)) => (sha.to_string(), true),
            None => (
                self.conn.query_row(
                    "SELECT sha256 FROM assets WHERE report_id = ?1 AND name = ?2",
                    [report_id, rest],
                    |r| r.get(0),
                )?,
                false,
            ),
        };
        // The lookup also guarantees `sha` is a stored hash, so it is safe to use as a file name.
        let mime: String = self.conn.query_row(
            "SELECT mime FROM asset_blobs WHERE report_id = ?1 AND sha256 = ?2",
            [report_id, &sha],
            |r| r.get(0),
        )?;
        Ok((mime, std::fs::read(self.assets_dir.join(blob_key(report_id, &sha)))?, immutable))
    }

    /// Files used to live at `<report>/<name>` and were overwritten in place. Copies each one to
    /// its content-addressed path; the old file is left where it is.
    pub fn migrate_legacy_assets(&mut self) -> AppResult<usize> {
        let rows: Vec<(String, String, String, String, i64)> = self
            .conn
            .prepare("SELECT report_id, name, sha256, mime, created_at FROM assets")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<Result<_, _>>()?;
        let mut moved = 0;
        for (report, name, sha, mime, created) in rows {
            if self.assets_dir.join(blob_key(&report, &sha)).exists() {
                continue;
            }
            let Ok(bytes) = std::fs::read(self.assets_dir.join(&report).join(&name)) else { continue };
            let actual = hex::encode(Sha256::digest(&bytes));
            self.write_blob(&report, &actual, &bytes)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO asset_blobs (report_id, sha256, mime, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![report, actual, mime, created],
            )?;
            self.conn.execute(
                "UPDATE assets SET sha256 = ?3, path = ?4 WHERE report_id = ?1 AND name = ?2",
                params![report, name, actual, blob_key(&report, &actual)],
            )?;
            moved += 1;
        }
        Ok(moved)
    }

    // ----- comments -----

    pub fn comments(&self, report_id: &str, version_id: Option<&str>) -> AppResult<Vec<Comment>> {
        let version = match version_id {
            Some(v) => self.version(v)?,
            None => self.current_version(report_id)?,
        };
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM comments WHERE report_id = ?1 ORDER BY created_at")?
            .query_map([report_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut out = Vec::new();
        for id in ids {
            out.push(load_comment(&self.conn, &id, Some((&version.id, version.seq, &version.doc)))?);
        }
        sort_by_position(&mut out, &version.doc);
        Ok(out)
    }

    pub fn comment(&self, id: &str) -> AppResult<Comment> {
        load_comment(&self.conn, id, None)
    }

    pub fn create_comment(&mut self, report_id: &str, req: NewComment) -> AppResult<Comment> {
        if req.body.trim().is_empty() {
            return Err(AppError::BadRequest("评论内容不能为空".into()));
        }
        ensure_not_archived(&self.conn, report_id, Role::Owner)?;
        let v = self.current_version(report_id)?;
        let anchor = req.anchor.validate(&v.doc)?;
        let tx = self.conn.transaction()?;
        let id = insert_draft(&tx, report_id, &v.id, req.body.trim(), &anchor)?;
        touch_report(&tx, report_id)?;
        tx.commit()?;
        self.comment(&id)
    }

    pub fn update_comment(&mut self, id: &str, patch: CommentPatch) -> AppResult<Comment> {
        let tx = self.conn.transaction()?;
        ensure_not_archived(&tx, &comment_report_round(&tx, id)?.0, Role::Owner)?;
        transition_comment(&tx, id, CommentAction::Edit, Role::Owner)?;
        if let Some(b) = patch.body {
            if b.trim().is_empty() {
                return Err(AppError::BadRequest("评论内容不能为空".into()));
            }
            tx.execute("UPDATE comments SET body = ?2 WHERE id = ?1", params![id, b.trim()])?;
        }
        tx.commit()?;
        self.comment(id)
    }

    pub fn delete_comment(&mut self, id: &str) -> AppResult<()> {
        let tx = self.conn.transaction()?;
        ensure_not_archived(&tx, &comment_report_round(&tx, id)?.0, Role::Owner)?;
        transition_comment(&tx, id, CommentAction::Delete, Role::Owner)?;
        tx.commit()?;
        Ok(())
    }

    pub fn owner_message(&mut self, id: &str, body: &str) -> AppResult<Comment> {
        if body.trim().is_empty() {
            return Err(AppError::BadRequest("回复内容不能为空".into()));
        }
        let tx = self.conn.transaction()?;
        ensure_not_archived(&tx, &comment_report_round(&tx, id)?.0, Role::Owner)?;
        transition_comment(&tx, id, CommentAction::OwnerMessage, Role::Owner)?;
        insert_message(&tx, id, Role::Owner, None, body.trim(), None)?;
        tx.commit()?;
        self.comment(id)
    }

    pub fn resolve(&mut self, id: &str) -> AppResult<Comment> {
        let tx = self.conn.transaction()?;
        let (report_id, round_id) = comment_report_round(&tx, id)?;
        ensure_not_archived(&tx, &report_id, Role::Owner)?;
        transition_comment(&tx, id, CommentAction::Resolve, Role::Owner)?;
        let vid: String = tx.query_row("SELECT current_version_id FROM reports WHERE id = ?1", [&report_id], |r| r.get(0))?;
        tx.execute("UPDATE comments SET resolved_version_id = ?2 WHERE id = ?1", params![id, vid])?;
        insert_message(&tx, id, Role::Owner, Some("resolve"), "", round_id.as_deref())?;
        if let Some(r) = round_id {
            maybe_complete_round(&tx, &r)?;
        }
        tx.commit()?;
        self.comment(id)
    }

    pub fn reopen(&mut self, id: &str, body: Option<&str>) -> AppResult<Comment> {
        let tx = self.conn.transaction()?;
        let (report_id, round_id) = comment_report_round(&tx, id)?;
        ensure_not_archived(&tx, &report_id, Role::Owner)?;
        transition_comment(&tx, id, CommentAction::Reopen, Role::Owner)?;
        tx.execute("UPDATE comments SET resolved_version_id = NULL WHERE id = ?1", [id])?;
        insert_message(&tx, id, Role::Owner, Some("reopen"), body.unwrap_or("").trim(), round_id.as_deref())?;
        if let Some(r) = round_id {
            maybe_complete_round(&tx, &r)?;
        }
        tx.commit()?;
        self.comment(id)
    }

    // ----- rounds -----

    fn ensure_no_pending_round(&self, report_id: &str) -> AppResult<()> {
        match self.active_round(report_id)? {
            Some(r) if matches!(r.status, RoundStatus::Submitted | RoundStatus::Processing) => {
                Err(round_busy(&r, "推送新版本"))
            }
            _ => Ok(()),
        }
    }

    pub fn active_round(&self, report_id: &str) -> AppResult<Option<Round>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM rounds WHERE report_id = ?1 AND status != 'done' ORDER BY seq DESC LIMIT 1",
                [report_id],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| load_round(&self.conn, &id)).transpose()
    }

    pub fn rounds(&self, report_id: &str) -> AppResult<Vec<Round>> {
        let ids: Vec<String> = self
            .conn
            .prepare("SELECT id FROM rounds WHERE report_id = ?1 ORDER BY seq DESC")?
            .query_map([report_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        ids.iter().map(|id| load_round(&self.conn, id)).collect()
    }

    pub fn round(&self, id: &str) -> AppResult<Round> {
        load_round(&self.conn, id)
    }

    /// Reports with a round waiting for the agent.
    pub fn pending_rounds(&self) -> AppResult<Vec<(ReportInfo, Round)>> {
        let ids: Vec<String> = self
            .conn
            .prepare(
                "SELECT r.report_id FROM rounds r JOIN reports p ON p.id = r.report_id
                 WHERE r.status IN ('submitted', 'processing') AND p.archived_at IS NULL ORDER BY r.submitted_at",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut out = Vec::new();
        for id in ids {
            let info = self.report_info(&id)?;
            if let Some(r) = info.active_round.clone() {
                out.push((info, r));
            }
        }
        Ok(out)
    }

    pub fn submit_round(&mut self, report_id: &str) -> AppResult<Round> {
        ensure_not_archived(&self.conn, report_id, Role::Owner)?;
        if let Some(r) = self.active_round(report_id)? {
            return Err(round_busy(&r, "提交下一轮"));
        }
        let tx = self.conn.transaction()?;
        let ids: Vec<String> = tx
            .prepare("SELECT id FROM comments WHERE report_id = ?1 AND status IN ('draft', 'open') ORDER BY created_at")?
            .query_map([report_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        if ids.is_empty() {
            return Err(AppError::Conflict("没有可提交的评论：先写评论，或重新打开需要继续修改的评论".into()));
        }
        let seq: i64 =
            tx.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM rounds WHERE report_id = ?1", [report_id], |r| r.get(0))?;
        let base: String = tx.query_row("SELECT current_version_id FROM reports WHERE id = ?1", [report_id], |r| r.get(0))?;
        let rid = insert_with_id("rd_", 6, |rid| {
            tx.execute(
                "INSERT INTO rounds (id, report_id, seq, status, base_version_id, submitted_at) VALUES (?1, ?2, ?3, 'submitted', ?4, ?5)",
                params![rid, report_id, seq, base, now_ms()],
            )
        })?;
        for id in &ids {
            transition_comment(&tx, id, CommentAction::Submit, Role::Owner)?;
            tx.execute("UPDATE comments SET round_id = ?2 WHERE id = ?1", params![id, rid])?;
        }
        touch_report(&tx, report_id)?;
        tx.commit()?;
        self.round(&rid)
    }

    pub fn claim(&mut self, round_id: &str, role: Role) -> AppResult<Round> {
        let tx = self.conn.transaction()?;
        let r = load_round_raw(&tx, round_id)?;
        ensure_not_archived(&tx, &r.report_id, role)?;
        let next = round_transition(r.status, RoundAction::Claim { lease_expired: lease_expired(&r) }, role)?;
        tx.execute(
            "UPDATE rounds SET status = ?2, claimed_at = ?3 WHERE id = ?1",
            params![round_id, next.as_str(), now_ms()],
        )?;
        tx.commit()?;
        self.round(round_id)
    }

    pub fn packet(&self, round_id: &str) -> AppResult<Packet> {
        let round = self.round(round_id)?;
        let report = self.report_info(&round.report_id)?;
        let base = self.version(&round.base_version_id)?;
        let all = self.comments(&round.report_id, Some(&base.id))?;
        let to_packet = |c: &Comment| packet_comment(c, &base.doc);
        Ok(Packet {
            report_id: report.id.clone(),
            report_title: report.title.clone(),
            base_version_id: base.id.clone(),
            base_seq: base.seq,
            comments: all
                .iter()
                .filter(|c| c.round_id.as_deref() == Some(round_id) && c.status == CommentStatus::Open)
                .map(to_packet)
                .collect(),
            resolved: all.iter().filter(|c| c.status == CommentStatus::Resolved).map(to_packet).collect(),
            round,
        })
    }

    pub fn submit_result(&mut self, round_id: &str, role: Role, req: ResultReq) -> AppResult<Round> {
        let tx = self.conn.transaction()?;
        let round = load_round_raw(&tx, round_id)?;
        ensure_not_archived(&tx, &round.report_id, role)?;
        let next = round_transition(round.status, RoundAction::Result, role)?;

        let open: Vec<String> = tx
            .prepare("SELECT id FROM comments WHERE round_id = ?1 AND status = 'open'")?
            .query_map([round_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let open_set: HashSet<&str> = open.iter().map(|s| s.as_str()).collect();
        let mut seen = HashSet::new();
        for r in &req.replies {
            if !open_set.contains(r.comment_id.as_str()) {
                return Err(AppError::BadRequest(format!("{} 不是本轮待处理的评论", r.comment_id)));
            }
            if !seen.insert(r.comment_id.as_str()) {
                return Err(AppError::BadRequest(format!("{} 有重复的回复，每条评论只回复一次", r.comment_id)));
            }
            if !matches!(r.action.as_str(), "changed" | "answered" | "clarify") {
                return Err(AppError::BadRequest(format!(
                    "{} 的回复 action 只能是 changed、answered 或 clarify",
                    r.comment_id
                )));
            }
            if r.body.trim().is_empty() {
                return Err(AppError::BadRequest(format!("{} 的回复内容为空", r.comment_id)));
            }
        }
        let missing: Vec<&str> = open.iter().map(|s| s.as_str()).filter(|id| !seen.contains(id)).collect();
        if !missing.is_empty() {
            return Err(AppError::BadRequest(format!(
                "缺少这些评论的回复：{}。每条待处理评论都要有一条回复，本次提交未生效",
                missing.join(", ")
            )));
        }

        let base = load_version(&tx, &round.base_version_id)?;
        let mut result_vid = base.id.clone();
        let mut relocs = Vec::new();
        let mut diff = None;
        let mut result_doc = None;
        if let Some(md) = req.markdown.as_deref().filter(|m| m.trim() != base.markdown.trim()) {
            let note = format!("第 {} 轮修改", round.seq);
            let (vid, r) = insert_version(&tx, &round.report_id, md, Some(round_id), &note, &HashMap::new())?;
            let result = load_version(&tx, &vid)?;
            // Tables already broken in the base are the owner's content, not this result's fault.
            let pre_existing = |text: &str| base.doc.blocks.iter().any(|b| b.kind == BlockKind::Paragraph && b.text == text);
            if let Some((line, _)) = result.doc.broken_tables(&result.markdown).find(|(_, b)| !pre_existing(&b.text)) {
                // Dropping the transaction discards the version just inserted, keeping the result atomic.
                return Err(AppError::BadRequest(format!(
                    "Markdown 第 {line} 行附近的表格无法解析，会显示成一段竖线文字。检查表头、分隔行（如 | --- | --- |）和每一行的列数是否一致，修好后重新提交；本次提交未生效"
                )));
            }
            result_vid = vid;
            relocs = r;
            diff = result.diff;
            result_doc = Some(result.doc);
        }

        for r in &req.replies {
            let action = match r.action.as_str() {
                "changed" => CommentAction::AgentChanged,
                "answered" => CommentAction::AgentAnswered,
                _ => CommentAction::AgentClarify,
            };
            transition_comment(&tx, &r.comment_id, action, role)?;
            insert_message(&tx, &r.comment_id, Role::Agent, Some(&r.action), r.body.trim(), Some(round_id))?;
        }
        for (cid, state) in &relocs {
            if *state == AnchorState::Orphaned {
                transition_comment(&tx, cid, CommentAction::Orphan, Role::Agent)?;
            }
        }

        let extra = match (&diff, &result_doc) {
            (Some(d), Some(result)) => extra_changes(&tx, round_id, &base.doc, result, d)?,
            _ => Vec::new(),
        };
        tx.execute(
            "UPDATE rounds SET status = ?2, result_version_id = ?3, summary = ?4, extra_changes = ?5 WHERE id = ?1",
            params![round_id, next.as_str(), result_vid, req.summary.trim(), serde_json::to_string(&extra)?],
        )?;
        maybe_complete_round(&tx, round_id)?;
        touch_report(&tx, &round.report_id)?;
        tx.commit()?;
        self.round(round_id)
    }

    /// Confirms one extra change (or all of them); the round completes once nothing is left.
    pub fn confirm_extra(&mut self, round_id: &str, block_id: Option<&str>) -> AppResult<Round> {
        let tx = self.conn.transaction()?;
        let mut round = load_round_raw(&tx, round_id)?;
        ensure_not_archived(&tx, &round.report_id, Role::Owner)?;
        round_transition(round.status, RoundAction::ConfirmExtra, Role::Owner)?;
        if let Some(b) = block_id
            && !round.extra_changes.iter().any(|e| e.change.block_id == b)
        {
            return Err(AppError::BadRequest(format!("{b} 不是本轮评论之外的改动，请刷新页面")));
        }
        for e in &mut round.extra_changes {
            if block_id.is_none_or(|b| b == e.change.block_id) {
                e.confirmed = true;
            }
        }
        tx.execute(
            "UPDATE rounds SET extra_changes = ?2 WHERE id = ?1",
            params![round_id, serde_json::to_string(&round.extra_changes)?],
        )?;
        maybe_complete_round(&tx, round_id)?;
        tx.commit()?;
        self.round(round_id)
    }

    /// Turns unwanted extra changes into one draft comment asking the agent to undo them, and counts them as
    /// handled. Several ids come from one displayed replacement (a deleted block plus the block that took its place).
    pub fn revert_extra(&mut self, round_id: &str, block_ids: &[String], body: &str) -> AppResult<Round> {
        if body.trim().is_empty() {
            return Err(AppError::BadRequest("评论内容不能为空".into()));
        }
        let tx = self.conn.transaction()?;
        let mut round = load_round_raw(&tx, round_id)?;
        ensure_not_archived(&tx, &round.report_id, Role::Owner)?;
        round_transition(round.status, RoundAction::ConfirmExtra, Role::Owner)?;
        let picked: Vec<BlockChange> = block_ids
            .iter()
            .map(|b| {
                round
                    .extra_changes
                    .iter()
                    .find(|e| &e.change.block_id == b && !e.confirmed)
                    .map(|e| e.change.clone())
                    .ok_or_else(|| AppError::BadRequest(format!("{b} 不是本轮待确认的改动，请刷新页面")))
            })
            .collect::<AppResult<_>>()?;
        if picked.is_empty() {
            return Err(AppError::BadRequest("没有选择要改回去的改动".into()));
        }

        let base = load_version(&tx, &round.base_version_id)?;
        let vid: String =
            tx.query_row("SELECT current_version_id FROM reports WHERE id = ?1", [&round.report_id], |r| r.get(0))?;
        let current = load_version(&tx, &vid)?;
        // Anchor on a block that exists now; a deletion is pinned to the block it used to follow.
        let anchor = picked
            .iter()
            .find_map(|c| match c.op {
                ChangeOp::Deleted => c.after.as_deref(),
                _ => Some(c.block_id.as_str()),
            }
            .filter(|b| current.doc.block(b).is_some()))
            .map_or(Anchor::Document, |b| Anchor::Block { block_id: b.to_string() });
        // The agent only sees the current version, so quote the original Markdown it should restore.
        let originals: Vec<String> = picked
            .iter()
            .filter_map(|c| base.doc.block(&c.block_id))
            .map(|b| {
                let (start, end) = b.src_lines;
                let skip = start.saturating_sub(1) as usize;
                let take = (end + 1).saturating_sub(start) as usize;
                base.markdown.lines().skip(skip).take(take).collect::<Vec<_>>().join("\n")
            })
            .collect();
        let mut full = body.trim().to_string();
        if !originals.is_empty() {
            full.push_str(&format!("\n\n修改前的原文：\n\n{}", originals.join("\n\n")));
        }
        let cid = insert_draft(&tx, &round.report_id, &current.id, &full, &anchor)?;

        for e in &mut round.extra_changes {
            if block_ids.contains(&e.change.block_id) {
                e.confirmed = true;
                e.revert_comment_id = Some(cid.clone());
            }
        }
        tx.execute(
            "UPDATE rounds SET extra_changes = ?2 WHERE id = ?1",
            params![round_id, serde_json::to_string(&round.extra_changes)?],
        )?;
        maybe_complete_round(&tx, round_id)?;
        touch_report(&tx, &round.report_id)?;
        tx.commit()?;
        self.round(round_id)
    }

    pub fn review(&self, round_id: &str) -> AppResult<Review> {
        let round = self.round(round_id)?;
        let base = self.version(&round.base_version_id)?;
        let result = match &round.result_version_id {
            Some(v) => Some(self.version(v)?),
            None => None,
        };
        let target = result.as_ref().unwrap_or(&base);
        let mut titles = section_titles(&base.doc);
        titles.extend(section_titles(&target.doc));
        let diff = result.as_ref().and_then(|r| r.diff.clone()).unwrap_or_default();
        let comments = self.comments(&round.report_id, Some(&target.id))?;
        let mut items = Vec::new();
        for mut c in comments.into_iter().filter(|c| c.round_id.as_deref() == Some(round_id)) {
            let base_anchor = anchor_at(&self.conn, &c.id, &base.id)?;
            // Blocks deleted in this round have no section in the result; use where they were.
            if c.section_id.is_none() && !matches!(c.anchor, Anchor::Document) {
                c.section_id = base_anchor
                    .as_ref()
                    .and_then(|a| a.block_id())
                    .and_then(|b| base.doc.block(b))
                    .map(|b| b.section_id.clone())
                    .filter(|s| !s.is_empty());
            }
            let spans = |a: &Anchor| matches!(a, Anchor::Text { end_block_id: Some(_), .. });
            let mut change = None;
            let mut section_changes = Vec::new();
            let heading = section_heading(&c.anchor, &base.doc).or_else(|| base_anchor.as_ref().and_then(|a| section_heading(a, &base.doc)));
            if let Some(h) = heading {
                let span: HashSet<String> = base.doc.section_span(h).into_iter().chain(target.doc.section_span(h)).collect();
                section_changes = diff.changes.iter().filter(|ch| span.contains(&ch.block_id)).cloned().collect();
            } else if spans(&c.anchor) || base_anchor.as_ref().is_some_and(spans) {
                let mut ids: HashSet<String> = c.anchor.block_ids(&target.doc).into_iter().collect();
                if let Some(a) = &base_anchor {
                    ids.extend(a.block_ids(&base.doc));
                }
                section_changes = diff.changes.iter().filter(|ch| ids.contains(&ch.block_id)).cloned().collect();
            } else {
                for id in [base_anchor.as_ref().and_then(|a| a.block_id()), c.anchor.block_id()].into_iter().flatten() {
                    if let Some(ch) = diff.get(id) {
                        change = Some(ch.clone());
                        break;
                    }
                }
            }
            let section_title = c.section_id.as_ref().and_then(|s| titles.get(s).cloned());
            items.push(ReviewItem { comment: c, base_anchor, section_title, change, section_changes });
        }
        Ok(Review {
            round,
            base_seq: base.seq,
            result_seq: result.as_ref().map(|r| r.seq),
            items,
            section_titles: titles,
        })
    }

    // ----- publications -----

    pub fn publications(&self, report_id: &str) -> AppResult<Vec<Publication>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.report_id, p.version_id, v.seq, p.token, p.revoked_at, p.created_at, p.updated_at,
                    COALESCE((SELECT SUM(count) FROM views WHERE publication_id = p.id), 0),
                    (SELECT MAX(last_viewed_at) FROM views WHERE publication_id = p.id)
             FROM publications p JOIN versions v ON v.id = p.version_id
             WHERE p.report_id = ?1 ORDER BY p.revoked_at IS NOT NULL, p.created_at DESC",
        )?;
        let rows = stmt.query_map([report_id], |r| {
            let token: String = r.get(4)?;
            Ok(Publication {
                id: r.get(0)?,
                report_id: r.get(1)?,
                version_id: r.get(2)?,
                version_seq: r.get(3)?,
                path: format!("/s/{token}"),
                token,
                revoked_at: r.get(5)?,
                created_at: r.get(6)?,
                updated_at: r.get(7)?,
                views: r.get(8)?,
                last_viewed_at: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn publish(&mut self, report_id: &str, version_id: Option<&str>) -> AppResult<Publication> {
        let v = match version_id {
            Some(v) => self.version(v)?,
            None => self.current_version(report_id)?,
        };
        if v.report_id != report_id {
            return Err(AppError::NotFound);
        }
        let now = now_ms();
        let active = self.publications(report_id)?.into_iter().find(|p| p.revoked_at.is_none());
        let id = match active {
            Some(p) => {
                self.conn.execute(
                    "UPDATE publications SET version_id = ?2, updated_at = ?3 WHERE id = ?1",
                    params![p.id, v.id, now],
                )?;
                p.id
            }
            None => insert_with_id("p_", 8, |id| {
                self.conn.execute(
                    "INSERT INTO publications (id, report_id, version_id, token, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![id, report_id, v.id, random_token(), now],
                )
            })?,
        };
        self.publications(report_id)?.into_iter().find(|p| p.id == id).ok_or(AppError::NotFound)
    }

    pub fn revoke(&mut self, publication_id: &str) -> AppResult<()> {
        let n = self.conn.execute(
            "UPDATE publications SET revoked_at = ?2, updated_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            params![publication_id, now_ms()],
        )?;
        if n == 0 { Err(AppError::NotFound) } else { Ok(()) }
    }

    /// The live publication behind a share token: `(publication_id, version_id, updated_at)`.
    fn publication_by_token(&self, token: &str) -> AppResult<ShareLookup<(String, String, i64)>> {
        let row: Option<(String, String, Option<i64>, i64)> = self
            .conn
            .query_row(
                "SELECT id, version_id, revoked_at, updated_at FROM publications WHERE token = ?1",
                [token],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        Ok(match row {
            None => ShareLookup::NotFound,
            Some((_, _, Some(_), _)) => ShareLookup::Revoked,
            Some((pid, vid, None, updated_at)) => ShareLookup::Found((pid, vid, updated_at)),
        })
    }

    /// Looks up a share token and records the view. Only published HTML leaves this function.
    pub fn share(&mut self, token: &str) -> AppResult<ShareLookup<SharePage>> {
        let (pid, vid, updated_at) = match self.publication_by_token(token)? {
            ShareLookup::Found(p) => p,
            ShareLookup::Revoked => return Ok(ShareLookup::Revoked),
            ShareLookup::NotFound => return Ok(ShareLookup::NotFound),
        };
        let (report_id, html, blocks): (String, String, String) = self.conn.query_row(
            "SELECT report_id, html, blocks FROM versions WHERE id = ?1",
            [&vid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let doc: Doc = serde_json::from_str(&blocks)?;
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO views (publication_id, day, count, last_viewed_at) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (publication_id, day) DO UPDATE SET count = count + 1, last_viewed_at = excluded.last_viewed_at",
            params![pid, day_string(now), now],
        )?;
        let toc = doc
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::Heading && matches!(b.level, Some(2 | 3)))
            .map(|b| (b.id.clone(), b.text.clone(), b.level.unwrap_or(2)))
            .collect();
        // Assets go through the token so they die with the link and the report id stays private.
        // Text content always has `"` escaped, so this only rewrites attribute values.
        let html = crate::doc::strip_data_attrs(&html)
            .replace(&format!("\"{}", asset_prefix(&report_id)), &format!("\"{}", share_asset_prefix(token)));
        Ok(ShareLookup::Found(SharePage { title: doc.title, summary: doc.summary, html, updated_at, toc }))
    }

    /// Serves an asset of a share page, but only one the published version actually references.
    pub fn share_asset(&self, token: &str, rest: &str) -> AppResult<ShareLookup<(String, Vec<u8>)>> {
        let vid = match self.publication_by_token(token)? {
            ShareLookup::Found((_, vid, _)) => vid,
            ShareLookup::Revoked => return Ok(ShareLookup::Revoked),
            ShareLookup::NotFound => return Ok(ShareLookup::NotFound),
        };
        let (report_id, html): (String, String) =
            self.conn.query_row("SELECT report_id, html FROM versions WHERE id = ?1", [&vid], |r| Ok((r.get(0)?, r.get(1)?)))?;
        if !html.contains(&format!("\"{}{rest}\"", asset_prefix(&report_id))) {
            return Ok(ShareLookup::NotFound);
        }
        match self.asset(&report_id, rest) {
            Ok((mime, bytes, _)) => Ok(ShareLookup::Found((mime, bytes))),
            Err(AppError::NotFound) => Ok(ShareLookup::NotFound),
            Err(e) => Err(e),
        }
    }

    // ----- sessions -----

    pub fn create_session(&mut self) -> AppResult<String> {
        let token = random_token();
        let now = now_ms();
        self.conn.execute("DELETE FROM sessions WHERE created_at <= ?1", [now - SESSION_TTL_MS])?;
        self.conn.execute("INSERT INTO sessions (token, created_at) VALUES (?1, ?2)", params![session_key(&token), now])?;
        Ok(token)
    }

    pub fn session_valid(&self, token: &str) -> AppResult<bool> {
        let created: Option<i64> = self
            .conn
            .query_row("SELECT created_at FROM sessions WHERE token = ?1", [session_key(token)], |r| r.get(0))
            .optional()?;
        Ok(created.is_some_and(|t| now_ms() - t < SESSION_TTL_MS))
    }

    /// Records that an agent authenticated. Writes at most once a minute so the owner's "last connected"
    /// indicator doesn't cost a disk write per agent request.
    pub fn agent_seen(&mut self) -> AppResult<()> {
        let now = now_ms();
        if self.agent_last_seen()?.is_some_and(|t| now - t < 60_000) {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO kv (key, value) VALUES ('agent_last_seen', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [now.to_string()],
        )?;
        Ok(())
    }

    pub fn agent_last_seen(&self) -> AppResult<Option<i64>> {
        let v: Option<String> =
            self.conn.query_row("SELECT value FROM kv WHERE key = 'agent_last_seen'", [], |r| r.get(0)).optional()?;
        Ok(v.and_then(|s| s.parse().ok()))
    }

    pub fn delete_session(&mut self, token: &str) -> AppResult<()> {
        self.conn.execute("DELETE FROM sessions WHERE token = ?1", [session_key(token)])?;
        Ok(())
    }
}

// ---------- helpers ----------

/// Path of an uploaded file relative to the assets directory, so the data directory can move.
fn blob_key(report_id: &str, sha: &str) -> String {
    format!("{report_id}/{sha}")
}

/// The `name → sha256` asset references a rendered version was built with.
fn asset_refs(html: &str, prefix: &str) -> HashMap<String, String> {
    let open = format!("\"{prefix}");
    html.split(open.as_str())
        .skip(1)
        .filter_map(|rest| rest.split('"').next()?.split_once('/'))
        .map(|(sha, name)| (name.to_string(), sha.to_string()))
        .collect()
}

pub const SESSION_TTL_MS: i64 = 30 * 24 * 3600 * 1000;

/// Sessions are stored hashed so a leaked database does not hand out logged-in cookies.
fn session_key(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Short readable ids can collide; retries with a fresh id when the primary key is taken.
fn insert_with_id(prefix: &str, len: usize, mut insert: impl FnMut(&str) -> rusqlite::Result<usize>) -> AppResult<String> {
    for _ in 0..4 {
        let id = short_id(prefix, len);
        match insert(&id) {
            Ok(_) => return Ok(id),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err(AppError::Internal(format!("could not allocate a unique {prefix} id")))
}

fn insert_draft(tx: &Transaction, report_id: &str, version_id: &str, body: &str, anchor: &Anchor) -> AppResult<String> {
    let now = now_ms();
    let id = insert_with_id("c_", 6, |id| {
        tx.execute(
            "INSERT INTO comments (id, report_id, status, body, created_version_id, created_at, updated_at)
             VALUES (?1, ?2, 'draft', ?3, ?4, ?5, ?5)",
            params![id, report_id, body, version_id, now],
        )
    })?;
    tx.execute(
        "INSERT INTO comment_anchors (comment_id, version_id, anchor, state) VALUES (?1, ?2, ?3, 'exact')",
        params![id, version_id, serde_json::to_string(anchor)?],
    )?;
    Ok(id)
}

fn round_busy(r: &Round, what: &str) -> AppError {
    let state = match r.status {
        RoundStatus::Verifying => "还在待验证，请先在验证页处理完",
        _ => "正在等待 AI 处理，请等 AI 交回结果",
    };
    AppError::Conflict(format!("第 {} 轮{state}，再{what}", r.seq))
}

fn touch_report(tx: &Connection, report_id: &str) -> AppResult<()> {
    tx.execute("UPDATE reports SET updated_at = ?2 WHERE id = ?1", params![report_id, now_ms()])?;
    Ok(())
}

/// Archived reports are read-only for everyone. The owner is told how to continue; the agent is told its
/// work was not saved, since its result is dropped rather than queued.
fn ensure_not_archived(conn: &Connection, report_id: &str, role: Role) -> AppResult<()> {
    let archived: Option<i64> = conn.query_row("SELECT archived_at FROM reports WHERE id = ?1", [report_id], |r| r.get(0))?;
    match (archived, role) {
        (None, _) => Ok(()),
        (Some(_), Role::Owner) => Err(AppError::Conflict("报告已归档，恢复后才能继续修改".into())),
        (Some(_), Role::Agent) => Err(AppError::Conflict("报告已归档，本次提交未保存；owner 恢复报告后可以重新提交".into())),
    }
}

fn comment_report_round(conn: &Connection, id: &str) -> AppResult<(String, Option<String>)> {
    Ok(conn.query_row("SELECT report_id, round_id FROM comments WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

/// The only place comment status is written.
fn transition_comment(tx: &Transaction, id: &str, action: CommentAction, role: Role) -> AppResult<Option<CommentStatus>> {
    let status: String = tx.query_row("SELECT status FROM comments WHERE id = ?1", [id], |r| r.get(0))?;
    let from = CommentStatus::parse(&status).ok_or_else(|| AppError::Internal(format!("bad status {status}")))?;
    let next = comment_transition(from, action, role)?;
    match next {
        Some(s) => {
            tx.execute(
                "UPDATE comments SET status = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, s.as_str(), now_ms()],
            )?;
        }
        None => {
            tx.execute("DELETE FROM comments WHERE id = ?1", [id])?;
        }
    }
    Ok(next)
}

/// A verifying round is done once no comment awaits verification and every edit outside the
/// comments has been confirmed.
fn maybe_complete_round(tx: &Transaction, round_id: &str) -> AppResult<()> {
    let round = load_round_raw(tx, round_id)?;
    if round.status != RoundStatus::Verifying {
        return Ok(());
    }
    let waiting: i64 = tx.query_row(
        "SELECT COUNT(*) FROM comments WHERE round_id = ?1 AND status = 'verify'",
        [round_id],
        |r| r.get(0),
    )?;
    if waiting == 0 && round.extra_changes.iter().all(|e| e.confirmed) {
        let next = round_transition(RoundStatus::Verifying, RoundAction::Complete, Role::Owner)?;
        tx.execute(
            "UPDATE rounds SET status = ?2, completed_at = ?3 WHERE id = ?1",
            params![round_id, next.as_str(), now_ms()],
        )?;
    }
    Ok(())
}

fn insert_message(
    tx: &Connection,
    comment_id: &str,
    author: Role,
    action: Option<&str>,
    body: &str,
    round_id: Option<&str>,
) -> AppResult<()> {
    let author = match author {
        Role::Owner => "owner",
        Role::Agent => "agent",
    };
    tx.execute(
        "INSERT INTO messages (comment_id, author, action, body, round_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![comment_id, author, action, body, round_id, now_ms()],
    )?;
    Ok(())
}

/// Inserts a new version, aligning with the current one and relocating comment anchors.
fn insert_version(
    tx: &Transaction,
    report_id: &str,
    markdown: &str,
    round_id: Option<&str>,
    note: &str,
    pinned: &HashMap<String, String>,
) -> AppResult<(String, Vec<(String, AnchorState)>)> {
    if markdown.trim().is_empty() {
        return Err(AppError::BadRequest("Markdown 内容为空".into()));
    }
    let prev_id: Option<String> = tx
        .query_row("SELECT current_version_id FROM reports WHERE id = ?1", [report_id], |r| r.get(0))
        .optional()?
        .flatten();
    let prev = prev_id.as_deref().map(|id| load_version(tx, id)).transpose()?;

    let latest: HashMap<String, String> = tx
        .prepare("SELECT name, sha256 FROM assets WHERE report_id = ?1")?
        .query_map([report_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let prefix = asset_prefix(report_id);
    // Rendering the content hash into the URL is what keeps each version's images fixed.
    let asset_url = |name: &str| match pinned.get(name).or_else(|| latest.get(name)) {
        Some(sha) => format!("{prefix}{sha}/{name}"),
        None => format!("{prefix}{name}"),
    };
    let mut doc = parse(markdown, &asset_url);
    let diff = match &prev {
        Some(p) => {
            align(&p.doc, &mut doc);
            Some(compare(&p.doc, &doc))
        }
        None => {
            assign_fresh(&mut doc);
            None
        }
    };
    let html = doc.render_html();
    let seq = prev.as_ref().map_or(1, |p| p.seq + 1);
    let blocks = serde_json::to_string(&doc)?;
    let diff_json = diff.as_ref().map(serde_json::to_string).transpose()?;
    let vid = insert_with_id("v_", 8, |vid| {
        tx.execute(
            "INSERT INTO versions (id, report_id, seq, markdown, html, blocks, block_diff, round_id, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![vid, report_id, seq, markdown, html, blocks, diff_json, round_id, note, now_ms()],
        )
    })?;

    let mut relocs = Vec::new();
    if let Some(p) = &prev {
        let rows: Vec<(String, String, String)> = tx
            .prepare(
                "SELECT ca.comment_id, ca.anchor, c.status FROM comment_anchors ca
                 JOIN comments c ON c.id = ca.comment_id WHERE ca.version_id = ?1",
            )?
            .query_map([&p.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        for (cid, anchor_json, status) in rows {
            let anchor: Anchor = serde_json::from_str(&anchor_json)?;
            let (na, state) = relocate(&anchor, &p.doc, &doc);
            tx.execute(
                "INSERT INTO comment_anchors (comment_id, version_id, anchor, state) VALUES (?1, ?2, ?3, ?4)",
                params![cid, vid, serde_json::to_string(&na)?, state.as_str()],
            )?;
            if status != "resolved" {
                relocs.push((cid, state));
            }
        }
    }

    tx.execute(
        "UPDATE reports SET current_version_id = ?2, title = ?3, summary = ?4, updated_at = ?5 WHERE id = ?1",
        params![report_id, vid, doc.title, doc.summary, now_ms()],
    )?;
    Ok((vid, relocs))
}

fn load_version(conn: &Connection, id: &str) -> AppResult<Version> {
    let (report_id, seq, markdown, html, blocks, diff, round_id, note, created_at): (
        String,
        i64,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        i64,
    ) = conn.query_row(
        "SELECT report_id, seq, markdown, html, blocks, block_diff, round_id, note, created_at FROM versions WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
    )?;
    Ok(Version {
        id: id.to_string(),
        report_id,
        seq,
        markdown,
        html,
        doc: serde_json::from_str(&blocks)?,
        diff: diff.map(|d| serde_json::from_str(&d)).transpose()?,
        round_id,
        note,
        created_at,
    })
}

/// Reads a round exactly as stored. State changes must start from this, not from `load_round`.
fn load_round_raw(conn: &Connection, id: &str) -> AppResult<Round> {
    let (mut round, status, extra): (Round, String, String) = conn.query_row(
        "SELECT id, report_id, seq, status, base_version_id, result_version_id, summary, extra_changes,
                submitted_at, claimed_at, completed_at,
                (SELECT COUNT(*) FROM comments WHERE round_id = rounds.id)
         FROM rounds WHERE id = ?1",
        [id],
        |r| {
            let round = Round {
                id: r.get(0)?,
                report_id: r.get(1)?,
                seq: r.get(2)?,
                status: RoundStatus::Submitted,
                base_version_id: r.get(4)?,
                result_version_id: r.get(5)?,
                summary: r.get(6)?,
                extra_changes: Vec::new(),
                submitted_at: r.get(8)?,
                claimed_at: r.get(9)?,
                completed_at: r.get(10)?,
                comment_count: r.get(11)?,
            };
            Ok((round, r.get(3)?, r.get(7)?))
        },
    )?;
    round.status = RoundStatus::parse(&status).ok_or_else(|| AppError::Internal(format!("bad round status {status}")))?;
    // Failing loudly matters: a defaulted empty list would be written back by `confirm_extra`.
    round.extra_changes = serde_json::from_str(&extra)?;
    Ok(round)
}

fn lease_expired(round: &Round) -> bool {
    round.status == RoundStatus::Processing && round.claimed_at.is_some_and(|t| now_ms() - t > LEASE_MS)
}

/// Reads a round for display. An abandoned claim is released lazily: the round reads as
/// submitted again, while `claim` sees the stored status and re-claims it through `domain`.
fn load_round(conn: &Connection, id: &str) -> AppResult<Round> {
    let mut round = load_round_raw(conn, id)?;
    if lease_expired(&round) {
        round.status = RoundStatus::Submitted;
    }
    Ok(round)
}

fn anchor_at(conn: &Connection, comment_id: &str, version_id: &str) -> AppResult<Option<Anchor>> {
    let a: Option<String> = conn
        .query_row(
            "SELECT anchor FROM comment_anchors WHERE comment_id = ?1 AND version_id = ?2",
            [comment_id, version_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(a.map(|a| serde_json::from_str(&a)).transpose()?)
}

/// Loads a comment with its anchor at the given version (or the closest earlier one).
fn load_comment(conn: &Connection, id: &str, at: Option<(&str, i64, &Doc)>) -> AppResult<Comment> {
    let mut c = conn.query_row(
        "SELECT id, report_id, round_id, status, body, created_version_id, resolved_version_id, created_at, updated_at
         FROM comments WHERE id = ?1",
        [id],
        |r| {
            let status: String = r.get(3)?;
            Ok(Comment {
                id: r.get(0)?,
                report_id: r.get(1)?,
                round_id: r.get(2)?,
                status: CommentStatus::parse(&status).unwrap_or(CommentStatus::Open),
                body: r.get(4)?,
                created_version_id: r.get(5)?,
                resolved_version_id: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
                messages: Vec::new(),
                anchor: Anchor::Document,
                anchor_state: AnchorState::Exact,
                anchor_version_id: String::new(),
                original_quote: None,
                section_id: None,
            })
        },
    )?;
    c.messages = conn
        .prepare("SELECT id, author, action, body, round_id, created_at FROM messages WHERE comment_id = ?1 ORDER BY id")?
        .query_map([id], |r| {
            let author: String = r.get(1)?;
            Ok(Message {
                id: r.get(0)?,
                author: if author == "agent" { Role::Agent } else { Role::Owner },
                action: r.get(2)?,
                body: r.get(3)?,
                round_id: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;

    let anchors: Vec<(String, i64, String, String)> = conn
        .prepare(
            "SELECT ca.version_id, v.seq, ca.anchor, ca.state FROM comment_anchors ca
             JOIN versions v ON v.id = ca.version_id WHERE ca.comment_id = ?1 ORDER BY v.seq",
        )?
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    if let Some((_, _, a, _)) = anchors.iter().find(|(v, ..)| *v == c.created_version_id) {
        if let Ok(Anchor::Text { quote, .. }) = serde_json::from_str::<Anchor>(a) {
            c.original_quote = Some(quote);
        }
    }
    let chosen = match at {
        Some((vid, seq, _)) => anchors
            .iter()
            .find(|(v, ..)| v == vid)
            .or_else(|| anchors.iter().rev().find(|(_, s, ..)| *s <= seq))
            .or(anchors.last()),
        None => anchors.last(),
    };
    if let Some((vid, _, a, state)) = chosen {
        c.anchor = serde_json::from_str(a)?;
        c.anchor_version_id = vid.clone();
        c.anchor_state = match state.as_str() {
            "moved" => AnchorState::Moved,
            "fuzzy" => AnchorState::Fuzzy,
            "orphaned" => AnchorState::Orphaned,
            _ => AnchorState::Exact,
        };
        // An anchor carried over from an older version than requested no longer points into it.
        if let Some((target, _, _)) = at {
            if vid != target {
                c.anchor_state = AnchorState::Orphaned;
            }
        }
    }
    let doc = at.map(|(_, _, d)| d);
    c.section_id = match &c.anchor {
        Anchor::Section { section_id } => Some(section_id.clone()),
        Anchor::Document => None,
        a => doc
            .and_then(|d| a.block_id().and_then(|b| d.block(b)))
            .map(|b| b.section_id.clone())
            .filter(|s| !s.is_empty()),
    };
    Ok(c)
}

fn sort_by_position(comments: &mut [Comment], doc: &Doc) {
    let pos: HashMap<&str, usize> = doc.blocks.iter().enumerate().map(|(i, b)| (b.id.as_str(), i)).collect();
    let key = |c: &Comment| -> (usize, u32, i64) {
        match &c.anchor {
            Anchor::Document => (0, 0, c.created_at),
            Anchor::Section { section_id } => (pos.get(section_id.as_str()).map_or(usize::MAX, |p| p + 1), 0, c.created_at),
            Anchor::Text { block_id, start, .. } => {
                (pos.get(block_id.as_str()).map_or(usize::MAX, |p| p + 1), start + 1, c.created_at)
            }
            a => (a.block_id().and_then(|b| pos.get(b)).map_or(usize::MAX, |p| p + 1), 0, c.created_at),
        }
    };
    comments.sort_by_key(key);
}

fn section_titles(doc: &Doc) -> HashMap<String, String> {
    doc.blocks.iter().filter(|b| b.kind == BlockKind::Heading).map(|b| (b.id.clone(), b.text.clone())).collect()
}

fn packet_comment(c: &Comment, doc: &Doc) -> PacketComment {
    let (anchor_type, quote, block) = match &c.anchor {
        Anchor::Document => ("document", None, None),
        Anchor::Section { section_id } => ("section", None, doc.block(section_id)),
        Anchor::Block { block_id } => ("block", None, doc.block(block_id)),
        Anchor::Cell { block_id, row, col } => {
            let b = doc.block(block_id);
            ("cell", b.and_then(|b| b.cell_text(*row, *col)).map(str::to_string), b)
        }
        Anchor::Text { block_id, quote, .. } => ("text", Some(quote.clone()), doc.block(block_id)),
    };
    let last = match &c.anchor {
        Anchor::Text { end_block_id: Some(l), .. } => doc.block(l).or(block),
        _ => block,
    };
    let idx = block.and_then(|b| doc.index_of(&b.id));
    let last_idx = last.and_then(|b| doc.index_of(&b.id));
    let block_text = match (idx, last_idx) {
        (Some(i), Some(j)) if j > i => {
            Some(doc.blocks[i..=j].iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n"))
        }
        _ => block.map(|b| b.text.clone()),
    };
    let neighbor = |i: Option<usize>| i.and_then(|i| doc.blocks.get(i)).map(|b| b.text.clone());
    let section = c
        .section_id
        .as_ref()
        .and_then(|s| doc.block(s))
        .map(|b| b.text.clone());
    let src_lines = match &c.anchor {
        Anchor::Section { section_id } => doc.index_of(section_id).map(|i| {
            let level = doc.blocks[i].level.unwrap_or(2);
            let end = doc.blocks[i + 1..]
                .iter()
                .find(|b| b.kind == BlockKind::Heading && b.level.unwrap_or(2) <= level)
                .map(|b| b.src_lines.0.saturating_sub(1))
                .unwrap_or_else(|| doc.blocks.last().map_or(0, |b| b.src_lines.1));
            (doc.blocks[i].src_lines.0, end)
        }),
        _ => block.zip(last).map(|(b, l)| (b.src_lines.0, l.src_lines.1.max(b.src_lines.1))),
    };
    PacketComment {
        id: c.id.clone(),
        status: c.status,
        body: c.body.clone(),
        anchor_type: anchor_type.to_string(),
        section,
        quote,
        block_text: block_text.filter(|_| anchor_type != "section"),
        context_before: if anchor_type == "section" { None } else { neighbor(idx.and_then(|i| i.checked_sub(1))) },
        context_after: if anchor_type == "section" { None } else { neighbor(last_idx.map(|i| i + 1)) },
        src_lines,
        messages: c.messages.iter().filter(|m| !m.body.is_empty()).cloned().collect(),
    }
}

/// Changed blocks that no comment of this round covers. A section comment, or any comment anchored on a
/// heading, covers the whole section in both versions so the agent's rewrite of it isn't flagged as extra.
fn extra_changes(tx: &Transaction, round_id: &str, base: &Doc, result: &Doc, diff: &BlockDiff) -> AppResult<Vec<ExtraChange>> {
    let ids: Vec<String> = tx
        .prepare("SELECT id FROM comments WHERE round_id = ?1")?
        .query_map([round_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut covered: HashSet<String> = HashSet::new();
    for id in ids {
        let anchors: Vec<String> = tx
            .prepare("SELECT anchor FROM comment_anchors WHERE comment_id = ?1")?
            .query_map([&id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        for a in anchors {
            let anchor = serde_json::from_str::<Anchor>(&a)?;
            if let Some(h) = section_heading(&anchor, base) {
                covered.extend(base.section_span(h));
                covered.extend(result.section_span(h));
            }
            covered.extend(anchor.block_ids(base));
        }
    }
    Ok(diff
        .changes
        .iter()
        .filter(|c| !covered.contains(&c.block_id))
        .map(|c| ExtraChange { change: c.clone(), confirmed: false, revert_comment_id: None })
        .collect())
}

/// The heading whose section a comment is about: an explicit section anchor, or any anchor sitting on a heading.
fn section_heading<'a>(anchor: &'a Anchor, doc: &Doc) -> Option<&'a str> {
    match anchor {
        Anchor::Section { section_id } => Some(section_id.as_str()),
        Anchor::Document => None,
        a => a.block_id().filter(|b| doc.block(b).is_some_and(|x| x.kind == BlockKind::Heading)),
    }
}

pub fn day_string(ms: i64) -> String {
    // Civil-from-days (Howard Hinnant), UTC.
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::day_string;

    #[test]
    fn days() {
        assert_eq!(day_string(0), "1970-01-01");
        assert_eq!(day_string(1_790_000_000_000), "2026-09-21");
    }
}
