import { describe, expect, it } from "vitest";
import pg16 from "./fixtures/plan-pg16.json?raw";
import { initialExpanded, parsePlan, type PlanNode } from "./plan-model";
import {
  DASH,
  ancestorsOf,
  barRatio,
  fmtCost,
  fmtMs,
  fmtRows,
  misestimateText,
  nodeNotes,
  summaryNotes,
  treeKey,
  visibleRows,
} from "./plan-view-model";
import { tplan } from "./strings-plan";

const parsed = (json: string) => {
  const p = parsePlan(json);
  if ("error" in p) throw new Error(p.error);
  return p;
};

const node = (id: number, children: PlanNode[] = [], over: Partial<PlanNode> = {}): PlanNode => ({
  id,
  nodeType: "Seq Scan",
  label: `n${id}`,
  startupCost: null,
  totalCost: null,
  planRows: null,
  planWidth: null,
  exclusiveCost: null,
  details: [],
  children,
  ...over,
});

// 0 ─┬─ 1 ── 2
//    └─ 3
const tree = node(0, [node(1, [node(2)]), node(3)]);

describe("görünür satırlar ve klavye", () => {
  it("yalnız açık düğümlerin çocukları görünür", () => {
    expect(visibleRows(tree, new Set([0, 1])).map((r) => [r.node.id, r.depth, r.parent])).toEqual([
      [0, 0, null],
      [1, 1, 0],
      [2, 2, 1],
      [3, 1, 0],
    ]);
    expect(visibleRows(tree, new Set([0])).map((r) => r.node.id)).toEqual([0, 1, 3]);
  });
  it("↑/↓/Home/End, →/← aç-kapat-git, Enter ayrıntı", () => {
    const open = new Set([0]);
    const rows = visibleRows(tree, open);
    expect(treeKey("ArrowDown", rows, 0, open)).toEqual({ active: 1 });
    expect(treeKey("ArrowDown", rows, 3, open)).toEqual({ active: 3 });
    expect(treeKey("ArrowUp", rows, 0, open)).toEqual({ active: 0 });
    expect(treeKey("End", rows, 0, open)).toEqual({ active: 3 });
    expect(treeKey("Home", rows, 3, open)).toEqual({ active: 0 });
    expect(treeKey("ArrowRight", rows, 1, open)).toEqual({ active: 1, expand: 1 });
    expect(treeKey("ArrowRight", rows, 0, open)).toEqual({ active: 1 });
    expect(treeKey("ArrowRight", rows, 3, open)).toEqual({ active: 3 });
    expect(treeKey("ArrowLeft", rows, 0, open)).toEqual({ active: 0, collapse: 0 });
    expect(treeKey("ArrowLeft", rows, 3, open)).toEqual({ active: 0 });
    expect(treeKey("Enter", rows, 3, open)).toEqual({ active: 3, open: 3 });
    expect(treeKey("a", rows, 3, open)).toBeNull();
    // Etkin düğüm görünmüyorsa (daraltıldı) ilk satıra döner.
    expect(treeKey("ArrowDown", rows, 2, open)).toEqual({ active: 0 });
  });
  it("ataları: en pahalı düğüm seçilince açılacak yol", () => {
    expect(ancestorsOf(tree, 2)).toEqual([0, 1]);
    expect(ancestorsOf(tree, 0)).toEqual([]);
    expect(ancestorsOf(tree, 9)).toBeNull();
  });
});

describe("biçimlendirme: null → —, asla 0", () => {
  it("COSTS OFF / TIMING OFF", () => {
    const n = node(5, [], { planRows: null, actual: { startup: null, total: null, rows: 4, loops: 2 } });
    expect(fmtCost(n)).toBe(DASH);
    expect(fmtRows(n)).toBe(`${DASH} → 8`);
    expect(fmtMs(undefined)).toBe(DASH);
    expect(fmtMs(null)).toBe(DASH);
    expect(barRatio(n, n, { triggers: [], details: [] })).toBeNull();
  });
  it("değerler", () => {
    const n = node(5, [], { startupCost: 1.5, totalCost: 10, planRows: 100 });
    expect(fmtCost(n)).toBe("1,5..10");
    expect(fmtRows(n)).toBe("100");
    expect(fmtMs(0.1234)).toBe("0,123 ms");
    expect(fmtMs(1500)).toBe("1,5 sn");
  });
  it("sapma rozetinin metin karşılığı yalnız eşik üstünde", () => {
    expect(misestimateText(node(1, [], { misestimate: { factor: 12, direction: "under", level: "warn" } }))).toBe(tplan.misestimate("12", "under"));
    expect(misestimateText(node(1, [], { misestimate: { factor: 2, direction: "over", level: "ok" } }))).toBeNull();
  });
  it("çubuk oranı: ANALYZE varsa özel süre / toplam", () => {
    const { root, summary } = parsed(pg16);
    const r = barRatio(root, root, summary);
    expect(r).not.toBeNull();
    expect(r!).toBeGreaterThanOrEqual(0);
    expect(r!).toBeLessThanOrEqual(1);
  });
});

describe("görünüm notları", () => {
  it("CTE'li ANALYZE planında özet ve CTE Scan ayrıntısında CTE notu", () => {
    const { root } = parsed(pg16);
    expect(summaryNotes(root)).toContain(tplan.noteCte);
    let cte: PlanNode | null = null;
    const stack = [root];
    while (stack.length) {
      const n = stack.pop()!;
      if (n.nodeType === "CTE Scan") cte = n;
      stack.push(...n.children);
    }
    expect(nodeNotes(root, cte!)).toContain(tplan.noteCte);
    expect(initialExpanded(root).has(root.id)).toBe(true);
  });
  it("Gather altındaki düğümde paralel notu; ANALYZE'sız planda not yok", () => {
    const leaf = node(2, [], { inclusiveMs: 3 });
    const g = node(1, [leaf], { nodeType: "Gather", inclusiveMs: 5 });
    const root = node(0, [g], { inclusiveMs: 6 });
    expect(nodeNotes(root, leaf)).toEqual([tplan.noteParallel]);
    expect(summaryNotes(root)).toEqual([tplan.noteParallel]);
    expect(summaryNotes(node(0, [node(1, [], { nodeType: "Gather" })]))).toEqual([]);
  });
});
