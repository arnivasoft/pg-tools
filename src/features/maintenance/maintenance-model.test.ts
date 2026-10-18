import { describe, expect, it } from "vitest";
import { phaseLabel } from "../../app/strings-maintenance";
import type { MaintenanceOp } from "../../ipc/bindings/MaintenanceOp";
import type { MaintenanceProgress } from "../../ipc/bindings/MaintenanceProgress";
import type { MaintenanceTarget } from "../../ipc/bindings/MaintenanceTarget";
import type { MaintenanceTargetKind } from "../../ipc/bindings/MaintenanceTargetKind";
import {
  createProgressPoller,
  defaultOperation,
  elapsedLabel,
  maintenanceRequest,
  messageSummary,
  needsCcnewNotice,
  normalize,
  opBlocked,
  optionStates,
  opsFor,
  progressView,
  parseVersionNum,
  splitFieldErrors,
} from "./maintenance-model";

const target = (over: Partial<MaintenanceTarget> = {}): MaintenanceTarget => ({
  kind: "table",
  schema: "s",
  name: "t",
  table: null,
  partitioned: false,
  system: false,
  clusterIndex: null,
  indexes: ["i1", "i2"],
  editable: true,
  unsupported: [],
  ...over,
});
type Vac = Extract<MaintenanceOp, { op: "vacuum" }>;
const vac = (over: Partial<Vac> = {}): Vac => ({ ...(defaultOperation("vacuum", target(), null) as Vac), ...over });

describe("opsFor (Rust ops_for aynası: crates/pgt-ddl/src/maintenance.rs)", () => {
  const all = ["vacuum", "analyze", "reindex", "cluster"];
  it("tablo", () => {
    for (const v of [100000, 130000, 140000, 150000, null]) expect(opsFor("table", false, v)).toEqual(all);
    expect(opsFor("table", true, 100000)).toEqual(["vacuum", "analyze"]);
    expect(opsFor("table", true, 130000)).toEqual(["vacuum", "analyze"]);
    expect(opsFor("table", true, 140000)).toEqual(["vacuum", "analyze", "reindex"]);
    expect(opsFor("table", true, 150000)).toEqual(all);
    expect(opsFor("table", true, null)).toEqual(all);
  });
  it("diğer türler", () => {
    for (const v of [100000, 140000, null]) {
      expect(opsFor("materialized_view", false, v)).toEqual(["vacuum", "analyze", "reindex"]);
      expect(opsFor("materialized_view", true, v)).toEqual(["vacuum", "analyze", "reindex"]);
      expect(opsFor("schema", false, v)).toEqual(["reindex"]);
      expect(opsFor("database", false, v)).toEqual(["vacuum", "analyze", "reindex"]);
      expect(opsFor("index", false, v)).toEqual(["reindex"]);
    }
    expect(opsFor("index", true, 130000)).toEqual([]);
    expect(opsFor("index", true, 140000)).toEqual(["reindex"]);
    expect(opsFor("index", true, null)).toEqual(["reindex"]);
  });
  it("kind türleri tam kapsanır", () => {
    const kinds: MaintenanceTargetKind[] = ["table", "materialized_view", "index", "schema", "database"];
    for (const k of kinds) expect(Array.isArray(opsFor(k, false, 170000))).toBe(true);
  });
});

describe("defaultOperation", () => {
  it("vacuum: yalnız ANALYZE açık", () => {
    expect(defaultOperation("vacuum", target(), 170000)).toEqual({
      op: "vacuum",
      full: false,
      freeze: false,
      analyze: true,
      verbose: false,
      disablePageSkipping: false,
      skipLocked: false,
      indexCleanup: null,
      parallel: null,
    });
  });
  it("analyze hepsi kapalı", () => {
    expect(defaultOperation("analyze", target(), 170000)).toEqual({ op: "analyze", verbose: false, skipLocked: false });
  });
  it("reindex: CONCURRENTLY PG 12+ ve sistem değilse açık", () => {
    const c = (t: MaintenanceTarget, v: number | null) =>
      (defaultOperation("reindex", t, v) as Extract<MaintenanceOp, { op: "reindex" }>).concurrently;
    expect(c(target(), 120000)).toBe(true);
    expect(c(target(), 110000)).toBe(false);
    expect(c(target(), null)).toBe(true);
    expect(c(target({ system: true }), 170000)).toBe(false);
  });
  it("cluster: clusterIndex, yoksa ilk indeks, yoksa null", () => {
    const u = (t: MaintenanceTarget) => (defaultOperation("cluster", t, 170000) as { usingIndex: string | null }).usingIndex;
    expect(u(target({ clusterIndex: "i2" }))).toBe("i2");
    expect(u(target())).toBe("i1");
    expect(u(target({ indexes: [] }))).toBeNull();
  });
});

describe("optionStates", () => {
  it("vacuum sürüm kapıları", () => {
    const s = (v: number | null) => optionStates(vac(), target(), v);
    expect(s(110000).skipLocked).toEqual({ visible: true, enabled: false, reason: "PostgreSQL 12 ve üzerini gerektirir" });
    expect(s(110000).indexCleanup!.enabled).toBe(false);
    expect(s(120000).skipLocked!.enabled).toBe(true);
    expect(s(120000).indexCleanup!.enabled).toBe(true);
    expect(s(120000).parallel!.reason).toBe("PostgreSQL 13 ve üzerini gerektirir");
    expect(s(130000).parallel!.enabled).toBe(true);
    expect(s(130000).indexCleanupAuto!.reason).toBe("PostgreSQL 14 ve üzerini gerektirir");
    expect(s(140000).indexCleanupAuto!.enabled).toBe(true);
  });
  it("sürüm bilinmiyorsa hiçbir kapı kapanmaz", () => {
    for (const st of Object.values(optionStates(vac(), target(), null))) expect(st).toEqual({ visible: true, enabled: true, reason: null });
  });
  it("FULL: parallel ve disablePageSkipping devre dışı", () => {
    const s = optionStates(vac({ full: true }), target(), 170000);
    expect(s.parallel).toEqual({ visible: true, enabled: false, reason: "FULL ile birlikte kullanılamaz" });
    expect(s.disablePageSkipping!.enabled).toBe(false);
    expect(s.disablePageSkipping!.reason).toBe("FULL ile birlikte kullanılamaz");
    expect(optionStates(vac(), target(), 170000).parallel!.enabled).toBe(true);
  });
  it("analyze skipLocked kapısı", () => {
    const op: MaintenanceOp = { op: "analyze", verbose: false, skipLocked: false };
    expect(optionStates(op, target(), 110000).skipLocked!.enabled).toBe(false);
    expect(optionStates(op, target(), 120000).skipLocked!.enabled).toBe(true);
  });
  it("reindex: kapı ve sistem nesnesi", () => {
    const op: MaintenanceOp = { op: "reindex", concurrently: false, verbose: false };
    expect(optionStates(op, target(), 110000).concurrently!.reason).toBe("PostgreSQL 12 ve üzerini gerektirir");
    expect(optionStates(op, target(), 120000).concurrently!.enabled).toBe(true);
    const sys = optionStates(op, target({ system: true }), 170000).concurrently!;
    expect(sys.enabled).toBe(false);
    expect(sys.reason).toBe("Sistem nesnelerinde kullanılamaz");
  });
  it("cluster USING: boş liste devre dışı", () => {
    const op: MaintenanceOp = { op: "cluster", verbose: false, usingIndex: null };
    expect(optionStates(op, target({ indexes: [] }), 170000).usingIndex).toEqual({
      visible: true,
      enabled: false,
      reason: "Tabloda seçilebilir indeks yok",
    });
    expect(optionStates(op, target(), 170000).usingIndex!.enabled).toBe(true);
  });
});

describe("normalize", () => {
  it("FULL açılınca parallel ve disablePageSkipping temizlenir", () => {
    const o = normalize(vac({ full: true, parallel: 4, disablePageSkipping: true }), target(), 170000) as Vac;
    expect(o.parallel).toBeNull();
    expect(o.disablePageSkipping).toBe(false);
    expect(o.full).toBe(true);
  });
  it("kapı altı seçenekler temizlenir", () => {
    const o = normalize(
      vac({ skipLocked: true, indexCleanup: "on", parallel: 2 }),
      target(),
      110000,
    ) as Vac;
    expect([o.skipLocked, o.indexCleanup, o.parallel]).toEqual([false, null, null]);
    expect((normalize(vac({ indexCleanup: "auto" }), target(), 130000) as Vac).indexCleanup).toBeNull();
    expect((normalize(vac({ indexCleanup: "on" }), target(), 130000) as Vac).indexCleanup).toBe("on");
    expect((normalize(vac({ indexCleanup: "auto", parallel: 0 }), target(), 140000) as Vac)).toMatchObject({
      indexCleanup: "auto",
      parallel: 0,
    });
  });
  it("analyze / reindex / cluster", () => {
    expect(normalize({ op: "analyze", verbose: true, skipLocked: true }, target(), 110000)).toEqual({
      op: "analyze",
      verbose: true,
      skipLocked: false,
    });
    expect(normalize({ op: "reindex", concurrently: true, verbose: false }, target({ system: true }), 170000)).toEqual({
      op: "reindex",
      concurrently: false,
      verbose: false,
    });
    expect(normalize({ op: "reindex", concurrently: true, verbose: false }, target(), 170000)).toMatchObject({
      concurrently: true,
    });
    expect(normalize({ op: "cluster", verbose: false, usingIndex: "yok" }, target({ clusterIndex: "i2" }), null)).toEqual({
      op: "cluster",
      verbose: false,
      usingIndex: "i2",
    });
    expect(normalize({ op: "cluster", verbose: false, usingIndex: "i2" }, target(), null)).toMatchObject({ usingIndex: "i2" });
  });
});

describe("maintenanceRequest", () => {
  it("operation alanı, hedef olduğu gibi", () => {
    const t = target();
    const op = defaultOperation("analyze", t, null);
    const object = { kind: "table" } as never;
    const r = maintenanceRequest(object, t, op);
    expect(r).toEqual({ op: "maintenance", object, target: t, operation: op });
    expect(r).not.toHaveProperty("def");
    if (r.op === "maintenance") expect(r.target).toBe(t);
  });
});

describe("progressView", () => {
  const p = (over: Partial<MaintenanceProgress>): MaintenanceProgress => ({
    phase: "scanning heap",
    done: null,
    total: null,
    unit: null,
    ...over,
  });
  it("total var: yüzde", () => {
    const v = progressView(p({ done: 25, total: 100, unit: "blocks" }), phaseLabel);
    expect(v.percent).toBe(25);
    expect(v.text).toBe("Tablo taranıyor — 25/100 blok (%25)");
    expect(progressView(p({ done: 150, total: 100, unit: "blocks" }), phaseLabel).percent).toBe(100);
  });
  it("total yok: belirsiz + sayaç", () => {
    const v = progressView(p({ phase: "writing new heap", done: 42, unit: "tuples" }), phaseLabel);
    expect(v.percent).toBeNull();
    expect(v.text).toBe("Yeni tablo yazılıyor — 42 satır");
    expect(progressView(p({ done: 0, total: 0, unit: "blocks" }), phaseLabel).percent).toBeNull();
  });
  it("sayaç da yok: yalnız aşama", () => {
    expect(progressView(p({ phase: "vacuuming indexes" }), phaseLabel)).toEqual({
      percent: null,
      text: "İndeksler temizleniyor",
    });
  });
  it("null: bekleniyor", () => {
    expect(progressView(null, phaseLabel)).toEqual({ percent: null, text: "İlerleme bilgisi bekleniyor…" });
  });
  it("bilinmeyen aşama olduğu gibi", () => {
    expect(progressView(p({ phase: "yeni aşama" }), phaseLabel).text).toBe("yeni aşama");
    expect(phaseLabel("toString")).toBe("toString");
    expect(phaseLabel("unknown")).toBe("Bilinmiyor");
  });
  it("spec §6.5 aşamalarının hepsi çevrilidir", () => {
    const known = [
      "initializing", "scanning heap", "vacuuming indexes", "vacuuming heap", "cleaning up indexes", "truncating heap",
      "performing final cleanup", "seq scanning heap", "index scanning heap", "sorting tuples", "writing new heap",
      "swapping relation files", "rebuilding index", "waiting for writers before build", "building index: scanning table",
      "building index: sorting live tuples", "building index: loading tuples in tree",
      "waiting for writers before validation", "index validation: scanning index", "index validation: sorting tuples",
      "index validation: scanning table", "waiting for old snapshots", "waiting for readers before marking dead",
      "waiting for readers before dropping", "acquiring sample rows", "acquiring inherited sample rows",
      "computing statistics", "computing extended statistics", "finalizing analyze", "unknown",
    ];
    for (const k of known) expect(phaseLabel(k), k).not.toBe(k);
  });
});

describe("messageSummary", () => {
  const m = (severity: string) => ({ severity, code: null, message: "x", detail: null, hint: null });
  it("WARNING sayılır, INFO (taşma) sayılmaz", () => {
    expect(messageSummary([m("WARNING"), m("NOTICE"), m("WARNING"), m("INFO")])).toEqual({ warnings: 2, total: 4 });
  });
  it("boş / tanımsız", () => {
    expect(messageSummary(undefined)).toEqual({ warnings: 0, total: 0 });
    expect(messageSummary([])).toEqual({ warnings: 0, total: 0 });
  });
});

describe("needsCcnewNotice", () => {
  const rc: MaintenanceOp = { op: "reindex", concurrently: true, verbose: false };
  it("yalnız CONCURRENTLY ve başarısız/iptal", () => {
    expect(needsCcnewNotice(rc, "failed")).toBe(true);
    expect(needsCcnewNotice(rc, "cancelled")).toBe(true);
    expect(needsCcnewNotice(rc, "applied")).toBe(false);
    expect(needsCcnewNotice({ ...rc, concurrently: false }, "failed")).toBe(false);
    expect(needsCcnewNotice({ op: "analyze", verbose: false, skipLocked: false }, "failed")).toBe(false);
  });
});

describe("elapsedLabel", () => {
  it("mm:ss", () => {
    expect(elapsedLabel(0)).toBe("00:00");
    expect(elapsedLabel(999)).toBe("00:00");
    expect(elapsedLabel(65_000)).toBe("01:05");
    expect(elapsedLabel(75 * 60_000 + 3000)).toBe("75:03");
    expect(elapsedLabel(-5)).toBe("00:00");
  });
});

describe("createProgressPoller", () => {
  const prog: MaintenanceProgress = { phase: "scanning heap", done: 1, total: 2, unit: "blocks" };
  function setup(fetchImpl: () => Promise<MaintenanceProgress | null>) {
    const timers: { cb: () => void; ms: number; live: boolean }[] = [];
    const updates: (MaintenanceProgress | null)[] = [];
    const stops: string[] = [];
    let calls = 0;
    const poller = createProgressPoller({
      fetch: () => {
        calls++;
        return fetchImpl();
      },
      schedule: (cb, ms) => {
        const t = { cb, ms, live: true };
        timers.push(t);
        return () => {
          t.live = false;
        };
      },
      onUpdate: (p) => updates.push(p),
      onStop: (r) => stops.push(r),
    });
    const fire = async () => {
      const t = timers.filter((x) => x.live).pop();
      if (t) t.live = false;
      t?.cb();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    };
    return { poller, timers, updates, stops, fire, calls: () => calls };
  }

  it("start ilk turu zamanlar; tur bitince sonrakini ~1 sn sonra", async () => {
    const s = setup(async () => prog);
    s.poller.start();
    expect(s.calls()).toBe(0);
    expect(s.timers).toHaveLength(1);
    expect(s.timers[0]!.ms).toBe(1000);
    await s.fire();
    expect(s.updates).toEqual([prog]);
    expect(s.timers).toHaveLength(2);
  });

  it("önceki yanıt gelmeden yeni istek yok", async () => {
    let resolve!: (p: MaintenanceProgress | null) => void;
    const s = setup(() => new Promise((r) => (resolve = r)));
    s.poller.start();
    await s.fire();
    expect(s.calls()).toBe(1);
    expect(s.timers).toHaveLength(1); // yanıt bekleniyor, yeni tur zamanlanmadı
    resolve(null);
    await Promise.resolve();
    await Promise.resolve();
    expect(s.updates).toEqual([null]);
    expect(s.timers).toHaveLength(2);
  });

  it("stop sonrası çağrı ve geri çağrı yok; geç yanıt atılır", async () => {
    let resolve!: (p: MaintenanceProgress | null) => void;
    const s = setup(() => new Promise((r) => (resolve = r)));
    s.poller.start();
    await s.fire();
    s.poller.stop();
    resolve(prog);
    await Promise.resolve();
    await Promise.resolve();
    expect(s.updates).toEqual([]);
    expect(s.timers.filter((t) => t.live)).toHaveLength(0);
    // zamanlanmış tur varken stop: iptal edilir
    const s2 = setup(async () => prog);
    s2.poller.start();
    s2.poller.stop();
    expect(s2.timers[0]!.live).toBe(false);
    expect(s2.calls()).toBe(0);
  });

  it("3 ardışık hata onStop; araya başarı sayacı sıfırlar", async () => {
    let n = 0;
    const seq = ["err", "err", "ok", "err", "err", "err"];
    const s = setup(async () => {
      if (seq[n++] === "err") throw new Error("x");
      return prog;
    });
    s.poller.start();
    for (let i = 0; i < 5; i++) await s.fire();
    expect(s.stops).toEqual([]);
    await s.fire();
    expect(s.stops).toEqual(["errors"]);
    expect(s.timers.filter((t) => t.live)).toHaveLength(0);
    expect(s.updates).toEqual([prog]);
  });

  it("start tekrarlı çağrıda ikinci zamanlayıcı kurmaz", () => {
    const s = setup(async () => prog);
    s.poller.start();
    s.poller.start();
    expect(s.timers).toHaveLength(1);
  });
});

describe("opBlocked", () => {
  it("cluster: sistem nesnesi ve indeks yokluğu", () => {
    expect(opBlocked("cluster", target({ system: true }))).toBe("Sistem nesnelerinde CLUSTER kullanılamaz");
    expect(opBlocked("cluster", target({ indexes: [] }))).toContain("kümeleme indeksi yok");
    expect(opBlocked("cluster", target({ indexes: [], clusterIndex: "i1" }))).toBeNull();
    expect(opBlocked("cluster", target())).toBeNull();
  });
  it("diğer işlemler engellenmez", () => {
    for (const o of ["vacuum", "analyze", "reindex"] as const) expect(opBlocked(o, target({ system: true, indexes: [] }))).toBeNull();
  });
});

describe("normalize ek kurallar", () => {
  it("PARALLEL 0–1024 tamsayıya sıkıştırılır", () => {
    const par = (n: number) => (normalize(vac({ parallel: n }), target(), 170000) as Vac).parallel;
    expect(par(2.9)).toBe(2);
    expect(par(-3)).toBe(0);
    expect(par(5000)).toBe(1024);
    expect(par(1024)).toBe(1024);
    expect(par(Number.NaN)).toBeNull();
  });
  it("USING boş + kümeleme indeksi yok -> ilk indeks", () => {
    const op: MaintenanceOp = { op: "cluster", verbose: false, usingIndex: null };
    expect(normalize(op, target(), null)).toMatchObject({ usingIndex: "i1" });
    expect(normalize(op, target({ clusterIndex: "i2" }), null)).toMatchObject({ usingIndex: null });
    expect(normalize(op, target({ indexes: [] }), null)).toMatchObject({ usingIndex: null });
  });
  it("poller: onUpdate fırlatması hata sayılmaz", async () => {
    const timers: (() => void)[] = [];
    let stopped = 0;
    const p = createProgressPoller({
      fetch: async () => null,
      schedule: (cb) => {
        timers.push(cb);
        return () => {};
      },
      onUpdate: () => {
        throw new Error("ui");
      },
      onStop: () => stopped++,
    });
    p.start();
    for (let i = 0; i < 4; i++) {
      timers.shift()?.();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    }
    expect(stopped).toBe(0);
    expect(timers.length).toBeGreaterThan(0);
  });
});

describe("parseVersionNum (sunucu düğümü `detail` = ServerInfo.version)", () => {
  it("PG 10+: major*10000 + minor", () => {
    expect(parseVersionNum("16.4")).toBe(160004);
    expect(parseVersionNum("10.23")).toBe(100023);
    expect(parseVersionNum("17beta1")).toBe(170000);
    expect(parseVersionNum("18devel")).toBe(180000);
  });
  it("PG < 10: major.minor.patch", () => {
    expect(parseVersionNum("9.6.24")).toBe(90624);
  });
  it("bilinmeyen/boş → null (sürüm kapısı yok; arka uç doğrular)", () => {
    expect(parseVersionNum(null)).toBeNull();
    expect(parseVersionNum("")).toBeNull();
    expect(parseVersionNum("abc")).toBeNull();
  });
});

describe("splitFieldErrors", () => {
  it("options.<alan> seçeneğe, diğerleri üst şeride", () => {
    const r = splitFieldErrors([
      { path: "options.parallel", message: "p" },
      { path: "options.skipLocked", message: "s" },
      { path: "object", message: "o" },
      { path: "op", message: "x" },
      { path: "weird", message: "w" },
    ]);
    expect(r.byOption).toEqual({ parallel: ["p"], skipLocked: ["s"] });
    expect(r.top).toEqual(["o", "x", "w"]);
  });
});
