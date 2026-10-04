import { get } from "./api";
import type { Block, BlockChange, BlockKind, Version } from "./types";

/** The two versions a diff was computed between, so changes can be shown rendered rather than as flat text. */
export interface DiffCtx {
  old: Map<string, Block>;
  new: Map<string, Block>;
  changes: BlockChange[];
}

const versionCache = new Map<string, Promise<Version>>();

/** Versions are immutable, so each one is fetched at most once per page load. */
export function loadVersion(id: string): Promise<Version> {
  let v = versionCache.get(id);
  if (!v) {
    v = get<Version>(`/api/versions/${id}`);
    v.catch(() => versionCache.delete(id));
    versionCache.set(id, v);
  }
  return v;
}

export async function diffCtx(oldId: string, newId: string, changes?: BlockChange[]): Promise<DiffCtx> {
  const [a, b] = await Promise.all([loadVersion(oldId), loadVersion(newId)]);
  const blocks = (v: Version) => new Map(v.doc.blocks.map((x) => [x.id, x]));
  return { old: blocks(a), new: blocks(b), changes: changes ?? b.diff?.changes ?? [] };
}

/**
 * The diff emits a deletion right after the new block it used to follow, so "added A, then deleted D with
 * D.after === A" means A took D's place. Returns the other half of such a replacement pair.
 */
export function partner(ctx: DiffCtx | null, ch: BlockChange): BlockChange | null {
  if (!ctx) return null;
  const i = ctx.changes.findIndex((c) => c.block_id === ch.block_id && c.op === ch.op);
  if (i < 0) return null;
  if (ch.op === "deleted") {
    const prev = ctx.changes[i - 1];
    return prev?.op === "added" && ch.after === prev.block_id ? prev : null;
  }
  if (ch.op === "added") {
    const next = ctx.changes[i + 1];
    return next?.op === "deleted" && next.after === ch.block_id ? next : null;
  }
  return null;
}

/** Kinds whose plain text says little about what changed; these are shown rendered. */
export const RICH_KINDS: ReadonlySet<BlockKind> = new Set(["table", "chart", "diagram", "image"]);

/** Drops the added half of each replacement; the deleted half renders the pair. */
export function withoutPartners<T extends BlockChange>(ctx: DiffCtx | null, list: T[]): T[] {
  return list.filter((c) => {
    if (c.op !== "added") return true;
    const p = partner(ctx, c);
    return !p || !list.some((x) => x.block_id === p.block_id && x.op === p.op);
  });
}
