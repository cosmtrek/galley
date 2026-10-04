import { describe, expect, it } from "vitest";
import { partner, withoutPartners, type DiffCtx } from "./diffctx";
import type { BlockChange } from "./types";

const ch = (op: BlockChange["op"], block_id: string, after?: string): BlockChange => ({
  op,
  block_id,
  kind: "paragraph",
  section_id: "",
  after,
});

describe("replacement pairs", () => {
  const added = ch("added", "p_new");
  const deleted = ch("deleted", "t_old", "p_new");
  const lone = ch("deleted", "x", "p_other");
  const ctx: DiffCtx = { old: new Map(), new: new Map(), changes: [added, deleted, ch("modified", "p_other"), lone] };

  it("pairs an addition with the deletion that follows it", () => {
    expect(partner(ctx, deleted)).toBe(added);
    expect(partner(ctx, added)).toBe(deleted);
    expect(partner(ctx, lone)).toBeNull();
    expect(partner(null, deleted)).toBeNull();
  });

  it("hides the added half only when its partner is listed too", () => {
    expect(withoutPartners(ctx, ctx.changes).map((c) => c.block_id)).toEqual(["t_old", "p_other", "x"]);
    expect(withoutPartners(ctx, [added])).toEqual([added]);
  });
});
