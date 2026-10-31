// Görsel EXPLAIN saf modeli: `EXPLAIN (FORMAT JSON, …)` metnini ağaca ayrıştırır, dahil/özel süreleri, özel maliyeti
// ve satır tahmini sapmasını hesaplar (spec §1 "Ayrıştırma"/"Hesaplar", §2). Yalnızca tembel QueryTab parçası içe
// aktarır; ana paket modüllerine (`app/strings`, `explorer/*`) bağımlılığı yoktur.
//
// Sürümler arası (PG 10–18) alanlar değişebildiği için ayrıştırma geneldir: bilinen anahtarlar tiplenir, kalan her
// anahtar `details` içinde özgün sırası ve değeriyle (aynen) korunur.
import { tplan } from "./strings-plan";

export type JsonValue = string | number | boolean | null | JsonValue[] | { [k: string]: JsonValue };

/** Düğüm (ya da planlama) buffer sayaçları; PG'de çocukları da kapsar (birikimli). */
export type PlanBuffers = {
  sharedHit: number;
  sharedRead: number;
  sharedDirtied: number;
  sharedWritten: number;
  localHit: number;
  localRead: number;
  localDirtied: number;
  localWritten: number;
  tempRead: number;
  tempWritten: number;
};

export type Misestimate = { factor: number; direction: "over" | "under"; level: "ok" | "warn" | "crit" };

export type PlanNode = {
  /** Önce-kök (preorder) sırası; kök 0. */
  id: number;
  nodeType: string;
  label: string;
  relation?: string;
  index?: string;
  joinType?: string;
  /** `COSTS OFF` ile JSON'da yoktur → null. */
  startupCost: number | null;
  totalCost: number | null;
  planRows: number | null;
  planWidth: number | null;
  /** Yalnız ANALYZE ile. `TIMING OFF` ise startup/total null. Hiç çalışmamış düğümde loops = 0. */
  actual?: { startup: number | null; total: number | null; rows: number; loops: number };
  /** `Actual Total Time` × `Actual Loops` (paralel düğümlerde döngü işçileri de içerir). */
  inclusiveMs?: number;
  /** Dahil − çocukların dahili (InitPlan/SubPlan/CTE alt planları da çocuk); negatif → 0. */
  exclusiveMs?: number;
  /** Toplam maliyet − çocukların toplam maliyeti; negatif → 0. Maliyet yoksa null. */
  exclusiveCost: number | null;
  misestimate?: Misestimate;
  buffers?: PlanBuffers;
  /** Tiplenmemiş tüm anahtarlar, JSON'daki sırasıyla ve aynen (ör. Filter, Index Cond, Workers, sürüme özgü alanlar). */
  details: [string, JsonValue][];
  children: PlanNode[];
};

export type PlanTrigger = { name: string; constraint?: string; relation?: string; timeMs?: number; calls?: number };

export type PlanSummary = {
  planningMs?: number;
  executionMs?: number;
  /** Yüzde çubukları için toplam: `Execution Time`, yoksa kök dahil süre. */
  totalMs?: number;
  triggers: PlanTrigger[];
  jit?: { functions?: number; totalMs?: number; details: [string, JsonValue][] };
  settings?: [string, string][];
  /** PG 13+ `Planning` buffer'ları (BUFFERS ile). */
  planningBuffers?: PlanBuffers;
  /** Üst düzeyde tanınmayan anahtarlar (ör. `Query Identifier`, `Serialization`), aynen. */
  details: [string, JsonValue][];
};

/** Sapma eşikleri (spec §1): ≥10× uyarı, ≥100× kritik. */
export const MISESTIMATE_WARN = 10;
export const MISESTIMATE_CRIT = 100;
/** Bu sayıda ve üstü düğümde başlangıçta yalnızca ilk 3 düzey açılır. */
export const LARGE_PLAN_NODES = 200;
const LARGE_PLAN_OPEN_DEPTH = 3;

const BUFFER_KEYS: [string, keyof PlanBuffers][] = [
  ["Shared Hit Blocks", "sharedHit"],
  ["Shared Read Blocks", "sharedRead"],
  ["Shared Dirtied Blocks", "sharedDirtied"],
  ["Shared Written Blocks", "sharedWritten"],
  ["Local Hit Blocks", "localHit"],
  ["Local Read Blocks", "localRead"],
  ["Local Dirtied Blocks", "localDirtied"],
  ["Local Written Blocks", "localWritten"],
  ["Temp Read Blocks", "tempRead"],
  ["Temp Written Blocks", "tempWritten"],
];
const BUFFER_KEY_SET = new Set(BUFFER_KEYS.map(([k]) => k));

/**
 * Tamamen tiplenen (details'e kopyalanmayan) sayısal düğüm anahtarları; buffer sayaçları da `buffers`'a taşınır.
 * `Plans` yalnız dizi ise tüketilir. `Relation Name`/`Index Name`/`Join Type` hem tiplenir hem ayrıntıda kalır.
 */
const TYPED_NODE_KEYS = new Set([
  "Node Type",
  "Startup Cost",
  "Total Cost",
  "Plan Rows",
  "Plan Width",
  "Actual Startup Time",
  "Actual Total Time",
  "Actual Rows",
  "Actual Loops",
]);

const TOP_KEYS = new Set(["Plan", "Planning", "Planning Time", "Execution Time", "Triggers", "JIT", "Settings"]);

type Obj = { [k: string]: JsonValue };

function isObj(v: unknown): v is Obj {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function num(v: unknown): number | undefined {
  if (typeof v === "number") return Number.isFinite(v) ? v : undefined;
  if (typeof v === "string" && v.trim() !== "") {
    const n = Number(v);
    return Number.isFinite(n) ? n : undefined;
  }
  return undefined;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function readBuffers(o: Obj): PlanBuffers | undefined {
  let any = false;
  const b = {} as PlanBuffers;
  for (const [k, f] of BUFFER_KEYS) {
    const n = num(o[k]);
    if (n !== undefined) any = true;
    b[f] = n ?? 0;
  }
  return any ? b : undefined;
}

/** JSON düğümünü (ve çocuklarını, önce-kök kimliklerle) PlanNode yapar; hesaplar çocuklardan sonra yapılır. */
function buildNode(o: Obj, nextId: { v: number }): PlanNode {
  const id = nextId.v++;
  const details: [string, JsonValue][] = [];
  const buffers = readBuffers(o);
  let kids: Obj[] = [];
  for (const k of Object.keys(o)) {
    // Beklenmedik türdeki değer tiplenemez; kaybolmasın diye ayrıntıda kalır.
    if (TYPED_NODE_KEYS.has(k) && (k === "Node Type" ? typeof o[k] === "string" : num(o[k]) !== undefined)) continue;
    if (buffers && BUFFER_KEY_SET.has(k) && num(o[k]) !== undefined) continue;
    if (k === "Plans" && Array.isArray(o[k])) {
      kids = (o[k] as JsonValue[]).filter(isObj);
      continue;
    }
    details.push([k, o[k]]);
  }

  const startupCost = num(o["Startup Cost"]) ?? null;
  const totalCost = num(o["Total Cost"]) ?? null;
  const planRows = num(o["Plan Rows"]) ?? null;
  const planWidth = num(o["Plan Width"]) ?? null;

  const rows = num(o["Actual Rows"]);
  const loops = num(o["Actual Loops"]);
  const actual =
    rows !== undefined && loops !== undefined
      ? {
          startup: num(o["Actual Startup Time"]) ?? null,
          total: num(o["Actual Total Time"]) ?? null,
          rows,
          loops,
        }
      : undefined;

  const node: PlanNode = {
    id,
    nodeType: str(o["Node Type"]) ?? "?",
    label: "",
    relation: str(o["Relation Name"]),
    index: str(o["Index Name"]),
    joinType: str(o["Join Type"]),
    startupCost,
    totalCost,
    planRows,
    planWidth,
    actual,
    exclusiveCost: null,
    buffers,
    details,
    children: kids.map((k) => buildNode(k, nextId)),
  };
  if (node.relation === undefined) delete node.relation;
  if (node.index === undefined) delete node.index;
  if (node.joinType === undefined) delete node.joinType;
  if (!actual) delete node.actual;
  if (!buffers) delete node.buffers;

  node.label = labelFrom(node.nodeType, o);
  computeNode(node);
  return node;
}

function computeNode(n: PlanNode): void {
  const a = n.actual;
  if (a && a.total !== null) {
    n.inclusiveMs = a.total * a.loops;
    let kids = 0;
    for (const c of n.children) kids += c.inclusiveMs ?? 0;
    n.exclusiveMs = Math.max(0, n.inclusiveMs - kids);
  }
  if (n.totalCost !== null) {
    let kids = 0;
    for (const c of n.children) kids += c.totalCost ?? 0;
    n.exclusiveCost = Math.max(0, n.totalCost - kids);
  }
  if (a && n.planRows !== null) {
    const m = misestimate(n.planRows, a.rows, a.loops);
    if (m) n.misestimate = m;
  }
}

/**
 * Tahmin/gerçek satır sapması (spec §1): max(gerçek×döngü, 1) / max(tahmin×döngü, 1) ya da tersi.
 * PG JSON'unda `Plan Rows` ve `Actual Rows` döngü başınadır. Gerçek > tahmin → "under" (az tahmin), aksi "over".
 * Döngü 0 (hiç çalışmadı) ya da geçersiz sayı → null.
 */
export function misestimate(planRows: number, actualRows: number, loops: number): Misestimate | null {
  if (!Number.isFinite(planRows) || !Number.isFinite(actualRows) || !Number.isFinite(loops) || loops <= 0) return null;
  const act = Math.max(actualRows * loops, 1);
  const est = Math.max(planRows * loops, 1);
  const under = act > est;
  const factor = under ? act / est : est / act;
  const level = factor >= MISESTIMATE_CRIT ? "crit" : factor >= MISESTIMATE_WARN ? "warn" : "ok";
  return { factor, direction: under ? "under" : "over", level };
}

const AGG_STRATEGY: Record<string, string> = {
  Plain: "Aggregate",
  Sorted: "GroupAggregate",
  Hashed: "HashAggregate",
  Mixed: "MixedAggregate",
};

/** PG metin biçimindeki düğüm başlığına yakın etiket (explain.c ile aynı sözcükler; PG terimleri çevrilmez). */
function labelFrom(nodeType: string, o: Obj): string {
  let base = nodeType;
  const strategy = str(o["Strategy"]);
  if (nodeType === "Aggregate" && strategy && AGG_STRATEGY[strategy]) base = AGG_STRATEGY[strategy];
  if (nodeType === "SetOp") {
    if (strategy === "Hashed") base = "HashSetOp";
    const cmd = str(o["Command"]);
    if (cmd) base += ` ${cmd}`;
  }
  if (nodeType === "ModifyTable") {
    const op = str(o["Operation"]);
    if (op) base = op;
  }
  if (nodeType === "Custom Scan") {
    const p = str(o["Custom Plan Provider"]);
    if (p) base += ` (${p})`;
  }
  const partial = str(o["Partial Mode"]);
  if (partial === "Partial" || partial === "Finalize") base = `${partial} ${base}`;
  if (o["Parallel Aware"] === true) base = `Parallel ${base}`;
  if (o["Async Capable"] === true) base = `Async ${base}`;

  const jt = str(o["Join Type"]);
  if (jt && jt !== "Inner") base = base.endsWith(" Join") ? `${base.slice(0, -5)} ${jt} Join` : `${base} ${jt} Join`;

  if (str(o["Scan Direction"]) === "Backward") base += " Backward";

  const index = str(o["Index Name"]);
  const relName = str(o["Relation Name"]);
  if (index) base += relName || nodeType !== "Bitmap Index Scan" ? ` using ${index}` : ` on ${index}`;

  // Taranan nesne: tablo, CTE, fonksiyon, tablo fonksiyonu… ya da yalnız takma ad (Subquery/Values Scan).
  const target =
    relName ?? str(o["CTE Name"]) ?? str(o["Function Name"]) ?? str(o["Table Function Name"]) ?? str(o["Tuplestore Name"]);
  const alias = str(o["Alias"]);
  if (target) {
    const schema = relName ? str(o["Schema"]) : undefined;
    base += ` on ${schema ? `${schema}.` : ""}${target}`;
    if (alias && alias !== target) base += ` ${alias}`;
  } else if (alias) {
    base += ` on ${alias}`;
  }
  return base;
}

/** Düğümün ağaçtaki etiketi ("Index Scan using idx on t" vb.); ayrıştırmada `label` olarak da saklanır. */
export function nodeLabel(n: PlanNode): string {
  return n.label;
}

/** Önce-kök gezinti (özyinelemesiz). */
export function walkPlan(root: PlanNode, fn: (n: PlanNode, depth: number) => void): void {
  const stack: [PlanNode, number][] = [[root, 0]];
  while (stack.length) {
    const [n, d] = stack.pop()!;
    fn(n, d);
    for (let i = n.children.length - 1; i >= 0; i--) stack.push([n.children[i], d + 1]);
  }
}

function readSummary(top: Obj, root: PlanNode): PlanSummary {
  const s: PlanSummary = { triggers: [], details: [] };
  const planning = num(top["Planning Time"]);
  const execution = num(top["Execution Time"]);
  if (planning !== undefined) s.planningMs = planning;
  if (execution !== undefined) s.executionMs = execution;
  const total = execution ?? root.inclusiveMs;
  if (total !== undefined) s.totalMs = total;

  const trg = top["Triggers"];
  if (Array.isArray(trg)) {
    for (const t of trg) {
      if (!isObj(t)) continue;
      s.triggers.push({
        name: str(t["Trigger Name"]) ?? "?",
        constraint: str(t["Constraint Name"]),
        relation: str(t["Relation"]),
        timeMs: num(t["Time"]),
        calls: num(t["Calls"]),
      });
    }
  }

  const jit = top["JIT"];
  if (isObj(jit)) {
    const timing = jit["Timing"];
    s.jit = {
      functions: num(jit["Functions"]),
      totalMs: isObj(timing) ? num(timing["Total"]) : undefined,
      details: Object.entries(jit),
    };
  }

  const settings = top["Settings"];
  if (isObj(settings)) s.settings = Object.entries(settings).map(([k, v]) => [k, typeof v === "string" ? v : JSON.stringify(v)]);

  const pb = top["Planning"];
  if (isObj(pb)) {
    const b = readBuffers(pb);
    if (b) s.planningBuffers = b;
  }

  for (const k of Object.keys(top)) {
    if (TOP_KEYS.has(k)) continue;
    s.details.push([k, top[k]]);
  }
  // Tanınmayan alt alanlı `Planning` (ör. ileride eklenecek planlama sayaçları) kaybolmasın.
  if (isObj(pb)) {
    const extra = Object.keys(pb).filter((k) => !BUFFER_KEY_SET.has(k));
    if (extra.length) s.details.push(["Planning", pb]);
  } else if (pb !== undefined) {
    s.details.push(["Planning", pb]);
  }
  return s;
}

/**
 * `EXPLAIN (FORMAT JSON)` metnini ayrıştırır: `[{ "Plan": {...}, "Planning Time", "Execution Time", "Triggers",
 * "JIT", "Settings", … }]`. Tek nesne de kabul edilir; dizide birden çok öğe varsa ilki alınır.
 */
export function parsePlan(json: string): { root: PlanNode; summary: PlanSummary } | { error: string } {
  let data: unknown;
  try {
    data = JSON.parse(json);
  } catch {
    return { error: tplan.errInvalidJson };
  }
  const top = Array.isArray(data) ? data[0] : data;
  if (!isObj(top) || !isObj(top["Plan"]) || typeof top["Plan"]["Node Type"] !== "string") {
    return { error: tplan.errNoPlan };
  }
  const root = buildNode(top["Plan"], { v: 0 });
  return { root, summary: readSummary(top, root) };
}

/** Ölçüt: ANALYZE süresi varsa özel süre, yoksa özel maliyet (spec §1 "En pahalı 5 düğüm"). */
function hotKey(timed: boolean, n: PlanNode): number {
  return (timed ? n.exclusiveMs : n.exclusiveCost) ?? 0;
}

/** En pahalı n düğüm: özel süreye (ANALYZE yoksa özel maliyete) göre azalan; eşitlikte kimlik sırası. */
export function hotNodes(root: PlanNode, n: number): PlanNode[] {
  if (n <= 0) return [];
  const timed = root.inclusiveMs !== undefined;
  const all: PlanNode[] = [];
  walkPlan(root, (x) => all.push(x));
  all.sort((a, b) => hotKey(timed, b) - hotKey(timed, a) || a.id - b.id);
  return all.slice(0, n);
}

/**
 * Başlangıçta açık (genişletilmiş) düğüm kimlikleri: çocuklu tüm düğümler; 200+ düğümde yalnız derinliği 0–2 olanlar
 * (ilk 3 düzey açık, sonrası daraltılmış).
 */
export function initialExpanded(root: PlanNode): Set<number> {
  let count = 0;
  walkPlan(root, () => count++);
  const limit = count >= LARGE_PLAN_NODES ? LARGE_PLAN_OPEN_DEPTH : Infinity;
  const out = new Set<number>();
  walkPlan(root, (x, d) => {
    if (x.children.length > 0 && d < limit) out.add(x.id);
  });
  return out;
}

/**
 * Izgaradaki sonuç kullanıcının kendi `EXPLAIN (FORMAT JSON)` çıktısı mı: tek `QUERY PLAN` sütunu (json), tek satır
 * ve plan biçiminde metin. Ucuz denetim; tam ayrıştırma "Planı göster" tıklanınca yapılır.
 */
export function looksLikePlanResult(columns: { name: string; typeName: string }[], rows: (string | null)[][]): boolean {
  if (columns.length !== 1 || rows.length !== 1) return false;
  const c = columns[0];
  if (c.name !== "QUERY PLAN" || c.typeName !== "json") return false;
  const cell = rows[0][0];
  if (typeof cell !== "string") return false;
  const head = cell.trimStart();
  return (head.startsWith("[") || head.startsWith("{")) && /"Plan"\s*:\s*\{/.test(cell.slice(0, 200));
}
