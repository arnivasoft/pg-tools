import { describe, expect, it } from "vitest";
import type { ImportItem } from "../../ipc/bindings/ImportItem";
import { canImport, defaultSelection, groupLabel, summarize, UNGROUPED } from "./import-model";

const item = (index: number, over: Partial<ImportItem> = {}): ImportItem => ({
  index, name: `s${index}`, group: null, host: "h", port: 5432, username: "u", duplicate: false, warnings: [], error: null, ...over,
});

describe("import model", () => {
  it("selects valid non-duplicate items by default", () => {
    const items = [item(0), item(1, { duplicate: true }), item(2, { error: "Host alanı yok." }), item(3)];
    expect([...defaultSelection(items)]).toEqual([0, 3]);
  });
  it("cannot import when nothing importable is selected", () => {
    const items = [item(0, { error: "x" }), item(1)];
    expect(canImport(items, new Set())).toBe(false);
    expect(canImport(items, new Set([0]))).toBe(false);
    expect(canImport(items, new Set([1]))).toBe(true);
  });
  it("summarizes counts", () => {
    const items = [item(0), item(1, { duplicate: true }), item(2, { error: "x" })];
    expect(summarize(items)).toEqual({ total: 3, importable: 2, duplicates: 1, errors: 1 });
  });
  it("labels ungrouped items", () => {
    expect(groupLabel(item(0))).toBe(UNGROUPED);
    expect(groupLabel(item(0, { group: "Üretim" }))).toBe("Üretim");
  });
});
