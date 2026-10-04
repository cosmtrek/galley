// Mirrors the Rust API types in server/src/store.rs, diff.rs, anchor.rs.

export type Role = "owner" | "agent";
export type CommentStatus = "draft" | "open" | "clarify" | "verify" | "resolved" | "orphaned";
export type RoundStatus = "submitted" | "processing" | "verifying" | "done";
export type AnchorState = "exact" | "moved" | "fuzzy" | "orphaned";
export type BlockKind =
  | "heading"
  | "paragraph"
  | "list_item"
  | "table"
  | "code"
  | "chart"
  | "diagram"
  | "image"
  | "quote"
  | "rule";

export type Anchor =
  | { type: "document" }
  | { type: "section"; section_id: string }
  | { type: "block"; block_id: string }
  | { type: "cell"; block_id: string; row: number; col: number }
  | {
      type: "text";
      block_id: string;
      cell?: [number, number] | null;
      start: number;
      /** Set when the selection crosses blocks; `end` is then an offset into this block. */
      end_block_id?: string | null;
      end: number;
      quote: string;
      prefix: string;
      suffix: string;
    };

export interface Block {
  id: string;
  kind: BlockKind;
  section_id: string;
  level?: number;
  text: string;
  cells?: string[][];
  src_lines: [number, number];
  html: string;
}

export interface Doc {
  title: string;
  summary: string;
  blocks: Block[];
}

export interface Segment {
  op: "eq" | "ins" | "del";
  text: string;
}

export interface CellChange {
  row: number;
  col: number;
  old: string | null;
  new: string | null;
}

export interface BlockChange {
  op: "modified" | "added" | "deleted" | "moved";
  block_id: string;
  kind: BlockKind;
  section_id: string;
  after?: string | null;
  old_text?: string | null;
  new_text?: string | null;
  segments?: Segment[] | null;
  cells?: CellChange[] | null;
}

export interface ExtraChange extends BlockChange {
  /** Handled: accepted, or turned into a draft comment (`revert_comment_id`) asking the agent to undo it. */
  confirmed: boolean;
  revert_comment_id?: string | null;
}

export interface Version {
  id: string;
  report_id: string;
  seq: number;
  markdown: string;
  html: string;
  doc: Doc;
  diff: { changes: BlockChange[] } | null;
  round_id: string | null;
  note: string;
  created_at: number;
}

export interface VersionMeta {
  id: string;
  seq: number;
  round_id: string | null;
  round_seq: number | null;
  note: string;
  created_at: number;
  changes: number;
}

export interface Round {
  id: string;
  report_id: string;
  seq: number;
  status: RoundStatus;
  base_version_id: string;
  result_version_id: string | null;
  summary: string;
  extra_changes: ExtraChange[];
  submitted_at: number;
  claimed_at: number | null;
  completed_at: number | null;
  comment_count: number;
}

export interface Counts {
  draft: number;
  open: number;
  clarify: number;
  verify: number;
  resolved: number;
  orphaned: number;
}

export interface Publication {
  id: string;
  report_id: string;
  version_id: string;
  version_seq: number;
  token: string;
  path: string;
  revoked_at: number | null;
  created_at: number;
  updated_at: number;
  views: number;
  last_viewed_at: number | null;
}

export interface ReportInfo {
  id: string;
  title: string;
  summary: string;
  current_version_id: string;
  current_seq: number;
  created_at: number;
  updated_at: number;
  counts: Counts;
  round_count: number;
  active_round: Round | null;
  publication: Publication | null;
  archived_at: number | null;
}

export interface Message {
  id: number;
  author: Role;
  action: string | null;
  body: string;
  round_id: string | null;
  created_at: number;
}

export interface Comment {
  id: string;
  report_id: string;
  round_id: string | null;
  status: CommentStatus;
  body: string;
  created_version_id: string;
  resolved_version_id: string | null;
  created_at: number;
  updated_at: number;
  messages: Message[];
  anchor: Anchor;
  anchor_state: AnchorState;
  anchor_version_id: string;
  original_quote: string | null;
  section_id: string | null;
}

export interface ReviewItem {
  comment: Comment;
  base_anchor: Anchor | null;
  section_title: string | null;
  change: BlockChange | null;
  section_changes: BlockChange[];
}

export interface Review {
  round: Round;
  base_seq: number;
  result_seq: number | null;
  items: ReviewItem[];
  section_titles: Record<string, string>;
}

export interface Comparison {
  from: VersionMeta;
  to: VersionMeta;
  changes: BlockChange[];
  section_titles: Record<string, string>;
}
