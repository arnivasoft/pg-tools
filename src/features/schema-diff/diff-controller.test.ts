import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tsd } from "../../app/strings-diff";
import type { DdlTarget } from "../../ipc/bindings/DdlTarget";
import type { DiffDatabase } from "../../ipc/bindings/DiffDatabase";
import type { DiffDetail } from "../../ipc/bindings/DiffDetail";
import type { DiffEvent } from "../../ipc/bindings/DiffEvent";
import type { DiffItem } from "../../ipc/bindings/DiffItem";
import type { DiffResult } from "../../ipc/bindings/DiffResult";
import type { DiffScript } from "../../ipc/bindings/DiffScript";
import type { DiffSide } from "../../ipc/bindings/DiffSide";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import { createDiffController, DETAIL_DELAY_MS, type DiffControllerDeps, type DiffTabCtl, type Picker } from "./diff-controller";

// Fikstürler yapay veridir. Ayırt edici işaretler: sızıntı testi bunları bildirim argümanlarında arar.
const PATH = "C:\\gizli\\YOLMARK.sql";
const SQL = "ALTER TABLE SQLMARK ADD COLUMN x int;";

const DBS: DiffDatabase[] = [
  { oid: 7, name: "app" },
  { oid: 8, name: "arsiv" },
];
const SCHEMAS: LookupItem[] = [
  { value: "Şube", detail: null },
  { value: "public", detail: null },
];
const schemaTarget: DdlTarget = {
  serverId: "s1", databaseOid: 7, object: { kind: "schema", oid: 100 }, create: null, focus: null, refreshNodeId: null,
};

function item(o: Partial<DiffItem> = {}): DiffItem {
  return {
    key: "k", kind: "table", targetKind: null, name: "t", table: null, signature: null, status: "different",
    problem: null, risk: "safe", notes: [], errors: [], selectable: true, defaultSelected: true, ...o,
  };
}
// Varsayılan filtrede görünenler: a, b, d (c ve e "Aynı").
const ITEMS: DiffItem[] = [
  item({ key: "a", name: "a" }),
  item({ key: "b", name: "b", status: "only_source" }),
  item({ key: "c", name: "c", status: "same", defaultSelected: false }),
  item({ key: "d", name: "d", problem: "manual", selectable: false, defaultSelected: false }),
  item({ key: "e", name: "e", status: "same", defaultSelected: true }),
];
const RESULT: DiffResult = { items: ITEMS, sourceVersion: "16", targetVersion: "16", warnings: [], elapsedMs: 10 };
const SCRIPT: DiffScript = { sql: SQL, transactional: true, warnings: [], risk: "safe", statements: 1 };
const SERVERS = ["s1", "s2", "s3"].map((id) => ({ id, name: `ad ${id}` })) as unknown as ReturnType<DiffControllerDeps["servers"]>;
const detailOf = (k: string): DiffDetail => ({ sourceText: `src ${k}`, targetText: `tgt ${k}`, lines: [] });

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((a, b) => {
    resolve = a;
    reject = b;
  });
  return { promise, resolve, reject };
}

const flush = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};

const err = (code: string, message = `msg ${code}`) => Object.assign(new Error(message), { code, message, detail: null, hint: null });

interface Start {
  diffId: string;
  source: DiffSide;
  target: DiffSide;
  onEvent: (e: DiffEvent) => void;
  d: ReturnType<typeof deferred<DiffResult>>;
}

function setup() {
  const starts: Start[] = [];
  const api = {
    schemaDiffDatabases: vi.fn(async (_s: string) => DBS),
    schemaDiffStart: vi.fn((diffId: string, source: DiffSide, target: DiffSide, _o: unknown, onEvent: (e: DiffEvent) => void) => {
      const d = deferred<DiffResult>();
      starts.push({ diffId, source, target, onEvent, d });
      return d.promise;
    }),
    schemaDiffCancel: vi.fn(async (_id: string) => null),
    schemaDiffDetail: vi.fn(async (_id: string, k: string) => detailOf(k)),
    schemaDiffScript: vi.fn(async (_id: string, _k: string[]) => SCRIPT),
    schemaDiffSave: vi.fn(async (_id: string, _p: string) => null),
    schemaDiffClose: vi.fn(async (_id: string) => null),
    ddlTargetOf: vi.fn(async (_n: string) => schemaTarget),
    ddlLookup: vi.fn(async () => SCHEMAS),
  };
  const timers: { fn: () => void; ms: number; live: boolean }[] = [];
  let tabN = 0;
  let idN = 0;
  const tabs = { openSchemaDiff: vi.fn((_i: { title: string }) => `s${++tabN}`), rename: vi.fn() };
  const picker: Picker = { save: vi.fn(async () => PATH as string | null) };
  const deps: DiffControllerDeps = {
    api: api as unknown as DiffControllerDeps["api"],
    servers: () => SERVERS,
    nodeLabel: (id) => (id === "sch" ? "Şube" : null),
    connect: vi.fn(async () => true),
    tabs: tabs as unknown as DiffControllerDeps["tabs"],
    openScript: vi.fn(async () => {}) as unknown as DiffControllerDeps["openScript"],
    copy: vi.fn(async () => {}),
    notify: vi.fn(),
    pick: vi.fn(async () => picker),
    newId: () => `d${++idN}`,
    now: () => new Date(2026, 9, 31, 9, 5),
    setTimer: (fn, ms) => {
      const h = { fn, ms, live: true };
      timers.push(h);
      return h;
    },
    clearTimer: (h) => void ((h as { live: boolean }).live = false),
  };
  let dispose = () => {};
  const ctl: DiffTabCtl = createRoot((d) => {
    dispose = d;
    return createDiffController(deps);
  });
  /** Bekleyen zamanlayıcıları çalıştırır. */
  const fire = async () => {
    for (const h of timers.splice(0)) if (h.live) h.fn();
    await flush();
  };
  const m = (id = "s1") => ctl.model(id)!;
  /** Şema düğümünden açar; hedef şeması "public" seçilir (karşılaştırılabilir). */
  const ready = async () => {
    await ctl.open("sch");
    await flush();
    ctl.setSchema("s1", "target", "public");
  };
  /** `ready` + karşılaştırma başarıyla biter. */
  const compared = async () => {
    await ready();
    void ctl.compare("s1");
    await flush();
    starts[0].d.resolve(RESULT);
    await flush();
    await fire();
  };
  return { api, ctl, deps, tabs, picker, timers, starts, fire, m, ready, compared, dispose: () => dispose() };
}

type S = ReturnType<typeof setup>;
const ev = (side: "source" | "target", loaded: number, total: number): DiffEvent => ({ event: "progress", side, loaded, total });

describe("diff-controller", () => {
  let spies: ReturnType<typeof vi.spyOn>[] = [];
  let cur: S | null = null;
  const mk = () => (cur = setup());
  beforeEach(() => {
    spies = (["log", "info", "warn", "error", "debug", "trace"] as const).map((k) => vi.spyOn(console, k));
  });
  afterEach(() => {
    cur?.dispose();
    cur = null;
    for (const s of spies) {
      expect(s).not.toHaveBeenCalled();
      s.mockRestore();
    }
  });

  describe("açılış", () => {
    it("şema düğümü → kaynak {s1, 7, Şube}, hedef {s1, 7, null}; listeler yüklenir", async () => {
      const s = mk();
      await s.ctl.open("sch");
      expect(s.tabs.openSchemaDiff).toHaveBeenCalledWith({ title: tsd.title.initial });
      expect(s.ctl.ids()).toEqual(["s1"]);
      await flush();
      const m = s.m();
      expect([m.source.serverId, m.source.databaseOid, m.source.schema]).toEqual(["s1", 7, "Şube"]);
      expect([m.target.serverId, m.target.databaseOid, m.target.schema]).toEqual(["s1", 7, null]);
      expect(s.deps.connect).toHaveBeenCalledWith("s1");
      expect(m.source.databases).toEqual(DBS);
      expect(m.target.databases).toEqual(DBS);
      expect(m.source.schemas).toEqual(["Şube", "public"]);
      expect(m.target.schemas).toEqual(["Şube", "public"]);
      expect(m.source.loadingLists).toBe(false);
      expect(m.target.loadingLists).toBe(false);
      expect(s.api.ddlLookup).toHaveBeenCalledWith("s1", 7, { kind: "schemas" });
      expect(m.phase).toBe("idle");
      expect(m.options).toEqual({ ownership: false, tablespace: false, comments: true });
    });

    it("connect false → databasesError, sekme açık", async () => {
      const s = mk();
      (s.deps.connect as ReturnType<typeof vi.fn>).mockResolvedValue(false);
      await s.ctl.open("sch");
      await flush();
      expect(s.m().source.databasesError).toBe("Sunucuya bağlanılamadı.");
      expect(s.m().source.databases).toBeNull();
      expect(s.ctl.ids()).toEqual(["s1"]);
      expect(s.api.schemaDiffDatabases).not.toHaveBeenCalled();
    });

    it("ddlTargetOf hatası → error", async () => {
      const s = mk();
      s.api.ddlTargetOf.mockRejectedValueOnce(err("internal", "düğüm yok"));
      await s.ctl.open("sch");
      await flush();
      expect(s.m().error).toBe("düğüm yok");
      expect(s.ctl.ids()).toEqual(["s1"]);
    });
  });

  describe("seçiciler", () => {
    it("sunucu değişince veritabanı ve şema sıfırlanır", async () => {
      const s = mk();
      await s.ready();
      const p = s.ctl.setServer("s1", "source", "s2");
      expect(s.m().source.databaseOid).toBeNull();
      expect(s.m().source.schema).toBeNull();
      expect(s.m().source.schemas).toBeNull();
      await p;
      expect(s.deps.connect).toHaveBeenLastCalledWith("s2");
      expect(s.m().source.serverId).toBe("s2");
      expect(s.m().source.databases).toEqual(DBS);
    });

    it("eski sunucunun geç gelen veritabanı listesi yazılmaz", async () => {
      const s = mk();
      await s.ready();
      const late = deferred<DiffDatabase[]>();
      s.api.schemaDiffDatabases.mockReturnValueOnce(late.promise);
      const p1 = s.ctl.setServer("s1", "source", "s2");
      await flush();
      const p2 = s.ctl.setServer("s1", "source", "s3");
      await p2;
      late.resolve([{ oid: 99, name: "eski" }]);
      await p1;
      await flush();
      expect(s.m().source.serverId).toBe("s3");
      expect(s.m().source.databases).toEqual(DBS);
    });

    it("setDatabase şemayı sıfırlar ve şema listesini yükler", async () => {
      const s = mk();
      await s.ready();
      await s.ctl.setDatabase("s1", "target", 8);
      expect(s.m().target.schema).toBeNull();
      expect(s.api.ddlLookup).toHaveBeenLastCalledWith("s1", 8, { kind: "schemas" });
      expect(s.m().target.schemas).toEqual(["Şube", "public"]);
    });

    it("swap tarafları değiştirir", async () => {
      const s = mk();
      await s.ready();
      s.ctl.swap("s1");
      expect(s.m().source.schema).toBe("public");
      expect(s.m().target.schema).toBe("Şube");
      expect(s.m().target.databases).toEqual(DBS);
    });

    it("sonuç varken setOption ve seçici değişikliği → stale", async () => {
      const s = mk();
      await s.compared();
      expect(s.m().stale).toBe(false);
      s.ctl.setOption("s1", "ownership", true);
      expect(s.m().options.ownership).toBe(true);
      expect(s.m().stale).toBe(true);
    });

    it("compareBlocked === same iken compare çağrı yapmaz", async () => {
      const s = mk();
      await s.ready();
      s.ctl.setSchema("s1", "target", "Şube");
      await s.ctl.compare("s1");
      expect(s.api.schemaDiffStart).not.toHaveBeenCalled();
      expect(s.m().phase).toBe("idle");
    });
  });

  describe("karşılaştırma", () => {
    it("ilerleme, sonuç, seçim, başlık", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      expect(s.m().phase).toBe("running");
      const st = s.starts[0];
      expect(st.diffId).toBe("d1");
      expect(st.source).toEqual({ serverId: "s1", databaseOid: 7, schema: "Şube" });
      expect(st.target).toEqual({ serverId: "s1", databaseOid: 7, schema: "public" });
      st.onEvent(ev("source", 0, 3));
      expect(s.m().progress.source).toEqual({ loaded: 0, total: 3 });
      st.onEvent(ev("target", 3, 3));
      st.onEvent(ev("source", 3, 3));
      expect(s.m().progress).toEqual({ source: { loaded: 3, total: 3 }, target: { loaded: 3, total: 3 } });
      st.d.resolve(RESULT);
      await flush();
      const m = s.m();
      expect(m.phase).toBe("done");
      expect(m.result?.items.length).toBe(5);
      expect([...m.selected].sort()).toEqual(["a", "b", "e"]);
      expect(m.focused).toBe("a");
      expect(m.stale).toBe(false);
      expect(s.tabs.rename).toHaveBeenCalledWith("s1", "app.Şube ↔ app.public");
    });

    it("ikinci compare önce eski diffId'yi kapatır; ilk işin geç olayı yazılmaz", async () => {
      const s = mk();
      await s.compared();
      const first = s.starts[0];
      void s.ctl.compare("s1");
      await flush();
      expect(s.api.schemaDiffClose.mock.calls).toEqual([["d1"]]);
      expect(s.api.schemaDiffClose.mock.invocationCallOrder[0]).toBeLessThan(s.api.schemaDiffStart.mock.invocationCallOrder[1]);
      expect(s.m().diffId).toBe("d2");
      expect(s.m().result).toBeNull();
      expect(s.m().selected.size).toBe(0);
      // Kanal, yerleşmiş ilk işin olayını yeni karşılaştırma başladıktan sonra teslim ederse:
      expect(() => first.onEvent(ev("source", 1, 9))).not.toThrow();
      await flush();
      expect(s.m().progress.source).toBeNull();
      expect(s.m().phase).toBe("running");
      expect(s.m().result).toBeNull();
      expect(s.api.schemaDiffCancel).not.toHaveBeenCalled();
      // İkinci iş biter; eski diffId bir daha kapatılmaz.
      s.starts[1].d.resolve(RESULT);
      await flush();
      expect(s.m().phase).toBe("done");
      s.ctl.close("s1");
      expect(s.api.schemaDiffClose.mock.calls).toEqual([["d1"], ["d2"]]);
    });

    it("çalışırken ikinci compare engellenir; ilk işin sonucu ancak kendi kuşağına yazılır", async () => {
      // Uçuştaki eski kuşak sonucu yalnız kapatma yoluyla oluşabilir (çalışırken compare engelli; bkz. kapatma testleri).
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      await s.ctl.compare("s1");
      expect(s.api.schemaDiffStart).toHaveBeenCalledTimes(1);
      expect(s.api.schemaDiffClose).not.toHaveBeenCalled();
      s.starts[0].d.resolve(RESULT);
      await flush();
      expect(s.m().phase).toBe("done");
      expect(s.m().diffId).toBe("d1");
    });

    it("diğer hata → error, phase idle", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      s.starts[0].d.reject(err("schema_not_found"));
      await flush();
      expect(s.m().phase).toBe("idle");
      expect(s.m().error).toBe(tsd.errors.schema_not_found);
    });
  });

  describe("iptal", () => {
    it("ilk olaydan sonra cancel → schemaDiffCancel(diffId) hemen", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      s.starts[0].onEvent(ev("source", 0, 3));
      s.ctl.cancel("s1");
      expect(s.m().cancelRequested).toBe(true);
      expect(s.api.schemaDiffCancel).toHaveBeenCalledWith("d1");
      s.ctl.cancel("s1");
      expect(s.api.schemaDiffCancel).toHaveBeenCalledTimes(1);
    });

    it("cancelled hata kodu → banner İptal edildi., result null, diffId null", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      s.starts[0].onEvent(ev("source", 0, 3));
      s.ctl.cancel("s1");
      s.starts[0].d.reject(err("cancelled"));
      await flush();
      expect(s.m().banner).toBe("İptal edildi.");
      expect(s.m().result).toBeNull();
      expect(s.m().diffId).toBeNull();
      expect(s.m().phase).toBe("idle");
      expect(s.m().cancelRequested).toBe(false);
    });

    it("iptal isteğinden sonra başarı gelirse sonuç gösterilir", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      s.starts[0].onEvent(ev("source", 0, 3));
      s.ctl.cancel("s1");
      s.starts[0].d.resolve(RESULT);
      await flush();
      expect(s.m().phase).toBe("done");
      expect(s.m().result).not.toBeNull();
      expect(s.m().cancelRequested).toBe(false);
    });

    it("ilk olaydan önce cancel → hemen bir kez, bekleyen de ilk olayda bir kez (en çok iki)", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      s.ctl.cancel("s1");
      s.ctl.cancel("s1");
      expect(s.m().cancelRequested).toBe(true);
      expect(s.api.schemaDiffCancel.mock.calls).toEqual([["d1"]]);
      s.starts[0].onEvent(ev("source", 0, 3));
      expect(s.api.schemaDiffCancel.mock.calls).toEqual([["d1"], ["d1"]]);
      s.starts[0].onEvent(ev("target", 0, 3));
      s.ctl.cancel("s1");
      expect(s.api.schemaDiffCancel).toHaveBeenCalledTimes(2);
    });

    it("ilk olaydan önce cancel ve olay gelmeden start döner → yalnız hemen gönderilen, bekleyen düşer", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      s.ctl.cancel("s1");
      s.starts[0].d.reject(err("schema_not_found"));
      await flush();
      expect(s.api.schemaDiffCancel).toHaveBeenCalledTimes(1);
      expect(s.m().error).toBe(tsd.errors.schema_not_found);
      // Geç (kayıt sonrası) bir olay da artık iptal göndermez.
      s.starts[0].onEvent(ev("source", 0, 3));
      expect(s.api.schemaDiffCancel).toHaveBeenCalledTimes(1);
    });
  });

  describe("ayrıntı", () => {
    it("art arda üç focus → tek schemaDiffDetail (son anahtar)", async () => {
      const s = mk();
      await s.compared();
      s.api.schemaDiffDetail.mockClear();
      s.ctl.focus("s1", "b");
      s.ctl.focus("s1", "d");
      s.ctl.focus("s1", "c");
      expect(s.timers.filter((t) => t.live).length).toBe(1);
      expect(s.timers.find((t) => t.live)?.ms).toBe(DETAIL_DELAY_MS);
      await s.fire();
      expect(s.api.schemaDiffDetail.mock.calls).toEqual([["d1", "c"]]);
      expect(s.m().detail).toEqual({ key: "c", data: detailOf("c"), error: null });
    });

    it("sıra dışı dönen eski yanıt yazılmaz", async () => {
      const s = mk();
      await s.compared();
      const da = deferred<DiffDetail>();
      const db = deferred<DiffDetail>();
      s.api.schemaDiffDetail.mockReturnValueOnce(da.promise).mockReturnValueOnce(db.promise);
      s.ctl.focus("s1", "b");
      await s.fire();
      s.ctl.focus("s1", "d");
      await s.fire();
      db.resolve(detailOf("d"));
      await flush();
      da.resolve(detailOf("b"));
      await flush();
      expect(s.m().focused).toBe("d");
      expect(s.m().detail?.key).toBe("d");
      expect(s.m().detail?.data).toEqual(detailOf("d"));
    });

    it("önbellekteki anahtar ikinci kez istenmez", async () => {
      const s = mk();
      await s.compared();
      // `compared` ilk görünen öğe (a) için ayrıntıyı zaten aldı.
      expect(s.api.schemaDiffDetail.mock.calls).toEqual([["d1", "a"]]);
      s.ctl.focus("s1", "b");
      await s.fire();
      s.ctl.focus("s1", "a");
      expect(s.m().detail).toEqual({ key: "a", data: detailOf("a"), error: null });
      await s.fire();
      expect(s.api.schemaDiffDetail).toHaveBeenCalledTimes(2);
    });

    it("moveFocus görünen öğelerde ilerler, filtre dışını atlar", async () => {
      const s = mk();
      await s.compared();
      expect(s.m().focused).toBe("a");
      s.ctl.moveFocus("s1", 1);
      expect(s.m().focused).toBe("b");
      s.ctl.moveFocus("s1", 1);
      expect(s.m().focused).toBe("d"); // c ("Aynı") filtre dışı
      s.ctl.moveFocus("s1", 1);
      expect(s.m().focused).toBe("d");
      s.ctl.moveFocus("s1", "home");
      expect(s.m().focused).toBe("a");
      s.ctl.moveFocus("s1", "end");
      expect(s.m().focused).toBe("d");
      s.ctl.moveFocus("s1", "home");
      // "d" yalnız d'yi gösterir (tür etiketi "Tablo" d içermez); odak (a) görünmüyor → +1 ilk görünene gider.
      s.ctl.setSearch("s1", "D");
      s.ctl.moveFocus("s1", 1);
      expect(s.m().focused).toBe("d");
    });

    it("diff_not_found ayrıntıda → banner gone, result null", async () => {
      const s = mk();
      await s.compared();
      s.api.schemaDiffDetail.mockRejectedValueOnce(err("diff_not_found"));
      s.ctl.focus("s1", "b");
      await s.fire();
      expect(s.m().banner).toBe(tsd.gone);
      expect(s.m().result).toBeNull();
      expect(s.m().diffId).toBeNull();
      expect(s.m().phase).toBe("idle");
      expect(s.m().script).toBeNull();
    });
  });

  describe("betik", () => {
    it("scriptBlocked (noSelection, stale) iken çağrı yok", async () => {
      const s = mk();
      await s.compared();
      s.ctl.selectAll("s1", false);
      expect(s.m().selected.size).toBe(0);
      await s.ctl.buildScript("s1");
      expect(s.api.schemaDiffScript).not.toHaveBeenCalled();
      s.ctl.selectAll("s1", true);
      s.ctl.setOption("s1", "comments", false);
      await s.ctl.buildScript("s1");
      expect(s.api.schemaDiffScript).not.toHaveBeenCalled();
      expect(s.m().script).toBeNull();
    });

    it("gizli seçili anahtar istekte var; sonuç diyaloğa yazılır", async () => {
      const s = mk();
      await s.compared();
      // e "Aynı" (filtre dışı) ama varsayılan seçili.
      await s.ctl.buildScript("s1");
      expect(s.api.schemaDiffScript).toHaveBeenCalledWith("d1", ["a", "b", "e"]);
      expect(s.m().script).toEqual({ data: SCRIPT, error: null, busy: false });
    });

    it("toggle ve selectVisible", async () => {
      const s = mk();
      await s.compared();
      s.ctl.toggle("s1", "a");
      expect(s.m().selected.has("a")).toBe(false);
      s.ctl.toggle("s1", "d"); // seçilemez
      expect(s.m().selected.has("d")).toBe(false);
      s.ctl.selectVisible("s1", false);
      expect([...s.m().selected]).toEqual(["e"]);
      s.ctl.selectVisible("s1", true);
      expect([...s.m().selected].sort()).toEqual(["a", "b", "e"]);
    });

    it("openInQueryTool hedefle çağrılır, diyalog kapanır", async () => {
      const s = mk();
      await s.ready();
      await s.ctl.setDatabase("s1", "target", 8);
      s.ctl.setSchema("s1", "target", "public");
      void s.ctl.compare("s1");
      await flush();
      s.starts[0].d.resolve(RESULT);
      await flush();
      await s.ctl.buildScript("s1");
      await s.ctl.openInQueryTool("s1");
      expect(s.deps.openScript).toHaveBeenCalledWith({ serverId: "s1", databaseOid: 8, database: "arsiv", text: SQL });
      expect(s.m().script).toBeNull();
    });

    it("copyScript → copy(sql) ve sabit bildirim", async () => {
      const s = mk();
      await s.compared();
      await s.ctl.buildScript("s1");
      await s.ctl.copyScript("s1");
      expect(s.deps.copy).toHaveBeenCalledWith(SQL);
      expect(s.deps.notify).toHaveBeenCalledWith(tsd.script.copied, "success");
    });

    it("saveScript: vazgeç → kayıt yok; yol → kayıt ve yol içermeyen bildirim", async () => {
      const s = mk();
      await s.compared();
      await s.ctl.buildScript("s1");
      (s.picker.save as ReturnType<typeof vi.fn>).mockResolvedValueOnce(null);
      await s.ctl.saveScript("s1");
      expect(s.api.schemaDiffSave).not.toHaveBeenCalled();
      expect(s.deps.notify).not.toHaveBeenCalled();
      await s.ctl.saveScript("s1");
      expect(s.picker.save).toHaveBeenLastCalledWith({
        defaultPath: "app_Şube__app_public_20261031_0905.sql",
        filters: [{ name: "SQL", extensions: ["sql"] }],
      });
      expect(s.api.schemaDiffSave).toHaveBeenCalledWith("d1", PATH);
      expect(s.deps.notify).toHaveBeenCalledWith(tsd.script.saved, "success");
      const all = JSON.stringify((s.deps.notify as ReturnType<typeof vi.fn>).mock.calls);
      expect(all).not.toContain("YOLMARK");
      expect(all).not.toContain("SQLMARK");
    });

    it("write_failed → script.error", async () => {
      const s = mk();
      await s.compared();
      await s.ctl.buildScript("s1");
      s.api.schemaDiffSave.mockRejectedValueOnce(err("write_failed", "os hatası YOLMARK"));
      await s.ctl.saveScript("s1");
      expect(s.m().script?.error).toBe("Dosya yazılamadı.");
      expect(s.deps.notify).not.toHaveBeenCalled();
    });

    it("diff_not_found betikte → gone", async () => {
      const s = mk();
      await s.compared();
      s.api.schemaDiffScript.mockRejectedValueOnce(err("diff_not_found"));
      await s.ctl.buildScript("s1");
      expect(s.m().banner).toBe(tsd.gone);
      expect(s.m().result).toBeNull();
      expect(s.m().script).toBeNull();
    });
  });

  describe("kapatma", () => {
    it("ilk olaydan sonra çalışırken close → cancel + close birer kez; geç sonuç ve olay sessiz", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      const st = s.starts[0];
      st.onEvent(ev("source", 0, 3));
      s.ctl.close("s1");
      expect(s.ctl.model("s1")).toBeNull();
      expect(s.ctl.ids()).toEqual([]);
      expect(s.api.schemaDiffCancel.mock.calls).toEqual([["d1"]]);
      expect(() => st.onEvent(ev("target", 1, 3))).not.toThrow();
      st.d.resolve(RESULT);
      await flush();
      expect(s.api.schemaDiffClose.mock.calls).toEqual([["d1"]]);
      expect(s.ctl.model("s1")).toBeNull();
      expect(s.tabs.rename).not.toHaveBeenCalled();
      s.ctl.close("s1");
      expect(s.api.schemaDiffClose).toHaveBeenCalledTimes(1);
    });

    it("ilk olaydan önce close → cancel hemen ve ilk olayda, close yerleşince bir kez", async () => {
      const s = mk();
      await s.ready();
      void s.ctl.compare("s1");
      await flush();
      const st = s.starts[0];
      s.ctl.close("s1");
      expect(s.api.schemaDiffCancel.mock.calls).toEqual([["d1"]]);
      expect(s.api.schemaDiffClose).not.toHaveBeenCalled();
      expect(() => st.onEvent(ev("source", 0, 3))).not.toThrow();
      expect(s.api.schemaDiffCancel.mock.calls).toEqual([["d1"], ["d1"]]);
      st.onEvent(ev("source", 1, 3));
      expect(s.api.schemaDiffCancel).toHaveBeenCalledTimes(2);
      expect(s.api.schemaDiffClose).not.toHaveBeenCalled();
      st.d.reject(err("cancelled"));
      await flush();
      expect(s.api.schemaDiffClose.mock.calls).toEqual([["d1"]]);
      expect(s.ctl.model("s1")).toBeNull();
    });

    it("çalışmıyorken close → close(diffId) hemen; zamanlayıcı temizlenir; geç ayrıntı sessiz", async () => {
      const s = mk();
      await s.compared();
      s.ctl.focus("s1", "b");
      const live = s.timers.filter((t) => t.live);
      expect(live.length).toBe(1);
      s.ctl.close("s1");
      expect(live[0].live).toBe(false);
      expect(s.api.schemaDiffClose.mock.calls).toEqual([["d1"]]);
      expect(s.api.schemaDiffCancel).not.toHaveBeenCalled();
      await s.fire();
      expect(s.ctl.model("s1")).toBeNull();
    });

    it("kapandıktan sonra gelen seçici yanıtı hata vermez", async () => {
      const s = mk();
      const late = deferred<DiffDatabase[]>();
      s.api.schemaDiffDatabases.mockReturnValueOnce(late.promise);
      const p = s.ctl.open("sch");
      await flush();
      s.ctl.close("s1");
      late.resolve(DBS);
      await p;
      await flush();
      expect(s.ctl.model("s1")).toBeNull();
      expect(s.api.schemaDiffClose).not.toHaveBeenCalled();
    });
  });

  describe("düzeltme turu 1", () => {
    it("servers() deps.servers()'ı döndürür", () => {
      const s = mk();
      const list = [{ id: "s1", name: "yerel" }] as unknown as ReturnType<DiffControllerDeps["servers"]>;
      s.deps.servers = () => list;
      expect(s.ctl.servers()).toBe(list);
    });

    it("ddlTargetOf yanıtından önce kapanırsa connect ve liste çağrısı yok", async () => {
      const s = mk();
      const late = deferred<DdlTarget>();
      s.api.ddlTargetOf.mockReturnValueOnce(late.promise);
      const p = s.ctl.open("sch");
      s.ctl.close("s1");
      late.resolve(schemaTarget);
      await p;
      await flush();
      expect(s.deps.connect).not.toHaveBeenCalled();
      expect(s.api.schemaDiffDatabases).not.toHaveBeenCalled();
      expect(s.api.ddlLookup).not.toHaveBeenCalled();
    });

    it("connect yanıtından önce kapanırsa liste çağrısı yok", async () => {
      const s = mk();
      const conn = deferred<boolean>();
      (s.deps.connect as ReturnType<typeof vi.fn>).mockReturnValueOnce(conn.promise);
      const p = s.ctl.open("sch");
      await flush();
      expect(s.deps.connect).toHaveBeenCalledTimes(1);
      s.ctl.close("s1");
      conn.resolve(true);
      await p;
      await flush();
      expect(s.api.schemaDiffDatabases).not.toHaveBeenCalled();
      expect(s.api.ddlLookup).not.toHaveBeenCalled();
    });

    it("connect yanıtından önce sunucu değişirse eski sunucunun listesi istenmez", async () => {
      const s = mk();
      await s.ready();
      s.api.schemaDiffDatabases.mockClear();
      const conn = deferred<boolean>();
      (s.deps.connect as ReturnType<typeof vi.fn>).mockReturnValueOnce(conn.promise);
      const p1 = s.ctl.setServer("s1", "source", "s2");
      await flush();
      await s.ctl.setServer("s1", "source", "s3");
      conn.resolve(true);
      await p1;
      await flush();
      expect(s.api.schemaDiffDatabases.mock.calls).toEqual([["s3"]]);
      expect(s.m().source.serverId).toBe("s3");
      expect(s.m().source.loadingLists).toBe(false);
    });

    it("eski ayrıntı hatası (diff_not_found dahil) hiçbir şey yazmaz", async () => {
      const s = mk();
      await s.compared();
      const db = deferred<DiffDetail>();
      s.api.schemaDiffDetail.mockReturnValueOnce(db.promise);
      s.ctl.focus("s1", "b");
      await s.fire();
      s.ctl.focus("s1", "d");
      await s.fire();
      db.reject(err("diff_not_found"));
      await flush();
      expect(s.m().banner).toBeNull();
      expect(s.m().result).not.toBeNull();
      expect(s.m().diffId).toBe("d1");
      expect(s.m().detail).toEqual({ key: "d", data: detailOf("d"), error: null });
    });

    it("diff_not_found kaydetmede → banner gone, result null", async () => {
      const s = mk();
      await s.compared();
      await s.ctl.buildScript("s1");
      s.api.schemaDiffSave.mockRejectedValueOnce(err("diff_not_found"));
      await s.ctl.saveScript("s1");
      expect(s.m().banner).toBe(tsd.gone);
      expect(s.m().result).toBeNull();
      expect(s.m().diffId).toBeNull();
      expect(s.m().script).toBeNull();
      expect(s.deps.notify).not.toHaveBeenCalled();
    });

    it("seçici açıkken diyalog kapanır, betik yenilenir ya da sekme kapanırsa kayıt yapılmaz", async () => {
      const s = mk();
      await s.compared();
      for (const interrupt of [
        () => s.ctl.closeScript("s1"),
        () => void s.ctl.buildScript("s1"),
        () => s.ctl.close("s1"),
      ]) {
        await s.ctl.buildScript("s1");
        const picked = deferred<string | null>();
        (s.picker.save as ReturnType<typeof vi.fn>).mockReturnValueOnce(picked.promise);
        const p = s.ctl.saveScript("s1");
        await flush();
        interrupt();
        await flush();
        picked.resolve(PATH);
        await p;
        await flush();
      }
      expect(s.picker.save).toHaveBeenCalledTimes(3);
      expect(s.api.schemaDiffSave).not.toHaveBeenCalled();
      expect(s.deps.notify).not.toHaveBeenCalled();
    });
  });

  describe("son inceleme düzeltmeleri", () => {
    const connectMock = (s: S) => s.deps.connect as ReturnType<typeof vi.fn>;
    /** Hedef ayrı sunucuda (s2/7/public). */
    const twoServers = async (s: S) => {
      await s.ready();
      await s.ctl.setServer("s1", "target", "s2");
      await s.ctl.setDatabase("s1", "target", 7);
      s.ctl.setSchema("s1", "target", "public");
      connectMock(s).mockClear();
    };

    it("compare start'tan önce iki sunucuya da bağlanır", async () => {
      const s = mk();
      await twoServers(s);
      void s.ctl.compare("s1");
      await flush();
      expect(connectMock(s).mock.calls).toEqual([["s1"], ["s2"]]);
      expect(connectMock(s).mock.invocationCallOrder[1]).toBeLessThan(s.api.schemaDiffStart.mock.invocationCallOrder[0]);
      expect(s.api.schemaDiffStart).toHaveBeenCalledTimes(1);
    });

    it("aynı sunucu iki tarafta → tek bağlanma", async () => {
      const s = mk();
      await s.ready();
      connectMock(s).mockClear();
      void s.ctl.compare("s1");
      await flush();
      expect(connectMock(s).mock.calls).toEqual([["s1"]]);
      expect(s.api.schemaDiffStart).toHaveBeenCalledTimes(1);
    });

    it("connect false (ya da hata) → start yok, connectFailed, phase idle", async () => {
      for (const fail of [() => Promise.resolve(false), () => Promise.reject(new Error("x"))]) {
        cur?.dispose();
        const s = mk();
        await twoServers(s);
        connectMock(s).mockResolvedValueOnce(true).mockImplementationOnce(fail);
        await s.ctl.compare("s1");
        await flush();
        expect(s.api.schemaDiffStart).not.toHaveBeenCalled();
        expect(s.m().error).toBe(tsd.errors.connectFailed);
        expect(s.m().phase).toBe("idle");
        expect(s.m().diffId).toBeNull();
        s.ctl.close("s1");
        expect(s.api.schemaDiffClose).not.toHaveBeenCalled();
        expect(s.api.schemaDiffCancel).not.toHaveBeenCalled();
      }
    });

    it("bağlanırken running: ikinci compare engellenir", async () => {
      const s = mk();
      await s.ready();
      const conn = deferred<boolean>();
      connectMock(s).mockReturnValueOnce(conn.promise);
      void s.ctl.compare("s1");
      expect(s.m().phase).toBe("running");
      await s.ctl.compare("s1");
      conn.resolve(true);
      await flush();
      expect(s.api.schemaDiffStart).toHaveBeenCalledTimes(1);
    });

    it("bağlanırken sekme kapanırsa start, iptal ve kapatma çağrısı yok", async () => {
      const s = mk();
      await s.ready();
      const conn = deferred<boolean>();
      connectMock(s).mockReturnValueOnce(conn.promise);
      const p = s.ctl.compare("s1");
      await flush();
      s.ctl.close("s1");
      conn.resolve(true);
      await p;
      await flush();
      expect(s.api.schemaDiffStart).not.toHaveBeenCalled();
      expect(s.api.schemaDiffCancel).not.toHaveBeenCalled();
      expect(s.api.schemaDiffClose).not.toHaveBeenCalled();
    });

    it("bağlanırken iptal → start yok, İptal edildi. şeridi", async () => {
      const s = mk();
      await s.ready();
      const conn = deferred<boolean>();
      connectMock(s).mockReturnValueOnce(conn.promise);
      const p = s.ctl.compare("s1");
      await flush();
      s.ctl.cancel("s1");
      expect(s.m().cancelRequested).toBe(true);
      conn.resolve(true);
      await p;
      expect(s.api.schemaDiffStart).not.toHaveBeenCalled();
      expect(s.api.schemaDiffCancel).not.toHaveBeenCalled();
      expect(s.m().phase).toBe("idle");
      expect(s.m().banner).toBe(tsd.cancelled);
      expect(s.m().cancelRequested).toBe(false);
    });

    it("start not_connected / connection_lost / unknown_server → eşlenmiş metin, ham ileti yok", async () => {
      for (const code of ["not_connected", "connection_lost", "unknown_server"]) {
        cur?.dispose();
        const s = mk();
        await s.ready();
        void s.ctl.compare("s1");
        await flush();
        s.starts[0].d.reject(err(code, "ham ileti"));
        await flush();
        expect(s.m().error).toBe(tsd.errors[code]);
        expect(s.m().error).not.toContain("ham ileti");
        expect(s.m().phase).toBe("idle");
      }
    });

    it("silinmiş sunucu (servers()'ta yok) → compare çağrı yapmaz", async () => {
      const s = mk();
      await s.ready();
      connectMock(s).mockClear();
      s.deps.servers = () => SERVERS.filter((c) => c.id !== "s1");
      await s.ctl.compare("s1");
      expect(s.deps.connect).not.toHaveBeenCalled();
      expect(s.api.schemaDiffStart).not.toHaveBeenCalled();
      expect(s.m().phase).toBe("idle");
    });

    it("comparedTarget karşılaştırılan hedefi tutar; Query Tool seçici değişse de onu kullanır", async () => {
      const s = mk();
      await s.ready();
      await s.ctl.setDatabase("s1", "target", 8);
      s.ctl.setSchema("s1", "target", "public");
      void s.ctl.compare("s1");
      await flush();
      expect(s.m().comparedTarget).toEqual({ serverId: "s1", databaseOid: 8, schema: "public" });
      s.starts[0].d.resolve(RESULT);
      await flush();
      await s.ctl.buildScript("s1");
      // Diyalog açıkken (normalde kipli) seçici değişirse:
      await s.ctl.setServer("s1", "target", "s2");
      expect(s.m().target.serverId).toBe("s2");
      expect(s.m().comparedTarget).toEqual({ serverId: "s1", databaseOid: 8, schema: "public" });
      await s.ctl.openInQueryTool("s1");
      expect(s.deps.openScript).toHaveBeenCalledWith({ serverId: "s1", databaseOid: 8, database: "arsiv", text: SQL });
      expect(s.m().script).toBeNull();
    });
  });
});
