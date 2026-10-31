import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tb } from "../../app/strings-backup";
import type { BackupEvent } from "../../ipc/bindings/BackupEvent";
import type { BackupRequest } from "../../ipc/bindings/BackupRequest";
import type { DdlTarget } from "../../ipc/bindings/DdlTarget";
import type { JobOutcome } from "../../ipc/bindings/JobOutcome";
import type { JobPlan } from "../../ipc/bindings/JobPlan";
import type { JobSummary } from "../../ipc/bindings/JobSummary";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { RestoreRequest } from "../../ipc/bindings/RestoreRequest";
import type { RestoreSource } from "../../ipc/bindings/RestoreSource";
import type { Risk } from "../../ipc/bindings/Risk";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { ToolDetection } from "../../ipc/bindings/ToolDetection";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { resultView } from "./backup-model";
import {
  createBackupController,
  createRestoreController,
  type BackupControllerDeps,
  type Picker,
} from "./backup-controller";

// ---- Ortak sahte veriler ------------------------------------------------------------------------------------------

const flags = (locked: boolean) => ({ connected: true, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked });
const node = (id: string, kind: NodeKind, label: string, p: Partial<TreeNode> = {}): TreeNode =>
  ({ id, kind, label, detail: null, hasChildren: true, serverId: "S1", flags: flags(false), color: null, tag: null, ...p });

function makeTree(locked: boolean) {
  const nodes = new Map<string, TreeNode>();
  for (const n of [
    node("srv", "server", "Sunucu", { detail: "16.4", flags: flags(locked) }),
    node("dbc", "databaseCollection", "Veritabanları"),
    node("db1", "database", "satış"),
    node("db2", "database", "diğer"),
    node("sch", "schema", "public"),
    node("tbl", "table", "a\"b"),
  ]) nodes.set(n.id, n);
  const parent = new Map<string, string>([["dbc", "srv"], ["db1", "dbc"], ["db2", "dbc"], ["sch", "db1"], ["tbl", "sch"]]);
  return { nodes, parent };
}

const ddlTarget = (oid: number): DdlTarget =>
  ({ serverId: "S1", databaseOid: oid, object: { kind: "database", oid }, create: null, focus: null, refreshNodeId: null });
const plan = (preview = "pg_dump PGPASSWORD=***", risk: Risk = "safe", p: Partial<JobPlan> = {}): JobPlan => ({ preview, risk, notes: [], errors: [], ...p });
const detection = (): ToolDetection => ({ sets: [], problem: null, chosenDump: null, chosenRestore: null, chosenPsql: null });
const summary = (outcome: JobOutcome, p: Partial<JobSummary> = {}): JobSummary =>
  ({ outcome, exitCode: 0, elapsedMs: 1500, warnings: 0, errors: 0, outputPath: null, hint: null, truncatedLines: 0, ...p });
const finished = (outcome: JobOutcome, p: Partial<JobSummary> = {}): BackupEvent => ({ event: "finished", ...summary(outcome, p) });
const src = (p: Partial<RestoreSource> = {}): RestoreSource =>
  ({ path: "C:\\Yedek\\a.backup", format: "custom", archive: { archiveVersion: "16.4", serverVersion: "16.4", dbName: "orijinal", items: 10, counts: [] }, isDir: false, ...p });
const err = (code: string, message = "hata", p: { hint?: string | null } = {}) => ({ code, message, detail: null, hint: p.hint ?? null });
const server = (p: Partial<ServerConfig> = {}): ServerConfig => ({
  id: "S1", name: "Ana", group: null, host: "h", port: 5432, maintenanceDb: "postgres", username: "u", savePassword: false,
  ssl: {} as ServerConfig["ssl"], color: null, label: null, readOnlyLock: false, ...p,
});

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function setup(opts: { locked?: boolean; servers?: ServerConfig[] } = {}) {
  let locked = opts.locked ?? false;
  const servers = opts.servers ?? [server()];
  let ids = 0;
  const jobs: { onEvent: (e: BackupEvent) => void; result: ReturnType<typeof deferred<JobSummary>> }[] = [];
  const launch = (onEvent: (e: BackupEvent) => void) => {
    const result = deferred<JobSummary>();
    jobs.push({ onEvent, result });
    return result.promise;
  };
  const api = {
    ddlTargetOf: vi.fn(async (n: string): Promise<DdlTarget> => ddlTarget(n === "db2" ? 8 : 5)),
    ddlLookup: vi.fn(async () => [{ value: "UTF8", detail: null }, { value: "LATIN5", detail: null }]),
    listChildren: vi.fn(async (id: string): Promise<TreeNode[]> =>
      id === "srv" ? [node("dbc", "databaseCollection", "Veritabanları")] : [node("db1", "database", "satış"), node("db2", "database", "diğer")]),
    backupTools: vi.fn(async (_s: string) => detection()),
    backupPlan: vi.fn(async (_s: string, _o: number, _r: BackupRequest) => plan()),
    backupStart: vi.fn((_s: string, _o: number, _r: BackupRequest, _j: string, onEvent: (e: BackupEvent) => void) => launch(onEvent)),
    backupCancel: vi.fn(async (_j: string) => null),
    backupReveal: vi.fn(async (_p: string) => null),
    restoreInspect: vi.fn(async (_p: string) => src()),
    restorePlan: vi.fn(async (_s: string, _o: number | null, _r: RestoreRequest) => plan("pg_restore PGPASSWORD=***", "destructive")),
    restoreStart: vi.fn((_s: string, _o: number | null, _r: RestoreRequest, _j: string, _c: boolean, _p: boolean, onEvent: (e: BackupEvent) => void) => launch(onEvent)),
    settingsGet: vi.fn(async () => ({ pgBinDir: "C:\\pg\\bin" }) as never),
    settingsSetPgBinDir: vi.fn(async (dir: string | null) => ({ pgBinDir: dir }) as never),
  };
  const pick = {
    save: vi.fn<Picker["save"]>(async () => "C:\\Yedek\\satis.backup"),
    openFile: vi.fn<Picker["openFile"]>(async () => "C:\\Yedek\\a.backup"),
    openDir: vi.fn<Picker["openDir"]>(async () => "C:\\pg\\yeni"),
  };
  const connect = vi.fn(async (_s: string) => true);
  const notify = vi.fn();
  const onClosed = vi.fn();
  const refreshNode = vi.fn();
  const refreshInspector = vi.fn();
  const deps: BackupControllerDeps = {
    api: api as unknown as BackupControllerDeps["api"],
    tree: () => makeTree(locked),
    servers: () => servers,
    connect,
    refreshNode,
    refreshInspector,
    notify,
    pick,
    onClosed,
    newId: () => `J${++ids}`,
    now: () => new Date(2026, 9, 31, 14, 5).getTime(),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  };
  return {
    deps, api, pick, connect, notify, onClosed, refreshNode, refreshInspector, jobs,
    setLocked: (v: boolean) => { locked = v; },
  };
}
type S = ReturnType<typeof setup>;

let dispose: (() => void) | null = null;
function inRoot<T>(fn: () => T): T {
  return createRoot((d) => {
    dispose = d;
    return fn();
  });
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  dispose?.();
  dispose = null;
  vi.useRealTimers();
});

const flush = () => vi.advanceTimersByTimeAsync(0);
const settle = () => vi.advanceTimersByTimeAsync(200);

async function backupReady(s: S, nodeId = "db1") {
  const ctl = inRoot(() => createBackupController(s.deps));
  ctl.open(nodeId);
  await settle();
  return ctl;
}
async function restoreReady(s: S, nodeId = "db1") {
  const ctl = inRoot(() => createRestoreController(s.deps));
  ctl.open(nodeId);
  await settle();
  return ctl;
}
const lastBackupReq = (s: S) => s.api.backupPlan.mock.calls.at(-1)![2];

// ---- Yedekleme ----------------------------------------------------------------------------------------------------

describe("yedek denetleyicisi: açılış", () => {
  it("kapsam, oid, araçlar, ayar ve kodlamalarla hazır olur; ilk plan istenir", async () => {
    const s = setup();
    const ctl = await backupReady(s, "tbl");
    const st = ctl.state()!;
    expect(st.phase).toBe("ready");
    expect(s.connect).toHaveBeenCalledWith("S1");
    expect(s.api.ddlTargetOf).toHaveBeenCalledWith("db1");
    expect(st.databaseOid).toBe(5);
    expect(st.scope).toEqual({ kind: "table", schema: "public", name: "a\"b" });
    expect(st.scopeLabel).toBe("public.a\"b");
    expect(st.dbName).toBe("satış");
    expect(st.pgBinDir).toBe("C:\\pg\\bin");
    expect(st.encodings).toEqual(["UTF8", "LATIN5"]);
    expect(st.serverMajor).toBe(16);
    expect(st.serverName).toBe("Ana");
    expect(s.api.backupPlan).toHaveBeenCalledTimes(1);
    expect(s.api.backupPlan.mock.calls[0]![0]).toBe("S1");
    expect(s.api.backupPlan.mock.calls[0]![1]).toBe(5);
  });

  it("connect false → loadError; yeniden dene başarılı akışa döner", async () => {
    const s = setup();
    s.connect.mockResolvedValueOnce(false);
    const ctl = inRoot(() => createBackupController(s.deps));
    ctl.open("db1");
    await settle();
    expect(ctl.state()!.phase).toBe("loadError");
    expect(ctl.state()!.loadError).toBe(tb.loadFailed);
    expect(s.api.ddlTargetOf).not.toHaveBeenCalled();
    ctl.open(ctl.state()!.nodeId);
    await settle();
    expect(ctl.state()!.phase).toBe("ready");
    expect(ctl.state()!.loadError).toBeNull();
  });

  it("backupTools hatası toolsError'a yazılır, diyalog açık kalır", async () => {
    const s = setup();
    s.api.backupTools.mockRejectedValueOnce(err("tool_not_found", "Araç bulunamadı."));
    const ctl = await backupReady(s);
    expect(ctl.state()!.phase).toBe("ready");
    expect(ctl.state()!.toolsError).toBe("Araç bulunamadı.");
    expect(ctl.state()!.tools).toBeNull();
  });

  it("kodlama aramasının hatası yalnız listeyi boş bırakır", async () => {
    const s = setup();
    s.api.ddlLookup.mockRejectedValueOnce(err("query"));
    const ctl = await backupReady(s);
    expect(ctl.state()!.phase).toBe("ready");
    expect(ctl.state()!.encodings).toEqual([]);
  });

  it("bölüm altı düğüm desteklenmez → loadError", async () => {
    const s = setup();
    const ctl = inRoot(() => createBackupController(s.deps));
    ctl.open("yok");
    await settle();
    expect(ctl.state()!.phase).toBe("loadError");
  });
});

describe("yedek denetleyicisi: plan", () => {
  it("150 ms debounce: art arda üç değişiklik tek backupPlan çağrısı yapar", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.api.backupPlan.mockClear();
    ctl.setOption("noOwner", true);
    await vi.advanceTimersByTimeAsync(100);
    ctl.setOption("noPrivileges", true);
    await vi.advanceTimersByTimeAsync(100);
    ctl.setOption("schemaOnly", true);
    expect(s.api.backupPlan).not.toHaveBeenCalled();
    expect(ctl.state()!.pending).toBe(true);
    await vi.advanceTimersByTimeAsync(149);
    expect(s.api.backupPlan).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2);
    expect(s.api.backupPlan).toHaveBeenCalledTimes(1);
    expect(ctl.state()!.pending).toBe(false);
  });

  it("istek {scope, format, options, outputPath}; options normalize edilmiştir", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    await ctl.chooseOutput();
    ctl.setOption("inserts", true); // custom'da görünmez → varsayılana döner
    ctl.setOption("noOwner", true);
    await settle();
    const req = lastBackupReq(s);
    expect(Object.keys(req).sort()).toEqual(["format", "options", "outputPath", "scope"]);
    expect(req.scope).toEqual({ kind: "database" });
    expect(req.format).toBe("custom");
    expect(req.outputPath).toBe("C:\\Yedek\\satis.backup");
    expect(req.options.inserts).toBe(false);
    expect(req.options.noOwner).toBe(true);
  });

  it("eski sürümün yanıtı yok sayılır", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    const a = deferred<JobPlan>();
    const b = deferred<JobPlan>();
    s.api.backupPlan.mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    ctl.setOption("noOwner", true);
    await settle(); // A uçuşta
    ctl.setOption("noPrivileges", true);
    await settle(); // B uçuşta
    b.resolve(plan("YENİ"));
    await flush();
    a.resolve(plan("ESKİ"));
    await flush();
    expect(ctl.state()!.plan!.preview).toBe("YENİ");
    expect(ctl.state()!.planFor).toBe(ctl.state()!.version);
  });

  it("sürüm değişmişken gelen yanıt (A, B'den önce biter) planı güncellemez", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    const a = deferred<JobPlan>();
    s.api.backupPlan.mockReturnValueOnce(a.promise);
    ctl.setOption("noOwner", true);
    await settle();
    ctl.setOption("noPrivileges", true); // sürüm ilerler, A bayatlar
    a.resolve(plan("ESKİ"));
    await flush();
    expect(ctl.state()!.plan!.preview).not.toBe("ESKİ");
    expect(ctl.state()!.pending).toBe(true);
  });

  it("not_connected → bir kez connect + yeniden plan", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.api.backupPlan.mockClear();
    s.connect.mockClear();
    s.api.backupPlan.mockRejectedValueOnce(err("not_connected", "Bağlı değil"));
    ctl.setOption("noOwner", true);
    await settle();
    expect(s.connect).toHaveBeenCalledTimes(1);
    expect(s.api.backupPlan).toHaveBeenCalledTimes(2);
    expect(ctl.state()!.planError).toBeNull();
    expect(ctl.state()!.plan).not.toBeNull();
  });

  it("ikinci not_connected → planError, daha fazla bağlanma denenmez", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.connect.mockClear();
    s.api.backupPlan.mockRejectedValue(err("not_connected", "Bağlı değil"));
    ctl.setOption("noOwner", true);
    await settle();
    expect(s.connect).toHaveBeenCalledTimes(1);
    expect(ctl.state()!.planError).toBe("Bağlı değil");
    expect(ctl.state()!.pending).toBe(false);
    expect(ctl.gate().kind).toBe("blocked");
  });

  it("bağlanma başarısızsa planError", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.connect.mockResolvedValueOnce(false);
    s.api.backupPlan.mockRejectedValueOnce(err("not_connected", "Bağlı değil"));
    ctl.setOption("noOwner", true);
    await settle();
    expect(ctl.state()!.planError).toBe("Bağlı değil");
  });
});

describe("yedek denetleyicisi: format ve yol", () => {
  it("chooseOutput pick.save'e suggestFileName + saveFilters verir; yol yazılır", async () => {
    const s = setup();
    const ctl = await backupReady(s, "tbl");
    await ctl.chooseOutput();
    const arg = s.pick.save.mock.calls[0]![0];
    expect(arg.defaultPath).toBe("satış_public_a_b_20261031_1405.backup");
    expect(arg.filters).toEqual([{ name: tb.filters.custom, extensions: ["backup", "dump"] }]);
    expect(ctl.state()!.outputPath).toBe("C:\\Yedek\\satis.backup");
  });

  it("vazgeç (null) yolu değiştirmez ve plan istemez", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    await ctl.chooseOutput();
    await settle();
    s.api.backupPlan.mockClear();
    const v = ctl.state()!.version;
    s.pick.save.mockResolvedValueOnce(null);
    await ctl.chooseOutput();
    await settle();
    expect(ctl.state()!.outputPath).toBe("C:\\Yedek\\satis.backup");
    expect(ctl.state()!.version).toBe(v);
    expect(s.api.backupPlan).not.toHaveBeenCalled();
  });

  it("yol seçildikten sonra format değişince swapExtension uygulanır ve plan yenilenir", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    await ctl.chooseOutput();
    s.api.backupPlan.mockClear();
    ctl.setFormat("tar");
    expect(ctl.state()!.outputPath).toBe("C:\\Yedek\\satis.tar");
    await settle();
    expect(s.api.backupPlan).toHaveBeenCalledTimes(1);
    expect(lastBackupReq(s).format).toBe("tar");
    expect(lastBackupReq(s).outputPath).toBe("C:\\Yedek\\satis.tar");
    ctl.setFormat("directory");
    expect(ctl.state()!.outputPath).toBe("C:\\Yedek\\satis");
  });

  it("kullanıcının kendi uzantısı korunur", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.pick.save.mockResolvedValueOnce("C:\\Yedek\\x.dump");
    await ctl.chooseOutput();
    ctl.setFormat("tar");
    expect(ctl.state()!.outputPath).toBe("C:\\Yedek\\x.dump");
  });

  it("plain sıkıştırmasını değiştirmek .sql ↔ .sql.gz uzantısını çevirir", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    ctl.setFormat("plain");
    s.pick.save.mockResolvedValueOnce("C:\\Yedek\\p.sql");
    await ctl.chooseOutput();
    ctl.setOption("compress", 5);
    expect(ctl.state()!.outputPath).toBe("C:\\Yedek\\p.sql.gz");
  });

  it("format değişince artık görünmeyen seçenekler sıfırlanır", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    ctl.setFormat("plain");
    ctl.setOption("clean", true);
    ctl.setFormat("custom");
    expect(ctl.state()!.options.clean).toBe(false);
  });
});

describe("yedek denetleyicisi: başlatma ve olaylar", () => {
  async function started(s: S) {
    const ctl = await backupReady(s);
    await ctl.chooseOutput();
    await settle();
    const p = ctl.start();
    await flush();
    return { ctl, p };
  }

  it("yol yokken ya da plan hazır değilken başlatılmaz", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    expect(ctl.gate()).toEqual({ kind: "blocked", reason: "noPath" });
    await ctl.start();
    expect(s.api.backupStart).not.toHaveBeenCalled();
    await ctl.chooseOutput();
    expect(ctl.gate()).toEqual({ kind: "blocked", reason: "pending" });
    await ctl.start();
    expect(s.api.backupStart).not.toHaveBeenCalled();
  });

  it("plan hatası varsa başlatılmaz", async () => {
    const s = setup();
    s.api.backupPlan.mockResolvedValue(plan("", "safe", { errors: [{ path: "outputPath", message: "Yol boş olamaz." }] }));
    const ctl = await backupReady(s);
    await ctl.chooseOutput();
    await settle();
    expect(ctl.gate()).toEqual({ kind: "blocked", reason: "invalid" });
    await ctl.start();
    expect(s.api.backupStart).not.toHaveBeenCalled();
  });

  it("started → line×3 → progress → finished durumu doğru kurar; success sonrası yol temizlenir", async () => {
    const s = setup();
    const { ctl, p } = await started(s);
    const [sid, oid, req, jobId] = s.api.backupStart.mock.calls[0]!;
    expect([sid, oid, jobId]).toEqual(["S1", 5, "J1"]);
    expect(req.outputPath).toBe("C:\\Yedek\\satis.backup");
    const st0 = ctl.state()!;
    expect(st0.phase).toBe("running");
    expect(st0.jobId).toBe("J1");
    expect(st0.startedAt).not.toBeNull();
    const { onEvent, result } = s.jobs[0]!;
    onEvent({ event: "started", preview: "pg_dump PGPASSWORD=***" });
    onEvent({ event: "line", severity: "info", text: "bir" });
    onEvent({ event: "line", severity: "warning", text: "iki" });
    onEvent({ event: "line", severity: "error", text: "üç" });
    onEvent({ event: "progress", done: 3, total: 10, bytes: 2048 });
    const mid = ctl.state()!;
    expect(mid.log.preview).toBe("pg_dump PGPASSWORD=***");
    expect(mid.log.lines.map((l) => l.text)).toEqual(["bir", "iki", "üç"]);
    expect(mid.log.progress).toEqual({ done: 3, total: 10, bytes: 2048 });
    onEvent(finished("success", { outputPath: "C:\\Yedek\\satis.backup" }));
    const end = ctl.state()!;
    expect(end.phase).toBe("ready");
    expect(end.log.summary?.outcome).toBe("success");
    expect(end.outputPath).toBe("");
    result.resolve(summary("success", { outputPath: "C:\\Yedek\\satis.backup" }));
    await p;
    expect(ctl.state()!.phase).toBe("ready");
    expect(s.refreshNode).not.toHaveBeenCalled();
  });

  it("başarılı yedekten sonra yol temizlenir, sürüm artar ve plan yenilenir", async () => {
    const s = setup();
    const { ctl, p } = await started(s);
    const v = ctl.state()!.version;
    s.api.backupPlan.mockClear();
    s.jobs[0]!.onEvent(finished("success"));
    expect(ctl.state()!.outputPath).toBe("");
    expect(ctl.state()!.version).toBe(v + 1);
    expect(ctl.state()!.pending).toBe(true);
    await settle();
    expect(s.api.backupPlan).toHaveBeenCalledTimes(1);
    expect(lastBackupReq(s).outputPath).toBe("");
    s.jobs[0]!.result.resolve(summary("success"));
    await p;
  });

  it("sonuç start dönüşünden gelirse de kurulur (finished olayı gelmeden)", async () => {
    const s = setup();
    const { ctl, p } = await started(s);
    s.jobs[0]!.result.resolve(summary("success"));
    await p;
    expect(ctl.state()!.phase).toBe("ready");
    expect(ctl.state()!.log.summary?.outcome).toBe("success");
    expect(ctl.state()!.outputPath).toBe("");
  });

  it("busy hatası startError'a yazılır, phase ready", async () => {
    const s = setup();
    const { ctl, p } = await started(s);
    s.jobs[0]!.result.reject(err("busy", "Başka bir iş sürüyor."));
    await p;
    expect(ctl.state()!.phase).toBe("ready");
    expect(ctl.state()!.startError).toBe("Başka bir iş sürüyor.");
    expect(ctl.state()!.outputPath).not.toBe("");
  });

  it("password_required start hatası yönlendirme metnini içerir", async () => {
    const s = setup();
    const { ctl, p } = await started(s);
    s.jobs[0]!.result.reject(err("password_required", "Parola gerekli."));
    await p;
    expect(ctl.state()!.startError).toContain("Parola gerekli.");
    expect(ctl.state()!.startError).toContain(tb.passwordAdvice);
  });

  it("sonuçtan sonra yeniden başlatılabilir (yeni jobId)", async () => {
    const s = setup();
    const { ctl, p } = await started(s);
    s.jobs[0]!.onEvent(finished("failed", { exitCode: 1, errors: 1 }));
    s.jobs[0]!.result.resolve(summary("failed"));
    await p;
    expect(ctl.state()!.outputPath).not.toBe(""); // başarısızlıkta yol korunur
    ctl.setOption("noOwner", true);
    await settle();
    const p2 = ctl.start();
    await flush();
    expect(s.api.backupStart.mock.calls[1]![3]).toBe("J2");
    expect(ctl.state()!.log.lines).toEqual([]);
    expect(ctl.state()!.log.summary).toBeNull();
    s.jobs[1]!.result.resolve(summary("success"));
    await p2;
  });
});

describe("yedek denetleyicisi: iptal ve yarışlar", () => {
  async function running(s: S) {
    const ctl = await backupReady(s);
    await ctl.chooseOutput();
    await settle();
    const p = ctl.start();
    await flush();
    return { ctl, p, job: s.jobs[0]! };
  }

  it("cancel → backupCancel(jobId); sonuç cancelled gelince 'İptal edildi' görünür", async () => {
    const s = setup();
    const { ctl, p, job } = await running(s);
    ctl.cancel();
    expect(s.api.backupCancel).toHaveBeenCalledWith("J1");
    expect(ctl.state()!.cancelRequested).toBe(true);
    expect(ctl.state()!.phase).toBe("running"); // sonuç arka uçtan beklenir
    job.onEvent(finished("cancelled", { exitCode: null }));
    job.result.resolve(summary("cancelled", { exitCode: null }));
    await p;
    const st = ctl.state()!;
    expect(st.phase).toBe("ready");
    expect(resultView(st.log.summary!, "backup", false).title).toBe(tb.result.cancelled);
    expect(st.outputPath).not.toBe("");
  });

  it("backupCancel hatası sessizce yutulur", async () => {
    const s = setup();
    const { ctl } = await running(s);
    s.api.backupCancel.mockRejectedValueOnce(err("unknown"));
    ctl.cancel();
    await flush();
    expect(ctl.state()!.phase).toBe("running");
    expect(s.notify).not.toHaveBeenCalled();
  });

  it("finished: success iptalden önce gelirse sonuç success kalır ve iptal etkisizdir", async () => {
    const s = setup();
    const { ctl, p, job } = await running(s);
    job.onEvent(finished("success"));
    ctl.cancel();
    expect(s.api.backupCancel).not.toHaveBeenCalled();
    job.result.resolve(summary("success"));
    await p;
    expect(ctl.state()!.log.summary?.outcome).toBe("success");
    expect(ctl.state()!.cancelRequested).toBe(false);
  });

  it("iptal istendikten sonra success gelirse sonuç success'tir", async () => {
    const s = setup();
    const { ctl, p, job } = await running(s);
    ctl.cancel();
    job.onEvent(finished("success"));
    job.result.resolve(summary("success"));
    await p;
    expect(ctl.state()!.log.summary?.outcome).toBe("success");
  });

  it("close() sonrası gelen olay ve dönüş durumu yazmaz", async () => {
    const s = setup();
    const { ctl, p, job } = await running(s);
    ctl.close();
    expect(ctl.state()).toBeNull();
    expect(s.api.backupCancel).toHaveBeenCalledWith("J1");
    job.onEvent({ event: "line", severity: "info", text: "geç" });
    job.onEvent(finished("success"));
    job.result.resolve(summary("success"));
    await p;
    expect(ctl.state()).toBeNull();
    expect(s.onClosed).not.toHaveBeenCalled();
  });

  it("close() sonrası yeniden açılan diyalog eski işin olaylarından etkilenmez", async () => {
    const s = setup();
    const { ctl, p, job } = await running(s);
    ctl.close();
    ctl.open("db1");
    await settle();
    job.onEvent({ event: "line", severity: "info", text: "eski" });
    job.result.reject(err("busy", "eski hata"));
    await p;
    expect(ctl.state()!.log.lines).toEqual([]);
    expect(ctl.state()!.startError).toBeNull();
    expect(ctl.state()!.phase).toBe("ready");
  });

  it("başlatılan iş sürerken yeniden open() yok sayılır", async () => {
    const s = setup();
    const { ctl, job } = await running(s);
    ctl.open("db2");
    await settle();
    expect(ctl.state()!.phase).toBe("running");
    expect(ctl.state()!.nodeId).toBe("db1");
    job.result.resolve(summary("success"));
  });
});

describe("yedek denetleyicisi: kapatma", () => {
  async function running(s: S) {
    const ctl = await backupReady(s);
    await ctl.chooseOutput();
    await settle();
    const p = ctl.start();
    await flush();
    return { ctl, p, job: s.jobs[0]! };
  }

  it("çalışmıyorken requestClose hemen kapatır", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    ctl.requestClose();
    expect(ctl.state()).toBeNull();
    expect(s.onClosed).toHaveBeenCalledTimes(1);
  });

  it("çalışırken requestClose → closeAsk", async () => {
    const s = setup();
    const { ctl, job } = await running(s);
    ctl.requestClose();
    expect(ctl.state()!.closeAsk).toBe(true);
    expect(s.onClosed).not.toHaveBeenCalled();
    job.result.resolve(summary("success"));
  });

  it("answerClose(true) → iptal + sonuç gelince onClosed bir kez", async () => {
    const s = setup();
    const { ctl, p, job } = await running(s);
    ctl.requestClose();
    ctl.answerClose(true);
    expect(s.api.backupCancel).toHaveBeenCalledTimes(1);
    expect(s.api.backupCancel).toHaveBeenCalledWith("J1");
    expect(ctl.state()!.closing).toBe(true);
    expect(ctl.state()!.closeAsk).toBe(false);
    expect(s.onClosed).not.toHaveBeenCalled();
    job.onEvent(finished("cancelled", { exitCode: null }));
    job.result.resolve(summary("cancelled", { exitCode: null }));
    await p;
    expect(ctl.state()).toBeNull();
    expect(s.onClosed).toHaveBeenCalledTimes(1);
    expect(s.api.backupCancel).toHaveBeenCalledTimes(1);
  });

  it("answerClose(true) sonrası start hata dönerse de kapanır", async () => {
    const s = setup();
    const { ctl, p, job } = await running(s);
    ctl.requestClose();
    ctl.answerClose(true);
    job.result.reject(err("cancelled", "iptal"));
    await p;
    expect(ctl.state()).toBeNull();
    expect(s.onClosed).toHaveBeenCalledTimes(1);
  });

  it("answerClose(false) → açık kalır", async () => {
    const s = setup();
    const { ctl, job } = await running(s);
    ctl.requestClose();
    ctl.answerClose(false);
    expect(ctl.state()!.closeAsk).toBe(false);
    expect(ctl.state()!.closing).toBe(false);
    expect(ctl.state()!.phase).toBe("running");
    expect(s.api.backupCancel).not.toHaveBeenCalled();
    expect(s.onClosed).not.toHaveBeenCalled();
    job.result.resolve(summary("success"));
  });

  it("onay sorulurken iş biterse soru kalkar ve diyalog sonucu gösterir", async () => {
    const s = setup();
    const { ctl, p, job } = await running(s);
    ctl.requestClose();
    job.onEvent(finished("success"));
    job.result.resolve(summary("success"));
    await p;
    expect(ctl.state()!.phase).toBe("ready");
    expect(ctl.state()!.closeAsk).toBe(false); // iş bitince soru kalkar
    ctl.answerClose(true); // soru yokken yok sayılır
    expect(ctl.state()).not.toBeNull();
    expect(s.api.backupCancel).not.toHaveBeenCalled();
    expect(s.onClosed).not.toHaveBeenCalled();
  });
});

describe("yedek denetleyicisi: araç ayarları ve klasörü göster", () => {
  it("chooseBinDir → settingsSetPgBinDir(dir) → backupTools → plan", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.api.backupTools.mockClear();
    s.api.backupPlan.mockClear();
    await ctl.chooseBinDir();
    expect(s.api.settingsSetPgBinDir).toHaveBeenCalledWith("C:\\pg\\yeni");
    expect(ctl.state()!.pgBinDir).toBe("C:\\pg\\yeni");
    expect(s.api.backupTools).toHaveBeenCalledTimes(1);
    await settle();
    expect(s.api.backupPlan).toHaveBeenCalledTimes(1);
  });

  it("vazgeçilirse hiçbir şey yapılmaz", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.pick.openDir.mockResolvedValueOnce(null);
    await ctl.chooseBinDir();
    expect(s.api.settingsSetPgBinDir).not.toHaveBeenCalled();
  });

  it("invalid hatası binDirError'a yazılır; yeniden tarama yapılmaz", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.api.backupTools.mockClear();
    s.api.settingsSetPgBinDir.mockRejectedValueOnce(err("invalid", "Bu klasörde araç yok."));
    await ctl.chooseBinDir();
    expect(ctl.state()!.binDirError).toBe("Bu klasörde araç yok.");
    expect(ctl.state()!.pgBinDir).toBe("C:\\pg\\bin");
    expect(s.api.backupTools).not.toHaveBeenCalled();
  });

  it("clearBinDir null gönderir ve hatayı temizler", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    await ctl.clearBinDir();
    expect(s.api.settingsSetPgBinDir).toHaveBeenCalledWith(null);
    expect(ctl.state()!.pgBinDir).toBeNull();
    expect(ctl.state()!.binDirError).toBeNull();
  });

  it("rescanTools backupTools'u çağırır ve planı yeniler; hata toolsError olur", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    s.api.backupPlan.mockClear();
    s.api.backupTools.mockRejectedValueOnce(err("tool_not_found", "yok"));
    await ctl.rescanTools();
    expect(ctl.state()!.toolsError).toBe("yok");
    await settle();
    expect(s.api.backupPlan).toHaveBeenCalledTimes(1);
  });

  it("reveal başarılı yedekten sonra outputPath ile çağrılır; hata sabit metinle bildirilir", async () => {
    const s = setup();
    const ctl = await backupReady(s);
    await ctl.chooseOutput();
    await settle();
    const p = ctl.start();
    await flush();
    s.jobs[0]!.onEvent(finished("success", { outputPath: "C:\\Yedek\\satis.backup" }));
    s.jobs[0]!.result.resolve(summary("success", { outputPath: "C:\\Yedek\\satis.backup" }));
    await p;
    await ctl.reveal();
    expect(s.api.backupReveal).toHaveBeenCalledWith("C:\\Yedek\\satis.backup");
    s.api.backupReveal.mockRejectedValueOnce(err("invalid", "C:\\Yedek\\satis.backup yok"));
    await ctl.reveal();
    expect(s.notify).toHaveBeenCalledWith(tb.revealFailed, "error");
    expect(JSON.stringify(s.notify.mock.calls)).not.toContain("Yedek");
  });
});

// ---- Geri yükleme -------------------------------------------------------------------------------------------------

async function withSource(s: S, nodeId = "db1") {
  const ctl = await restoreReady(s, nodeId);
  await ctl.chooseSource("file");
  await settle();
  return ctl;
}

describe("geri yükleme denetleyicisi: açılış ve kaynak", () => {
  it("veritabanı düğümünde oid alınır, hedef existing; kaynak yokken plan istenmez", async () => {
    const s = setup();
    const ctl = await restoreReady(s);
    const st = ctl.state()!;
    expect(st.phase).toBe("ready");
    expect(st.databaseOid).toBe(5);
    expect(st.target).toBe("existing");
    expect(st.fromServer).toBe(false);
    expect(st.dbNodeId).toBe("db1");
    expect(s.api.restorePlan).not.toHaveBeenCalled();
    expect(ctl.gate()).toEqual({ kind: "blocked", reason: "noSource" });
  });

  it("sunucu düğümünde databases null ve hedef create_from_archive", async () => {
    const s = setup();
    const ctl = await restoreReady(s, "srv");
    const st = ctl.state()!;
    expect(st.fromServer).toBe(true);
    expect(st.databases).toBeNull();
    expect(st.target).toBe("create_from_archive");
    expect(st.databaseOid).toBeNull();
    expect(s.api.ddlTargetOf).not.toHaveBeenCalled();
    expect(st.serverMajor).toBe(16);
  });

  it("connect false → loadError", async () => {
    const s = setup();
    s.connect.mockResolvedValueOnce(false);
    const ctl = await restoreReady(s);
    expect(ctl.state()!.phase).toBe("loadError");
  });

  it("chooseSource('file') → restoreInspect; kaynak yazılır ve plan istenir", async () => {
    const s = setup();
    const ctl = await withSource(s);
    expect(s.api.restoreInspect).toHaveBeenCalledWith("C:\\Yedek\\a.backup");
    expect(ctl.state()!.source?.format).toBe("custom");
    expect(ctl.state()!.inspecting).toBe(false);
    expect(s.api.restorePlan).toHaveBeenCalledTimes(1);
    const [sid, oid, req] = s.api.restorePlan.mock.calls[0]!;
    expect([sid, oid]).toEqual(["S1", 5]);
    expect(req).toEqual({ path: "C:\\Yedek\\a.backup", target: "existing", options: expect.objectContaining({ create: false }) });
  });

  it("chooseSource('dir') openDir kullanır; vazgeçilirse değişmez", async () => {
    const s = setup();
    const ctl = await restoreReady(s);
    s.pick.openDir.mockResolvedValueOnce(null);
    await ctl.chooseSource("dir");
    expect(s.api.restoreInspect).not.toHaveBeenCalled();
    await ctl.chooseSource("dir");
    expect(s.api.restoreInspect).toHaveBeenCalledWith("C:\\pg\\yeni");
  });

  it("inceleme sürerken kapı 'inspecting'", async () => {
    const s = setup();
    const d = deferred<RestoreSource>();
    s.api.restoreInspect.mockReturnValueOnce(d.promise);
    const ctl = await restoreReady(s);
    void ctl.chooseSource("file");
    await flush();
    expect(ctl.state()!.inspecting).toBe(true);
    expect(ctl.gate()).toEqual({ kind: "blocked", reason: "inspecting" });
    d.resolve(src());
    await settle();
    expect(ctl.state()!.inspecting).toBe(false);
  });

  it("invalid_source → sourceError (yol iletide yok), plan yok", async () => {
    const s = setup();
    s.api.restoreInspect.mockRejectedValueOnce(err("invalid_source", "Geçerli bir yedek değil."));
    const ctl = await restoreReady(s);
    await ctl.chooseSource("file");
    await settle();
    const st = ctl.state()!;
    expect(st.sourceError).toBe("Geçerli bir yedek değil.");
    expect(st.sourceError).not.toContain("Yedek");
    expect(st.source).toBeNull();
    expect(s.api.restorePlan).not.toHaveBeenCalled();
    expect(JSON.stringify(s.notify.mock.calls)).toBe("[]");
  });

  it("eski incelemenin yanıtı yeni seçimi ezmez", async () => {
    const s = setup();
    const a = deferred<RestoreSource>();
    s.api.restoreInspect.mockReturnValueOnce(a.promise);
    const ctl = await restoreReady(s);
    void ctl.chooseSource("file");
    await flush();
    s.pick.openFile.mockResolvedValueOnce("C:\\Yedek\\b.sql");
    s.api.restoreInspect.mockResolvedValueOnce(src({ path: "C:\\Yedek\\b.sql", format: "plain", archive: null }));
    // inspecting sırasında ikinci seçim: editable (phase ready) olduğundan geçer
    await ctl.chooseSource("file");
    a.resolve(src({ path: "ESKİ" }));
    await settle();
    expect(ctl.state()!.source?.path).toBe("C:\\Yedek\\b.sql");
  });

  it("plain kaynakta hedef existing'e zorlanır, seçenekler normalize edilir", async () => {
    const s = setup();
    s.api.restoreInspect.mockResolvedValueOnce(src({ format: "plain", archive: null, path: "C:\\Yedek\\a.sql" }));
    const ctl = await restoreReady(s, "srv");
    ctl.setOption("clean", true);
    expect(ctl.state()!.options.clean).toBe(true);
    await ctl.chooseSource("file");
    await settle();
    const st = ctl.state()!;
    expect(st.target).toBe("existing");
    expect(st.options.clean).toBe(false); // plain'de gizli
    expect(st.options.create).toBe(false);
    ctl.setTarget("create_from_archive");
    expect(ctl.state()!.target).toBe("existing");
  });
});

describe("geri yükleme denetleyicisi: hedef ve plan", () => {
  it("sunucu düğümünde setTargetDb: ddlTargetOf oid'i istekte kullanılır", async () => {
    const s = setup();
    const ctl = await withSource(s, "srv");
    s.api.restorePlan.mockClear();
    ctl.setTarget("existing");
    await settle();
    expect(s.api.restorePlan).toHaveBeenCalledTimes(0); // db seçilmedi → plan yok
    expect(ctl.gate()).toEqual({ kind: "blocked", reason: "noTarget" });
    await ctl.setTargetDb("db2");
    await settle();
    expect(s.api.ddlTargetOf).toHaveBeenCalledWith("db2");
    expect(ctl.state()!.databaseOid).toBe(8);
    expect(ctl.state()!.dbNodeId).toBe("db2");
    const [, oid, req] = s.api.restorePlan.mock.calls.at(-1)!;
    expect(oid).toBe(8);
    expect(req.target).toBe("existing");
    expect(req.options.create).toBe(false);
  });

  it("oluşturma hedefinde databaseOid null gider ve create türetilir", async () => {
    const s = setup();
    const ctl = await withSource(s, "db1");
    ctl.setTarget("create_from_archive");
    await settle();
    const [, oid, req] = s.api.restorePlan.mock.calls.at(-1)!;
    expect(oid).toBeNull();
    expect(req.target).toBe("create_from_archive");
    expect(req.options.create).toBe(true);
  });

  it("loadDatabases sunucu → koleksiyon → veritabanları zincirini bir kez yükler", async () => {
    const s = setup();
    const ctl = await restoreReady(s, "srv");
    await ctl.loadDatabases();
    expect(s.api.listChildren.mock.calls.map((c) => c[0])).toEqual(["srv", "dbc"]);
    expect(ctl.state()!.databases).toEqual([{ nodeId: "db1", label: "satış" }, { nodeId: "db2", label: "diğer" }]);
    await ctl.loadDatabases();
    expect(s.api.listChildren).toHaveBeenCalledTimes(2);
  });

  it("loadDatabases hatasında liste boş olur", async () => {
    const s = setup();
    s.api.listChildren.mockRejectedValueOnce(err("not_connected"));
    const ctl = await restoreReady(s, "srv");
    await ctl.loadDatabases();
    expect(ctl.state()!.databases).toEqual([]);
  });

  it("plan değişince (clean açılınca) onay ve yazılan ad sıfırlanır", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true, typedName: "Ana" });
    expect(ctl.state()!.confirm).toEqual({ checked: true, typedName: "Ana" });
    const v = ctl.state()!.version;
    ctl.setOption("clean", true);
    expect(ctl.state()!.version).toBe(v + 1);
    expect(ctl.state()!.confirm).toEqual({ checked: false, typedName: "" });
    await settle();
    expect(ctl.state()!.confirm).toEqual({ checked: false, typedName: "" });
  });

  it("hedef değişince de onay sıfırlanır; setConfirm sürümü artırmaz", async () => {
    const s = setup();
    const ctl = await withSource(s);
    const v = ctl.state()!.version;
    ctl.setConfirm({ checked: true });
    expect(ctl.state()!.version).toBe(v);
    ctl.setTarget("create_from_archive");
    expect(ctl.state()!.confirm.checked).toBe(false);
  });

  it("restoreStart/plan: eski sürümün yanıtı yok sayılır", async () => {
    const s = setup();
    const ctl = await withSource(s);
    const a = deferred<JobPlan>();
    s.api.restorePlan.mockReturnValueOnce(a.promise);
    ctl.setOption("noOwner", true);
    await settle();
    ctl.setOption("noPrivileges", true);
    a.resolve(plan("ESKİ", "safe"));
    await settle();
    expect(ctl.state()!.plan!.preview).toBe("pg_restore PGPASSWORD=***");
  });

  it("not_connected → bir kez connect + yeniden plan", async () => {
    const s = setup();
    const ctl = await withSource(s);
    s.connect.mockClear();
    s.api.restorePlan.mockClear();
    s.api.restorePlan.mockRejectedValueOnce(err("not_connected", "Bağlı değil"));
    ctl.setOption("noOwner", true);
    await settle();
    expect(s.connect).toHaveBeenCalledTimes(1);
    expect(s.api.restorePlan).toHaveBeenCalledTimes(2);
    expect(ctl.state()!.planError).toBeNull();
  });
});

describe("geri yükleme denetleyicisi: onay, başlatma, kilit", () => {
  const prod = () => setup({ servers: [server({ name: "Prod-1", label: "production" })] });

  it("destructive: onay kutusu işaretlenmeden restoreStart çağrılmaz; sonra confirmed gider", async () => {
    const s = setup();
    const ctl = await withSource(s);
    expect(ctl.gate()).toEqual({ kind: "needsConfirm" });
    await ctl.start();
    expect(s.api.restoreStart).not.toHaveBeenCalled();
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    const a = s.api.restoreStart.mock.calls[0]!;
    expect(a[0]).toBe("S1");
    expect(a[1]).toBe(5);
    expect(a[3]).toBe("J1");
    expect(a[4]).toBe(true);
    expect(a[5]).toBe(false);
    s.jobs[0]!.result.resolve(summary("success"));
    await p;
  });

  it("üretim + data_loss: ad yazılmadan çağrı yok; doğru adla confirmed ve confirmedProduction gider", async () => {
    const s = prod();
    s.api.restorePlan.mockResolvedValue(plan("pg_restore PGPASSWORD=***", "data_loss"));
    const ctl = await withSource(s);
    expect(ctl.gate()).toEqual({ kind: "needsProductionName", expected: "Prod-1" });
    ctl.setConfirm({ checked: true });
    await ctl.start();
    expect(s.api.restoreStart).not.toHaveBeenCalled();
    ctl.setConfirm({ typedName: "prod-1" }); // büyük/küçük harf duyarlı
    await ctl.start();
    expect(s.api.restoreStart).not.toHaveBeenCalled();
    ctl.setConfirm({ typedName: "  Prod-1 " });
    const p = ctl.start();
    await flush();
    const a = s.api.restoreStart.mock.calls[0]!;
    expect(a[4]).toBe(true);
    expect(a[5]).toBe(true);
    s.jobs[0]!.result.resolve(summary("success"));
    await p;
  });

  it("üretim + data_loss: işaretli ad plan değişince sıfırlanır ve yeniden gerekir", async () => {
    const s = prod();
    s.api.restorePlan.mockResolvedValue(plan("pg_restore PGPASSWORD=***", "data_loss"));
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true, typedName: "Prod-1" });
    ctl.setOption("clean", true);
    await settle();
    await ctl.start();
    expect(s.api.restoreStart).not.toHaveBeenCalled();
    expect(ctl.state()!.confirm).toEqual({ checked: false, typedName: "" });
  });

  it("kilit ağaçta açılınca gate().reason === 'locked' ve start çağrı yapmaz", async () => {
    const s = setup({ locked: true });
    const ctl = await withSource(s);
    expect(ctl.state()!.locked).toBe(true);
    expect(ctl.gate()).toEqual({ kind: "blocked", reason: "locked" });
    ctl.setConfirm({ checked: true });
    await ctl.start();
    expect(s.api.restoreStart).not.toHaveBeenCalled();
  });

  it("kilit canlı okunur: sonradan kilitlenirse start engellenir", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    s.setLocked(true);
    expect(ctl.gate()).toEqual({ kind: "blocked", reason: "locked" });
    await ctl.start();
    expect(s.api.restoreStart).not.toHaveBeenCalled();
  });

  it("success → refreshNode(veritabanı düğümü) + refreshInspector; onay sıfırlanır", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    s.jobs[0]!.onEvent(finished("success"));
    s.jobs[0]!.result.resolve(summary("success"));
    await p;
    expect(s.refreshNode).toHaveBeenCalledTimes(1);
    expect(s.refreshNode).toHaveBeenCalledWith("db1");
    expect(s.refreshInspector).toHaveBeenCalledTimes(1);
    expect(ctl.state()!.confirm.checked).toBe(false);
  });

  it("oluşturma hedefinde success → refreshNode(sunucu düğümü)", async () => {
    const s = setup();
    const ctl = await withSource(s, "srv");
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    expect(s.api.restoreStart.mock.calls[0]![1]).toBeNull();
    s.jobs[0]!.result.resolve(summary("success"));
    await p;
    expect(s.refreshNode).toHaveBeenCalledWith("srv");
  });

  it("completed_with_errors → 'Tamamlandı, N hata' ve yenileme", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    s.jobs[0]!.onEvent(finished("completed_with_errors", { errors: 3, exitCode: 1 }));
    s.jobs[0]!.result.resolve(summary("completed_with_errors", { errors: 3, exitCode: 1 }));
    await p;
    const st = ctl.state()!;
    expect(resultView(st.log.summary!, "restore", false).title).toBe(tb.result.completedWithErrors(3));
    expect(resultView(st.log.summary!, "restore", false).title).toContain("Tamamlandı, 3 hata");
    expect(s.refreshNode).toHaveBeenCalledTimes(1);
    expect(s.refreshInspector).toHaveBeenCalledTimes(1);
  });

  it("failed/cancelled sonuçta Explorer yenilenmez", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    s.jobs[0]!.result.resolve(summary("failed", { exitCode: 1 }));
    await p;
    expect(s.refreshNode).not.toHaveBeenCalled();
    expect(s.refreshInspector).not.toHaveBeenCalled();
  });

  it("diyalog kapanmışken biten işte yine de Explorer yenilenir ama durum yazılmaz", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    ctl.close();
    s.jobs[0]!.onEvent(finished("success"));
    s.jobs[0]!.result.resolve(summary("success"));
    await p;
    expect(ctl.state()).toBeNull();
    expect(s.refreshNode).toHaveBeenCalledTimes(1);
    expect(s.onClosed).not.toHaveBeenCalled();
  });

  it("finished olayı ve start dönüşü birlikte gelince yenileme bir kez yapılır", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    s.jobs[0]!.onEvent(finished("success"));
    s.jobs[0]!.result.resolve(summary("success"));
    await p;
    expect(s.refreshNode).toHaveBeenCalledTimes(1);
  });

  it("iptal ve kapatma onayı geri yüklemede de çalışır", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    ctl.cancel();
    expect(s.api.backupCancel).toHaveBeenCalledWith("J1");
    ctl.requestClose();
    expect(ctl.state()!.closeAsk).toBe(true);
    ctl.answerClose(true);
    s.jobs[0]!.onEvent(finished("cancelled", { exitCode: null }));
    s.jobs[0]!.result.resolve(summary("cancelled", { exitCode: null }));
    await p;
    expect(ctl.state()).toBeNull();
    expect(s.onClosed).toHaveBeenCalledTimes(1);
  });

  it("password_required start hatası passwordAdvice içerir", async () => {
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    s.jobs[0]!.result.reject(err("password_required", "Parola gerekli."));
    await p;
    expect(ctl.state()!.startError).toContain(tb.passwordAdvice);
    expect(ctl.state()!.phase).toBe("ready");
  });
});

describe("geri yükleme denetleyicisi: bayat onay ve yarışlar", () => {
  it("plan beklerken verilen onay yok sayılır; yeni plan data_loss ise onay yeniden istenir", async () => {
    const s = setup();
    const ctl = await withSource(s);
    s.api.restorePlan.mockResolvedValue(plan("pg_restore PGPASSWORD=***", "data_loss"));
    ctl.setOption("clean", true);
    expect(ctl.state()!.pending).toBe(true);
    ctl.setConfirm({ checked: true, typedName: "Ana" });
    expect(ctl.state()!.confirm).toEqual({ checked: false, typedName: "" });
    await settle();
    expect(ctl.state()!.plan!.risk).toBe("data_loss");
    expect(ctl.gate()).toEqual({ kind: "needsConfirm" });
    await ctl.start();
    expect(s.api.restoreStart).not.toHaveBeenCalled();
  });

  it("uçuştaki plan sürerken (pending) onay kabul edilmez", async () => {
    const s = setup();
    const ctl = await withSource(s);
    const a = deferred<JobPlan>();
    s.api.restorePlan.mockReturnValueOnce(a.promise);
    ctl.setOption("noOwner", true);
    await settle();
    ctl.setConfirm({ checked: true });
    expect(ctl.state()!.confirm.checked).toBe(false);
    a.resolve(plan("pg_restore PGPASSWORD=***", "destructive"));
    await flush();
    expect(ctl.state()!.confirm.checked).toBe(false);
    ctl.setConfirm({ checked: true });
    expect(ctl.state()!.confirm.checked).toBe(true);
  });

  it("setTargetDb eşzamanlı çağrılarda son seçim kazanır", async () => {
    const s = setup();
    const ctl = await withSource(s, "srv");
    const a = deferred<DdlTarget>();
    s.api.ddlTargetOf.mockReturnValueOnce(a.promise);
    const p1 = ctl.setTargetDb("db1");
    await ctl.setTargetDb("db2");
    a.resolve(ddlTarget(5));
    await p1;
    expect(ctl.state()!.databaseOid).toBe(8);
    expect(ctl.state()!.dbNodeId).toBe("db2");
  });
});

describe("geri yükleme denetleyicisi: araç ayarları", () => {
  it("chooseBinDir → settingsSetPgBinDir → backupTools → plan", async () => {
    const s = setup();
    const ctl = await withSource(s);
    s.api.backupTools.mockClear();
    s.api.restorePlan.mockClear();
    await ctl.chooseBinDir();
    expect(s.api.settingsSetPgBinDir).toHaveBeenCalledWith("C:\\pg\\yeni");
    expect(s.api.backupTools).toHaveBeenCalledTimes(1);
    await settle();
    expect(s.api.restorePlan).toHaveBeenCalledTimes(1);
  });

  it("invalid hatası binDirError'a yazılır; clearBinDir null gönderir", async () => {
    const s = setup();
    const ctl = await restoreReady(s);
    s.api.settingsSetPgBinDir.mockRejectedValueOnce(err("invalid", "Geçersiz klasör."));
    await ctl.chooseBinDir();
    expect(ctl.state()!.binDirError).toBe("Geçersiz klasör.");
    await ctl.clearBinDir();
    expect(s.api.settingsSetPgBinDir).toHaveBeenLastCalledWith(null);
    expect(ctl.state()!.binDirError).toBeNull();
  });
});

describe("gizlilik", () => {
  it("bildirimlere ve konsola yol/ad/parola yazılmaz", async () => {
    const log = vi.spyOn(console, "log");
    const warn = vi.spyOn(console, "warn");
    const error = vi.spyOn(console, "error");
    const s = setup();
    const ctl = await withSource(s);
    ctl.setConfirm({ checked: true });
    const p = ctl.start();
    await flush();
    s.jobs[0]!.onEvent({ event: "line", severity: "info", text: "C:\\Yedek\\a.backup" });
    s.jobs[0]!.result.reject(err("failed", "x"));
    await p;
    expect(s.notify).not.toHaveBeenCalled();
    expect(log).not.toHaveBeenCalled();
    expect(warn).not.toHaveBeenCalled();
    expect(error).not.toHaveBeenCalled();
    vi.restoreAllMocks();
  });
});
