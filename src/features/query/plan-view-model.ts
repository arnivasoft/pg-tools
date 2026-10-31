// Plan ağacı görünümünün saf yardımcıları: görünür satırlar, klavye gezintisi (WAI-ARIA tree), biçimlendirme ve
// görünüm notları (CTE, paralel). Yalnız tembel QueryTab parçası içe aktarır.
import { walkPlan, type PlanBuffers, type PlanNode, type PlanSummary } from "./plan-model";
import { tplan } from "./strings-plan";

export type PlanRow = { node: PlanNode; depth: number; parent: number | null };

/** Açık düğümlere göre görünür satırlar (önce-kök sırası). */
export function visibleRows(root: PlanNode, expanded: ReadonlySet<number>): PlanRow[] {
  const out: PlanRow[] = [];
  const stack: PlanRow[] = [{ node: root, depth: 0, parent: null }];
  while (stack.length) {
    const r = stack.pop()!;
    out.push(r);
    if (!expanded.has(r.node.id)) continue;
    const kids = r.node.children;
    for (let i = kids.length - 1; i >= 0; i--) stack.push({ node: kids[i], depth: r.depth + 1, parent: r.node.id });
  }
  return out;
}

/** Kökten düğüme giden yol (kök dahil, düğüm hariç); düğüm yoksa null. */
export function ancestorsOf(root: PlanNode, id: number): number[] | null {
  const parent = new Map<number, number>();
  let found = false;
  walkPlan(root, (n) => {
    if (n.id === id) found = true;
    for (const c of n.children) parent.set(c.id, n.id);
  });
  if (!found) return null;
  const path: number[] = [];
  for (let p = parent.get(id); p !== undefined; p = parent.get(p)) path.unshift(p);
  return path;
}

export function findNode(root: PlanNode, id: number): PlanNode | null {
  let hit: PlanNode | null = null;
  walkPlan(root, (n) => {
    if (n.id === id) hit = n;
  });
  return hit;
}

export type TreeKeyResult = { active: number; expand?: number; collapse?: number; open?: number } | null;

/**
 * Ağaç klavyesi (WAI-ARIA tree): ↑/↓ önceki/sonraki görünür satır, → kapalıysa aç / açıksa ilk çocuk,
 * ← açıksa kapat / değilse üst düğüm, Home/End ilk/son satır, Enter ayrıntıyı açar. Tanınmayan tuş → null.
 */
export function treeKey(key: string, rows: PlanRow[], active: number, expanded: ReadonlySet<number>): TreeKeyResult {
  const i = rows.findIndex((r) => r.node.id === active);
  if (i < 0) return rows.length > 0 ? { active: rows[0].node.id } : null;
  const row = rows[i];
  const hasKids = row.node.children.length > 0;
  switch (key) {
    case "ArrowDown":
      return { active: rows[Math.min(rows.length - 1, i + 1)].node.id };
    case "ArrowUp":
      return { active: rows[Math.max(0, i - 1)].node.id };
    case "Home":
      return { active: rows[0].node.id };
    case "End":
      return { active: rows[rows.length - 1].node.id };
    case "ArrowRight":
      if (!hasKids) return { active };
      if (!expanded.has(active)) return { active, expand: active };
      return { active: row.node.children[0].id };
    case "ArrowLeft":
      if (hasKids && expanded.has(active)) return { active, collapse: active };
      return { active: row.parent ?? active };
    case "Enter":
      return { active, open: active };
    default:
      return null;
  }
}

// ---------------------------------------------------------------- Biçimlendirme (null → "—", asla 0)

export const DASH = "—";

export function fmtMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return DASH;
  if (ms >= 1000) return `${(ms / 1000).toLocaleString("tr-TR", { maximumFractionDigits: 2 })} sn`;
  return `${ms.toLocaleString("tr-TR", { maximumFractionDigits: ms < 10 ? 3 : 1 })} ms`;
}

export function fmtNum(n: number | null | undefined, digits = 2): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return DASH;
  return n.toLocaleString("tr-TR", { maximumFractionDigits: digits });
}

/** Maliyet aralığı "başlangıç..toplam"; maliyet yoksa (COSTS OFF) "—". */
export function fmtCost(n: PlanNode): string {
  if (n.totalCost === null) return DASH;
  return n.startupCost === null ? fmtNum(n.totalCost) : `${fmtNum(n.startupCost)}..${fmtNum(n.totalCost)}`;
}

/** Tahmin → gerçek satır (gerçek: döngü başı × döngü = toplam satır). */
export function fmtRows(n: PlanNode): string {
  const est = n.planRows === null ? DASH : fmtNum(n.planRows, 0);
  if (!n.actual) return est;
  return `${est} → ${fmtNum(n.actual.rows * n.actual.loops, 0)}`;
}

/** Sapma rozetinin metni (rozetin görünür metin karşılığı); sapma yoksa ya da eşik altıysa null. */
export function misestimateText(n: PlanNode): string | null {
  const m = n.misestimate;
  if (!m || m.level === "ok") return null;
  return tplan.misestimate(fmtNum(m.factor, m.factor < 100 ? 1 : 0), m.direction);
}

/** Buffer kısaltması: "hit 12 / read 3" (yalnız shared; yoksa null). */
export function fmtBuffers(b: PlanBuffers | undefined): string | null {
  if (!b) return null;
  return tplan.buffersShort(fmtNum(b.sharedHit, 0), fmtNum(b.sharedRead, 0));
}

/**
 * Süre çubuğu oranı (0–1): ANALYZE varsa özel süre / toplam, yoksa özel maliyet / kök toplam maliyeti.
 * Ölçü yoksa null (çubuk çizilmez).
 */
export function barRatio(n: PlanNode, root: PlanNode, summary: PlanSummary): number | null {
  if (root.inclusiveMs !== undefined) {
    const total = summary.totalMs ?? root.inclusiveMs;
    if (n.exclusiveMs === undefined || !total) return null;
    return Math.min(1, n.exclusiveMs / total);
  }
  if (n.exclusiveCost === null || !root.totalCost) return null;
  return Math.min(1, n.exclusiveCost / root.totalCost);
}

export function fmtPercent(r: number | null): string {
  return r === null ? DASH : `${(r * 100).toLocaleString("tr-TR", { maximumFractionDigits: 1 })}%`;
}

// ---------------------------------------------------------------- Görünüm notları

const isGather = (n: PlanNode) => n.nodeType === "Gather" || n.nodeType === "Gather Merge";

function detail(n: PlanNode, key: string): unknown {
  return n.details.find(([k]) => k === key)?.[1];
}

/** CTE ile ilgili düğüm: CTE Scan, CTE alt planı ya da CTE alt planı olan düğüm. */
export function isCteNode(n: PlanNode): boolean {
  const sub = (x: PlanNode) => {
    const s = detail(x, "Subplan Name");
    return typeof s === "string" && s.startsWith("CTE ");
  };
  return n.nodeType === "CTE Scan" || sub(n) || n.children.some(sub);
}

/** Paralel bağlam: Gather/Gather Merge, paralel farkında düğüm ya da Gather altındaki düğüm. */
export function isParallelNode(root: PlanNode, n: PlanNode): boolean {
  if (isGather(n) || detail(n, "Parallel Aware") === true) return true;
  const path = ancestorsOf(root, n.id) ?? [];
  return path.some((id) => {
    const a = findNode(root, id);
    return a !== null && isGather(a);
  });
}

/** Özet notları: planda CTE ya da paralel düğüm varsa (yalnız ANALYZE sürelerinde anlamlı). */
export function summaryNotes(root: PlanNode): string[] {
  if (root.inclusiveMs === undefined) return [];
  let cte = false;
  let parallel = false;
  walkPlan(root, (n) => {
    if (isCteNode(n)) cte = true;
    if (isGather(n)) parallel = true;
  });
  const out: string[] = [];
  if (cte) out.push(tplan.noteCte);
  if (parallel) out.push(tplan.noteParallel);
  return out;
}

/** Ayrıntı panelinin notları (yalnız ANALYZE sürelerinde). */
export function nodeNotes(root: PlanNode, n: PlanNode): string[] {
  if (n.inclusiveMs === undefined) return [];
  const out: string[] = [];
  if (isCteNode(n)) out.push(tplan.noteCte);
  if (isParallelNode(root, n)) out.push(tplan.noteParallel);
  return out;
}

/** Ayrıntı değerinin metni: dizi/nesne JSON, diğerleri aynen. */
export function detailText(v: unknown): string {
  if (v === null) return "null";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  return JSON.stringify(v);
}
