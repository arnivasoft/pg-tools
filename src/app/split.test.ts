import { describe, expect, it } from "vitest";
import { clampWidth, parseStoredWidth } from "./split";

describe("clampWidth", () => {
  it("keeps the explorer between 180px and total-240px", () => {
    expect(clampWidth(100, 1200)).toBe(180);
    expect(clampWidth(500, 1200)).toBe(500);
    expect(clampWidth(1100, 1200)).toBe(960);
    expect(clampWidth(300, 300)).toBe(180);
  });
});

describe("parseStoredWidth", () => {
  it("falls back to default for missing or invalid values", () => {
    expect(parseStoredWidth(null)).toBe(320);
    expect(parseStoredWidth("abc")).toBe(320);
    expect(parseStoredWidth("0")).toBe(320);
    expect(parseStoredWidth("450")).toBe(450);
  });
});
