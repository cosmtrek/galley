import type { Anchor, BlockChange, CommentStatus, RoundStatus } from "./types";

export const STATUS_LABEL: Record<CommentStatus, string> = {
  draft: "未提交",
  open: "待处理",
  clarify: "待澄清",
  verify: "待验证",
  resolved: "已解决",
  orphaned: "已失效",
};

export const ROUND_LABEL: Record<RoundStatus, string> = {
  submitted: "已提交，等待 AI",
  processing: "AI 处理中",
  verifying: "待验证",
  done: "已完成",
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

export function anchorLabel(a: Anchor, sectionTitle?: string | null): string {
  switch (a.type) {
    case "document":
      return "整篇报告";
    case "section":
      return `章节：${sectionTitle ?? ""}`;
    case "block":
      return "整段";
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

export function truncate(s: string, n = 60): string {
  const chars = Array.from(s);
  return chars.length > n ? chars.slice(0, n).join("") + "…" : s;
}
