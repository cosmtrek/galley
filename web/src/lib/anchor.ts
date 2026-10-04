// Selection ↔ anchor conversion.
//
// The server measures offsets in Unicode code points over a block's textContent; the DOM
// measures UTF-16 code units. Everything that crosses the boundary goes through these helpers.

import type { Anchor } from "./types";

export const CONTEXT_CHARS = 32;

export type TextAnchor = Extract<Anchor, { type: "text" }>;

export function cpLength(s: string): number {
  let n = 0;
  for (const _ of s) n++;
  return n;
}

/** UTF-16 offset → code point offset within `text`. */
export function utf16ToCp(text: string, u16: number): number {
  let cp = 0;
  let i = 0;
  while (i < u16 && i < text.length) {
    const code = text.codePointAt(i)!;
    i += code > 0xffff ? 2 : 1;
    cp++;
  }
  return cp;
}

/** Code point offset → UTF-16 offset within `text`. */
export function cpToUtf16(text: string, cp: number): number {
  let i = 0;
  let n = 0;
  while (n < cp && i < text.length) {
    const code = text.codePointAt(i)!;
    i += code > 0xffff ? 2 : 1;
    n++;
  }
  return i;
}

export function cpSlice(text: string, start: number, end: number): string {
  return Array.from(text).slice(start, end).join("");
}

/** Builds quote/prefix/suffix for a code point range. */
export function contextOf(text: string, start: number, end: number, n = CONTEXT_CHARS) {
  const chars = Array.from(text);
  return {
    quote: chars.slice(start, end).join(""),
    prefix: chars.slice(Math.max(0, start - n), start).join(""),
    suffix: chars.slice(end, end + n).join(""),
  };
}

/**
 * Locates an anchor's quote in `text` (code points). Uses the stored offsets when they still
 * match, otherwise the occurrence of the quote closest to the stored start.
 */
export function locateQuote(
  text: string,
  a: { start: number; end: number; quote: string },
): [number, number] | null {
  if (!a.quote) return null;
  if (cpSlice(text, a.start, a.end) === a.quote) return [a.start, a.end];
  const qLen = cpLength(a.quote);
  let best: number | null = null;
  let from = 0;
  for (;;) {
    const idx = text.indexOf(a.quote, from);
    if (idx < 0) break;
    const cp = utf16ToCp(text, idx);
    if (best === null || Math.abs(cp - a.start) < Math.abs(best - a.start)) best = cp;
    from = idx + 1;
  }
  return best === null ? null : [best, best + qLen];
}

/** Trims whitespace off both ends of a UTF-16 range over `text`. */
export function trimRange(text: string, s: number, e: number): [number, number] {
  while (s < e && /\s/.test(text[s])) s++;
  while (e > s && /\s/.test(text[e - 1])) e--;
  return [s, e];
}

// ---------- DOM ----------

function utf16OffsetIn(scope: Node, node: Node, offset: number): number {
  const r = document.createRange();
  r.setStart(scope, 0);
  r.setEnd(node, offset);
  return r.toString().length;
}

function blockOf(node: Node | null): HTMLElement | null {
  const el = node instanceof Element ? node : node?.parentElement;
  return (el?.closest("[data-block]") as HTMLElement | null) ?? null;
}

function cellOf(node: Node | null): HTMLElement | null {
  const el = node instanceof Element ? node : node?.parentElement;
  return (el?.closest("[data-cell]") as HTMLElement | null) ?? null;
}

/** Converts the current selection into a text anchor, or null if it spans blocks or is empty. */
export function selectionToAnchor(sel: Selection | null, root: HTMLElement): TextAnchor | null {
  if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return null;
  const range = sel.getRangeAt(0);
  if (!root.contains(range.commonAncestorContainer)) return null;
  const block = blockOf(range.startContainer);
  if (!block || block !== blockOf(range.endContainer)) return null;

  const startCell = cellOf(range.startContainer);
  const endCell = cellOf(range.endContainer);
  const cellEl = startCell && startCell === endCell && block.contains(startCell) ? startCell : null;
  const scope = cellEl ?? block;
  const text = scope.textContent ?? "";

  let s = utf16OffsetIn(scope, range.startContainer, range.startOffset);
  let e = utf16OffsetIn(scope, range.endContainer, range.endOffset);
  [s, e] = trimRange(text, s, e);
  if (s >= e) return null;

  const start = utf16ToCp(text, s);
  const end = utf16ToCp(text, e);
  const cell = cellEl
    ? (cellEl.dataset.cell!.split(",").map(Number) as [number, number])
    : null;
  return { type: "text", block_id: block.dataset.block!, cell, start, end, ...contextOf(text, start, end) };
}

function pointAt(scope: Node, u16: number): [Node, number] {
  const walker = document.createTreeWalker(scope, NodeFilter.SHOW_TEXT);
  let remaining = u16;
  let last: Text | null = null;
  for (let n = walker.nextNode() as Text | null; n; n = walker.nextNode() as Text | null) {
    if (remaining <= n.length) return [n, remaining];
    remaining -= n.length;
    last = n;
  }
  return last ? [last, last.length] : [scope, 0];
}

export function scopeFor(root: HTMLElement, a: Anchor): HTMLElement | null {
  if (a.type === "document") return null;
  const id = a.type === "section" ? a.section_id : a.block_id;
  const block = root.querySelector<HTMLElement>(`[data-block="${CSS.escape(id)}"]`);
  if (!block) return null;
  const cell = a.type === "cell" ? [a.row, a.col] : a.type === "text" ? a.cell : null;
  if (cell) return block.querySelector<HTMLElement>(`[data-cell="${cell[0]},${cell[1]}"]`) ?? block;
  return block;
}

/** Builds a DOM range for a text anchor (falls back to searching the quote). */
export function anchorToRange(root: HTMLElement, a: Anchor): Range | null {
  if (a.type !== "text") return null;
  const scope = scopeFor(root, a);
  if (!scope) return null;
  const text = scope.textContent ?? "";
  const loc = locateQuote(text, a);
  if (!loc) return null;
  const r = document.createRange();
  const [sn, so] = pointAt(scope, cpToUtf16(text, loc[0]));
  const [en, eo] = pointAt(scope, cpToUtf16(text, loc[1]));
  r.setStart(sn, so);
  r.setEnd(en, eo);
  return r;
}
