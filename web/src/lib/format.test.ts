import { describe, expect, it } from "vitest";
import { commentPosition } from "./format";
import type { Anchor } from "./types";

const index = new Map([
  ["h1", 0],
  ["p1", 1],
  ["p2", 2],
]);
const text = (block_id: string, start: number): Anchor => ({
  type: "text",
  block_id,
  start,
  end: start + 1,
  quote: "x",
  prefix: "",
  suffix: "",
});
const sorted = (anchors: Anchor[]) =>
  [...anchors].sort((a, b) => {
    const [pa, qa] = commentPosition(a, index);
    const [pb, qb] = commentPosition(b, index);
    return pa === pb ? qa - qb : pa - pb;
  });

describe("commentPosition", () => {
  it("puts document comments first", () => {
    expect(commentPosition({ type: "document" }, index)).toEqual([-1, 0]);
    const doc: Anchor = { type: "document" };
    expect(sorted([text("h1", 0), doc])[0]).toBe(doc);
  });

  it("places a section before body blocks and after earlier ones", () => {
    const section: Anchor = { type: "section", section_id: "p1" };
    const before: Anchor = { type: "block", block_id: "h1" };
    const body: Anchor = text("p1", 0);
    expect(sorted([body, section, before])).toEqual([before, section, body]);
  });

  it("orders text anchors within a block by start", () => {
    const a = text("p1", 20);
    const b = text("p1", 5);
    expect(sorted([a, b])).toEqual([b, a]);
  });

  it("sorts unknown blocks last", () => {
    const missing: Anchor = { type: "block", block_id: "gone" };
    const last: Anchor = text("p2", 99);
    expect(commentPosition(missing, index)[0]).toBe(Infinity);
    expect(sorted([missing, last])).toEqual([last, missing]);
  });
});
