import { describe, expect, it } from "vitest";
import { contextOf, cpLength, cpSlice, cpToUtf16, locateQuote, trimRange, utf16ToCp } from "./anchor";

describe("code point conversion", () => {
  const text = "前缀😀生僻字𠀀之后";

  it("counts astral characters once", () => {
    expect(text.length).toBe(11);
    expect(cpLength(text)).toBe(9);
  });

  it("round-trips offsets", () => {
    for (let cp = 0; cp <= cpLength(text); cp++) {
      expect(utf16ToCp(text, cpToUtf16(text, cp))).toBe(cp);
    }
    expect(cpToUtf16(text, 3)).toBe(4); // after the emoji
    expect(utf16ToCp(text, text.indexOf("之"))).toBe(7);
  });

  it("slices by code points", () => {
    expect(cpSlice(text, 6, 9)).toBe("𠀀之后");
  });
});

describe("context", () => {
  it("builds quote, prefix and suffix", () => {
    const c = contextOf("abcdefghij", 3, 5, 2);
    expect(c).toEqual({ quote: "de", prefix: "bc", suffix: "fg" });
  });

  it("clamps at block edges", () => {
    expect(contextOf("abc", 0, 3, 32)).toEqual({ quote: "abc", prefix: "", suffix: "" });
  });
});

describe("locateQuote", () => {
  it("uses offsets when they still match", () => {
    expect(locateQuote("增长 10%，增长 10%", { start: 10, end: 13, quote: "10%" })).toEqual([10, 13]);
  });

  it("falls back to the nearest occurrence", () => {
    expect(locateQuote("新增：增长 10%，增长 10%", { start: 10, end: 13, quote: "10%" })).toEqual([13, 16]);
    expect(locateQuote("😀 10%", { start: 0, end: 3, quote: "10%" })).toEqual([2, 5]);
  });

  it("returns null when the quote is gone", () => {
    expect(locateQuote("增长 12.3%", { start: 3, end: 6, quote: "10%" })).toBeNull();
  });
});

describe("trimRange", () => {
  it("drops surrounding whitespace", () => {
    expect(trimRange("  ab \n", 0, 6)).toEqual([2, 4]);
    expect(trimRange("   ", 0, 3)).toEqual([3, 3]);
  });
});
