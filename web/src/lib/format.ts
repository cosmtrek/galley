import type { Anchor, Block, BlockChange, BlockKind, CommentStatus } from "./types";

export const STATUS_LABEL: Record<CommentStatus, string> = {
  draft: "草稿",
  open: "等 AI 处理",
  clarify: "待澄清",
  verify: "待验证",
  resolved: "已解决",
  orphaned: "已失效",
};

export const ACTION_LABEL: Record<string, string> = {
  changed: "已修改",
  answered: "已回答",
  clarify: "有疑问",
  resolve: "已解决",
  reopen: "重新打开",
};

export const CHANGE_LABEL: Record<BlockChange["op"], string> = {
  modified: "修改",
  added: "新增",
  deleted: "删除",
  moved: "移动",
};

export const BLOCK_KIND_LABEL: Record<BlockKind, string> = {
  heading: "标题",
  paragraph: "段落",
  list_item: "列表项",
  table: "表格",
  code: "代码块",
  chart: "图表",
  diagram: "图表",
  image: "图片",
  quote: "引用",
  rule: "分隔线",
};

/** Readable one-line summary of a block; tables use their header row instead of concatenated cell text. */
export function blockSummary(b: Pick<Block, "kind" | "text" | "cells">): string {
  if (b.kind === "table" && b.cells?.length) return b.cells[0].join(" · ");
  return b.text;
}

export function anchorLabel(a: Anchor, sectionTitle?: string | null, kind?: BlockKind | null): string {
  switch (a.type) {
    case "document":
      return "整篇报告";
    case "section":
      return `章节：${sectionTitle ?? ""}`;
    case "block":
      return kind ? BLOCK_KIND_LABEL[kind] : "整段";
    case "cell":
      return `表格单元格 (${a.row + 1}, ${a.col + 1})`;
    case "text":
      return a.quote;
  }
}

export function fmtTime(ms: number | null | undefined): string {
  if (!ms) return "-";
  const d = new Date(ms);
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (d.toDateString() === now.toDateString()) return `今天 ${hm}`;
  const md = `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${hm}`;
  return d.getFullYear() === now.getFullYear() ? md : `${d.getFullYear()}-${md}`;
}

export function fmtAgo(ms: number): string {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return "刚刚";
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return fmtTime(ms);
}

/** Display-only: selections often carry stray edge whitespace, which reads badly inside 「」. */
export function truncate(s: string, n = 60): string {
  const t = s.trim();
  const chars = Array.from(t);
  return chars.length > n ? chars.slice(0, n).join("") + "…" : t;
}

/** Document-order sort key for a comment against the current version's blocks. */
export function commentPosition(a: Anchor, blockIndex: Map<string, number>): [number, number] {
  const idx = (id: string) => blockIndex.get(id) ?? Infinity;
  switch (a.type) {
    case "document":
      return [-1, 0];
    case "section":
      return [idx(a.section_id), -1];
    case "block":
    case "cell":
      return [idx(a.block_id), 0];
    case "text":
      return [idx(a.block_id), a.start];
  }
}
