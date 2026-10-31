import { describe, expect, it } from "vitest";
import pg10 from "./fixtures/plan-pg10.json?raw";
import pg14 from "./fixtures/plan-pg14.json?raw";
import pg17 from "./fixtures/plan-pg17.json?raw";
import pg18 from "./fixtures/plan-pg18.json?raw";
import {
  hotNodes,
  initialExpanded,
  looksLikePlanResult,
  misestimate,
  nodeLabel,
  parsePlan,
  walkPlan,
  type PlanNode,
  type PlanSummary,
} from "./plan-model";
import { tplan } from "./strings-plan";

function ok(json: string): { root: PlanNode; summary: PlanSummary } {
  const r = parsePlan(json);
  if ("error" in r) throw new Error(r.error);
  return r;
}

function flat(root: PlanNode): PlanNode[] {
  const out: PlanNode[] = [];
  walkPlan(root, (n) => out.push(n));
  return out;
}

/** Sentetik plan: `[{ Plan: node, ...top }]` metni. */
function wrap(plan: unknown, top: Record<string, unknown> = {}): string {
  return JSON.stringify([{ Plan: plan, ...top }]);
}

const scan = (over: Record<string, unknown> = {}) => ({
  "Node Type": "Seq Scan",
  "Relation Name": "t",
  Alias: "t",
  "Startup Cost": 0,
  "Total Cost": 10,
  "Plan Rows": 100,
  "Plan Width": 4,
  ...over,
});

describe("parsePlan — gerçek fixture'lar (PG 10/14/17/18)", () => {
  const cases: [string, string, number, number][] = [
    ["10", pg10, 0.433, 1.893],
    ["14", pg14, 0.5, 6.501],
    ["17", pg17, 0.375, 2.066],
    ["18", pg18, 0.553, 1.765],
  ];
  for (const [v, json, planning, execution] of cases) {
    it(`PG ${v}: 14 düğüm, özet ve sıralı kimlikler`, () => {
      const { root, summary } = ok(json);
      const nodes = flat(root);
      expect(nodes).toHaveLength(14);
      expect(nodes.map((n) => n.id)).toEqual([...Array(14).keys()]);
      expect(root.nodeType).toBe("Limit");
      expect(root.actual?.loops).toBe(1);
      expect(summary.planningMs).toBe(planning);
      expect(summary.executionMs).toBe(execution);
      expect(summary.triggers).toEqual([]);
      expect(summary.totalMs).toBe(execution);
      // PG 13+ planlama buffer'ları; PG 10'da yok.
      if (v === "10") expect(summary.planningBuffers).toBeUndefined();
      else expect(summary.planningBuffers?.sharedHit).toBeGreaterThan(0);
      // Her düğümde maliyet, gerçek değerler ve buffer var; sapma hesaplanmış.
      for (const n of nodes) {
        expect(n.totalCost).not.toBeNull();
        expect(n.actual).toBeDefined();
        expect(n.inclusiveMs).toBeGreaterThanOrEqual(0);
        expect(n.exclusiveMs).toBeGreaterThanOrEqual(0);
        expect(n.buffers).toBeDefined();
        expect(n.misestimate).toBeDefined();
        expect(n.details.some(([k]) => k === "Plans")).toBe(false);
      }
    });
  }

  it("PG 10: etiketler, ilişki/indeks ve ayrıntılar", () => {
    const nodes = flat(ok(pg10).root);
    expect(nodes.map(nodeLabel)).toEqual([
      "Limit",
      "HashAggregate",
      "Seq Scan on qe_fx_orders",
      "Aggregate",
      "CTE Scan on totals",
      "Result",
      "Sort",
      "Hash Join",
      "CTE Scan on totals t",
      "Hash",
      "Seq Scan on qe_fx_customers c",
      "Aggregate",
      "Bitmap Heap Scan on qe_fx_orders o",
      "Bitmap Index Scan on qe_fx_orders_customer",
    ]);
    expect(nodes.map((n) => n.label)).toEqual(nodes.map(nodeLabel));
    expect(nodes[2].relation).toBe("qe_fx_orders");
    expect(nodes[13].index).toBe("qe_fx_orders_customer");
    expect(nodes[7].joinType).toBe("Inner");
    const details = new Map(nodes[1].details);
    expect(details.get("Parent Relationship")).toBe("InitPlan");
    expect(details.get("Subplan Name")).toBe("CTE totals");
    expect(details.get("Group Key")).toEqual(["qe_fx_orders.customer_id"]);
    // Tiplenmiş anahtarlar ayrıntıya kopyalanmaz.
    expect(details.has("Node Type")).toBe(false);
    expect(details.has("Total Cost")).toBe(false);
    expect(details.has("Shared Hit Blocks")).toBe(false);
  });

  it("PG 18: sürüme özgü anahtarlar (Async Capable, Disabled, Index Searches) ayrıntıda korunur", () => {
    const nodes = flat(ok(pg18).root);
    const d0 = new Map(nodes[0].details);
    expect(d0.get("Async Capable")).toBe(false);
    expect(d0.get("Disabled")).toBe(false);
    const d13 = new Map(nodes[13].details);
    expect(d13.get("Index Searches")).toBe(10);
    // PG 18 kesirli "Actual Rows" (10.00) sayı olarak okunur.
    expect(nodes[0].actual?.rows).toBe(10);
  });
});

describe("dahil / özel süre", () => {
  it("PG 10: döngü çarpımı, InitPlan/SubPlan çocuk sayılır, negatif → 0", () => {
    const n = flat(ok(pg10).root);
    // Limit: 1.775 − (1.336 + 0.104 + 1.773) < 0 → 0
    expect(n[0].inclusiveMs).toBeCloseTo(1.775, 9);
    expect(n[0].exclusiveMs).toBe(0);
    // CTE totals (InitPlan): 1.336 − 0.33
    expect(n[1].exclusiveMs).toBeCloseTo(1.006, 9);
    // Result: 1.773 − (1.575 + 0.019 × 10)
    expect(n[5].exclusiveMs).toBeCloseTo(0.008, 9);
    // SubPlan 2: loops = 10 → dahil 0.19; çocuk 0.014 × 10
    expect(n[11].inclusiveMs).toBeCloseTo(0.19, 9);
    expect(n[11].exclusiveMs).toBeCloseTo(0.05, 9);
    expect(n[13].inclusiveMs).toBeCloseTo(0.06, 9);
    expect(n[13].exclusiveMs).toBeCloseTo(0.06, 9);
  });

  it("özel maliyet = toplam − çocuk toplamları (negatif → 0)", () => {
    const n = flat(ok(pg10).root);
    expect(n[0].exclusiveCost).toBe(0); // 470.91 < çocuklar
    expect(n[1].exclusiveCost).toBeCloseTo(40, 9); // 118 − 78
    expect(n[2].exclusiveCost).toBe(78);
  });

  it("TIMING OFF: gerçek satır/döngü var, süre yok", () => {
    const { root } = ok(
      wrap({ ...scan(), "Actual Rows": 100, "Actual Loops": 1 }, { "Planning Time": 0.1, "Execution Time": 0.5 }),
    );
    expect(root.actual).toEqual({ startup: null, total: null, rows: 100, loops: 1 });
    expect(root.inclusiveMs).toBeUndefined();
    expect(root.exclusiveMs).toBeUndefined();
    expect(root.misestimate?.level).toBe("ok");
  });
});

describe("misestimate", () => {
  it("eşikler: ≥10× uyarı, ≥100× kritik", () => {
    expect(misestimate(100, 100, 1)).toEqual({ factor: 1, direction: "over", level: "ok" });
    expect(misestimate(10, 99, 1)?.level).toBe("ok");
    expect(misestimate(10, 100, 1)).toEqual({ factor: 10, direction: "under", level: "warn" });
    expect(misestimate(10, 999, 1)?.level).toBe("warn");
    expect(misestimate(10, 1000, 1)).toEqual({ factor: 100, direction: "under", level: "crit" });
    expect(misestimate(1000, 10, 1)).toEqual({ factor: 100, direction: "over", level: "crit" });
    expect(misestimate(500, 10, 1)).toEqual({ factor: 50, direction: "over", level: "warn" });
  });

  it("0 satır: max(…, 1) ile sıfıra bölünmez; döngü çarpımı alt sınırı etkiler", () => {
    expect(misestimate(1, 0, 1)).toEqual({ factor: 1, direction: "over", level: "ok" });
    expect(misestimate(0, 0, 5)?.factor).toBe(1);
    expect(misestimate(1000, 0, 1)?.factor).toBe(1000);
    // Döngü başına 0.5 gerçek (PG 18 kesirli) × 10 döngü = 5; tahmin 1 × 10 = 10 → 2× fazla tahmin.
    expect(misestimate(1, 0.5, 10)).toEqual({ factor: 2, direction: "over", level: "ok" });
  });

  it("döngü 0 (hiç çalışmadı) ya da geçersiz girdi → null", () => {
    expect(misestimate(10, 0, 0)).toBeNull();
    expect(misestimate(10, 5, Number.NaN)).toBeNull();
    expect(misestimate(Number.NaN, 5, 1)).toBeNull();
  });
});

describe("Never executed", () => {
  it("döngü 0: dahil/özel süre 0, sapma yok", () => {
    const never = {
      ...scan({ "Relation Name": "u", Alias: "u", "Plan Rows": 1000 }),
      "Actual Startup Time": 0,
      "Actual Total Time": 0,
      "Actual Rows": 0,
      "Actual Loops": 0,
    };
    const { root } = ok(
      wrap({
        "Node Type": "Nested Loop",
        "Join Type": "Left",
        "Startup Cost": 0,
        "Total Cost": 40,
        "Plan Rows": 1,
        "Plan Width": 8,
        "Actual Startup Time": 0.01,
        "Actual Total Time": 0.02,
        "Actual Rows": 0,
        "Actual Loops": 1,
        Plans: [
          { ...scan(), "Actual Startup Time": 0.01, "Actual Total Time": 0.015, "Actual Rows": 0, "Actual Loops": 1 },
          never,
        ],
      }),
    );
    const n = root.children[1];
    expect(n.actual).toEqual({ startup: 0, total: 0, rows: 0, loops: 0 });
    expect(n.inclusiveMs).toBe(0);
    expect(n.exclusiveMs).toBe(0);
    expect(n.misestimate).toBeUndefined();
    expect(root.exclusiveMs).toBeCloseTo(0.005, 9);
    expect(nodeLabel(root)).toBe("Nested Loop Left Join");
  });
});

describe("paralel Workers", () => {
  it("döngü işçileri içerir; Workers dizisi ayrıntıda aynen kalır", () => {
    const workers = [
      { "Worker Number": 0, "Actual Startup Time": 0.1, "Actual Total Time": 4.9, "Actual Rows": 3300, "Actual Loops": 1 },
      { "Worker Number": 1, "Actual Startup Time": 0.1, "Actual Total Time": 5.0, "Actual Rows": 3400, "Actual Loops": 1 },
    ];
    const { root } = ok(
      wrap({
        "Node Type": "Gather",
        "Startup Cost": 1000,
        "Total Cost": 2000,
        "Plan Rows": 10000,
        "Plan Width": 4,
        "Actual Startup Time": 0.3,
        "Actual Total Time": 6,
        "Actual Rows": 10000,
        "Actual Loops": 1,
        "Workers Planned": 2,
        "Workers Launched": 2,
        "Single Copy": false,
        Plans: [
          {
            ...scan({ "Parallel Aware": true, "Plan Rows": 4167, "Total Cost": 900 }),
            "Parent Relationship": "Outer",
            "Actual Startup Time": 0.05,
            "Actual Total Time": 5,
            "Actual Rows": 3333,
            "Actual Loops": 3,
            Workers: workers,
          },
        ],
      }),
    );
    const s = root.children[0];
    expect(nodeLabel(s)).toBe("Parallel Seq Scan on t");
    expect(s.inclusiveMs).toBeCloseTo(15, 9); // 5 × 3 (işçiler dahil)
    expect(root.exclusiveMs).toBe(0); // 6 − 15 < 0
    expect(new Map(s.details).get("Workers")).toEqual(workers);
    expect(new Map(root.details).get("Workers Launched")).toBe(2);
    expect(s.misestimate?.level).toBe("ok"); // 3333×3 / 4167×3
  });
});

describe("ANALYZE'sız plan", () => {
  it("gerçek alan yok; özel maliyetle en pahalı düğümler", () => {
    const { root, summary } = ok(
      wrap({
        "Node Type": "Hash Join",
        "Join Type": "Inner",
        "Startup Cost": 30,
        "Total Cost": 100,
        "Plan Rows": 50,
        "Plan Width": 8,
        "Hash Cond": "(a.id = b.id)",
        Plans: [
          scan({ "Relation Name": "a", Alias: "a", "Total Cost": 40, "Parent Relationship": "Outer" }),
          {
            "Node Type": "Hash",
            "Parent Relationship": "Inner",
            "Startup Cost": 25,
            "Total Cost": 25,
            "Plan Rows": 10,
            "Plan Width": 4,
            Plans: [scan({ "Relation Name": "b", Alias: "bb", "Total Cost": 25 })],
          },
        ],
      }),
    );
    expect(root.actual).toBeUndefined();
    expect(root.inclusiveMs).toBeUndefined();
    expect(root.misestimate).toBeUndefined();
    expect(root.exclusiveCost).toBe(35);
    expect(root.children[1].exclusiveCost).toBe(0);
    expect(summary).toEqual({ triggers: [], details: [] });
    // 0 kök: 35, 1 a: 40, 2 Hash: 0, 3 b: 25
    expect(hotNodes(root, 2).map((n) => n.id)).toEqual([1, 0]);
    expect(hotNodes(root, 5).map((n) => n.id)).toEqual([1, 0, 3, 2]);
    expect(nodeLabel(root.children[1].children[0])).toBe("Seq Scan on b bb");
  });

  it("COSTS OFF: maliyet/satır null, özel maliyet null", () => {
    const { root } = ok(wrap({ "Node Type": "Seq Scan", "Relation Name": "t", Alias: "t" }));
    expect(root).toMatchObject({ startupCost: null, totalCost: null, planRows: null, planWidth: null, exclusiveCost: null });
    expect(hotNodes(root, 5).map((n) => n.id)).toEqual([0]);
  });
});

describe("hotNodes (ANALYZE)", () => {
  it("özel süreye göre sıralar", () => {
    const { root } = ok(pg10);
    // 8: CTE Scan t (1.432), 1: CTE totals (1.006), 2: Seq Scan orders (0.33)
    expect(hotNodes(root, 3).map((n) => n.id)).toEqual([8, 1, 2]);
    expect(hotNodes(root, 0)).toEqual([]);
    expect(hotNodes(root, 100)).toHaveLength(14);
  });
});

describe("bilinmeyen anahtarlar ve özet", () => {
  it("düğüm ve üst düzeydeki bilinmeyen anahtarlar sırasıyla korunur; tetikleyici, JIT, ayarlar", () => {
    const jit = {
      Functions: 4,
      Options: { Inlining: false, Optimization: false, Expressions: true, Deforming: true },
      Timing: { Generation: { Deform: 0.1, Total: 0.5 }, Inlining: 0, Optimization: 0.2, Emission: 3, Total: 3.7 },
    };
    const { root, summary } = ok(
      wrap(
        {
          ...scan(),
          "Future Key": { nested: [1, 2] },
          "Actual Startup Time": 0.1,
          "Actual Total Time": 1,
          "Actual Rows": 100,
          "Actual Loops": 1,
          "WAL Records": 3,
        },
        {
          "Planning Time": 0.2,
          Triggers: [
            { "Trigger Name": "trg_audit", Relation: "t", Time: 2.5, Calls: 100 },
            { "Trigger Name": "RI_ConstraintTrigger_a_1", "Constraint Name": "fk_x", Relation: "u", Time: 1, Calls: 3 },
          ],
          JIT: jit,
          Settings: { work_mem: "64MB", enable_seqscan: "off" },
          "Query Identifier": 123456789,
          "Execution Time": 6.5,
        },
      ),
    );
    expect(root.details).toEqual([
      ["Relation Name", "t"],
      ["Alias", "t"],
      ["Future Key", { nested: [1, 2] }],
      ["WAL Records", 3],
    ]);
    expect(summary.triggers).toEqual([
      { name: "trg_audit", constraint: undefined, relation: "t", timeMs: 2.5, calls: 100 },
      { name: "RI_ConstraintTrigger_a_1", constraint: "fk_x", relation: "u", timeMs: 1, calls: 3 },
    ]);
    expect(summary.jit).toEqual({
      functions: 4,
      totalMs: 3.7,
      details: [
        ["Functions", 4],
        ["Options", jit.Options],
        ["Timing", jit.Timing],
      ],
    });
    expect(summary.settings).toEqual([
      ["work_mem", "64MB"],
      ["enable_seqscan", "off"],
    ]);
    expect(summary.details).toEqual([["Query Identifier", 123456789]]);
    expect(summary.totalMs).toBe(6.5);
  });

  it("tiplenen anahtarın değeri beklenmedik türdeyse ayrıntıda kalır", () => {
    const { root } = ok(wrap({ ...scan(), "Plan Width": "geniş" }));
    expect(root.planWidth).toBeNull();
    expect(new Map(root.details).get("Plan Width")).toBe("geniş");
  });

  it("tek nesne (dizi değil) ve ek öğeler de kabul edilir", () => {
    expect(ok(JSON.stringify({ Plan: scan() })).root.nodeType).toBe("Seq Scan");
    expect(ok(JSON.stringify([{ Plan: scan() }, { Plan: scan({ "Node Type": "Result" }) }])).root.nodeType).toBe(
      "Seq Scan",
    );
  });
});

describe("bozuk girdi", () => {
  it("geçersiz JSON → errInvalidJson", () => {
    expect(parsePlan("{oops")).toEqual({ error: tplan.errInvalidJson });
    expect(parsePlan("")).toEqual({ error: tplan.errInvalidJson });
  });
  it("plan içermeyen JSON → errNoPlan", () => {
    for (const s of ["[]", "null", "42", '"x"', "[{}]", '[{"Plan": 5}]', '{"Plan": {"Total Cost": 1}}', '[null]']) {
      expect(parsePlan(s), s).toEqual({ error: tplan.errNoPlan });
    }
  });
  it("Plans dizisi değilse ya da öğesi nesne değilse yok sayılır, ayrıntıda kalır", () => {
    const { root } = ok(wrap({ ...scan(), Plans: "garip" }));
    expect(root.children).toEqual([]);
    expect(new Map(root.details).get("Plans")).toBe("garip");
    const r2 = ok(wrap({ ...scan(), Plans: [5, scan({ "Relation Name": "z" })] })).root;
    expect(r2.children).toHaveLength(1);
    expect(r2.children[0].relation).toBe("z");
  });
});

describe("nodeLabel", () => {
  const lbl = (o: Record<string, unknown>) => ok(wrap(o)).root.label;
  it("PG metin biçimine yakın adlar", () => {
    expect(lbl({ "Node Type": "Index Scan", "Index Name": "idx", "Relation Name": "t", Alias: "t" })).toBe(
      "Index Scan using idx on t",
    );
    expect(
      lbl({
        "Node Type": "Index Only Scan",
        "Scan Direction": "Backward",
        "Index Name": "idx",
        "Relation Name": "t",
        Schema: "public",
        Alias: "x",
      }),
    ).toBe("Index Only Scan Backward using idx on public.t x");
    expect(lbl({ "Node Type": "Aggregate", Strategy: "Sorted", "Partial Mode": "Finalize" })).toBe(
      "Finalize GroupAggregate",
    );
    expect(lbl({ "Node Type": "Aggregate", Strategy: "Hashed", "Partial Mode": "Partial" })).toBe(
      "Partial HashAggregate",
    );
    expect(lbl({ "Node Type": "Aggregate", Strategy: "Mixed" })).toBe("MixedAggregate");
    expect(lbl({ "Node Type": "SetOp", Strategy: "Hashed", Command: "Except All" })).toBe("HashSetOp Except All");
    expect(lbl({ "Node Type": "Hash Join", "Join Type": "Anti" })).toBe("Hash Anti Join");
    expect(lbl({ "Node Type": "Merge Join", "Join Type": "Right Semi" })).toBe("Merge Right Semi Join");
    expect(lbl({ "Node Type": "ModifyTable", Operation: "Delete", "Relation Name": "t", Alias: "t" })).toBe(
      "Delete on t",
    );
    expect(lbl({ "Node Type": "Function Scan", "Function Name": "generate_series", Alias: "g" })).toBe(
      "Function Scan on generate_series g",
    );
    expect(lbl({ "Node Type": "Subquery Scan", Alias: "sq" })).toBe("Subquery Scan on sq");
    expect(lbl({ "Node Type": "Foreign Scan", "Async Capable": true, "Relation Name": "ft", Alias: "ft" })).toBe(
      "Async Foreign Scan on ft",
    );
    expect(lbl({ "Node Type": "Custom Scan", "Custom Plan Provider": "ColumnarScan" })).toBe("Custom Scan (ColumnarScan)");
  });
});

describe("initialExpanded", () => {
  /** count düğümlü, dalı 4 olan dengeli ağaç (genişlik öncelikli; i. düğümün ebeveyni ⌊(i−1)/4⌋). */
  function bigTree(count: number): string {
    const nodes: Record<string, unknown>[] = [];
    for (let i = 0; i < count; i++) {
      const node: Record<string, unknown> = {
        "Node Type": i % 2 ? "Hash Join" : "Nested Loop",
        "Join Type": "Inner",
        "Startup Cost": 0,
        "Total Cost": 1000 - i,
        "Plan Rows": 10,
        "Plan Width": 4,
        "Actual Startup Time": 0.01,
        "Actual Total Time": 1 + (i % 7),
        "Actual Rows": 1000,
        "Actual Loops": 1,
        "Shared Hit Blocks": 5,
        "Shared Read Blocks": 1,
        Filter: "(x > 1)",
      };
      nodes.push(node);
      if (i > 0) {
        const parent = nodes[Math.floor((i - 1) / 4)];
        ((parent.Plans ??= []) as unknown[]).push(node);
      }
    }
    return wrap(nodes[0], { "Planning Time": 1, "Execution Time": 2 });
  }

  it("200 altı: çocuklu tüm düğümler açık", () => {
    const { root } = ok(pg10);
    const exp = initialExpanded(root);
    const parents = flat(root).filter((n) => n.children.length > 0);
    expect([...exp].sort((a, b) => a - b)).toEqual(parents.map((n) => n.id));
  });

  it("200+ düğüm: yalnızca ilk 3 düzey (derinlik 0–2) açık", () => {
    const { root } = ok(bigTree(250));
    const all = flat(root);
    expect(all.length).toBe(250);
    const depth = new Map<number, number>();
    const visit = (n: PlanNode, d: number) => {
      depth.set(n.id, d);
      n.children.forEach((c) => visit(c, d + 1));
    };
    visit(root, 0);
    const exp = initialExpanded(root);
    for (const n of all) {
      const shouldOpen = n.children.length > 0 && depth.get(n.id)! < 3;
      expect(exp.has(n.id), `id ${n.id}`).toBe(shouldOpen);
    }
    expect(exp.has(0)).toBe(true);
  });

  it("performans: 1000 düğüm ayrıştırma + hesap < 50 ms", () => {
    const json = bigTree(1000);
    ok(json); // ısınma (JIT)
    const t0 = performance.now();
    const { root } = ok(json);
    hotNodes(root, 5);
    initialExpanded(root);
    const ms = performance.now() - t0;
    expect(flat(root)).toHaveLength(1000);
    expect(ms).toBeLessThan(50);
  });
});

describe("looksLikePlanResult", () => {
  const col = [{ name: "QUERY PLAN", typeName: "json" }];
  const plan = '[\n  {\n    "Plan": {\n      "Node Type": "Result"}}]';
  it("tek sütun QUERY PLAN (json), tek satır, plan metni", () => {
    expect(looksLikePlanResult(col, [[plan]])).toBe(true);
    expect(looksLikePlanResult(col, [[pg17]])).toBe(true);
  });
  it("diğer durumlar false", () => {
    expect(looksLikePlanResult([{ name: "QUERY PLAN", typeName: "text" }], [["Seq Scan on t"]])).toBe(false);
    expect(looksLikePlanResult([{ name: "plan", typeName: "json" }], [[plan]])).toBe(false);
    expect(looksLikePlanResult([...col, { name: "x", typeName: "int4" }], [[plan, "1"]])).toBe(false);
    expect(looksLikePlanResult(col, [])).toBe(false);
    expect(looksLikePlanResult(col, [[plan], [plan]])).toBe(false);
    expect(looksLikePlanResult(col, [[null]])).toBe(false);
    expect(looksLikePlanResult(col, [['{"a": 1}']])).toBe(false);
  });
});
