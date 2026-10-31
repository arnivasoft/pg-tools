import { describe, expect, it } from "vitest";
import { tb } from "../../app/strings-backup";
import type { BackupEvent } from "../../ipc/bindings/BackupEvent";
import type { BackupOptions } from "../../ipc/bindings/BackupOptions";
import type { DumpFormat } from "../../ipc/bindings/DumpFormat";
import type { JobPlan } from "../../ipc/bindings/JobPlan";
import type { JobSummary } from "../../ipc/bindings/JobSummary";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { RestoreOptions } from "../../ipc/bindings/RestoreOptions";
import type { SourceFormat } from "../../ipc/bindings/SourceFormat";
import type { ToolDetection } from "../../ipc/bindings/ToolDetection";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import {
  applyEvent,
  backupGate,
  backupOptionStates,
  backupScopeOf,
  canStart,
  chosenTool,
  confirmFlags,
  defaultBackupOptions,
  defaultRestoreOptions,
  elapsedLabel,
  emptyLog,
  errorText,
  fileExt,
  formatBytes,
  normalizeBackup,
  normalizeRestore,
  progressView,
  restoreGate,
  restoreOptionStates,
  resultView,
  saveFilters,
  serverMajor,
  splitFieldErrors,
  suggestFileName,
  swapExtension,
  toolAdvice,
  type BackupKey,
  type RestoreKey,
} from "./backup-model";

const FORMATS: DumpFormat[] = ["custom", "plain", "directory", "tar"];
const SOURCES: SourceFormat[] = ["custom", "directory", "tar", "plain"];
const B_KEYS: BackupKey[] = ["schemaOnly", "dataOnly", "noOwner", "noPrivileges", "ifExists", "clean", "create", "inserts", "columnInserts", "compress", "jobs", "encoding"];
const R_KEYS: RestoreKey[] = ["clean", "ifExists", "singleTransaction", "jobs", "schemaOnly", "dataOnly", "noOwner", "noPrivileges", "exitOnError"];

const bo = (p: Partial<BackupOptions> = {}): BackupOptions => ({ ...defaultBackupOptions(), ...p });
const ro = (p: Partial<RestoreOptions> = {}): RestoreOptions => ({ ...defaultRestoreOptions(), ...p });

describe("varsayılanlar", () => {
  it("tüm bool false, sayılar ve kodlama null", () => {
    expect(defaultBackupOptions()).toEqual({
      schemaOnly: false, dataOnly: false, noOwner: false, noPrivileges: false, ifExists: false, clean: false, create: false,
      inserts: false, columnInserts: false, compress: null, jobs: null, encoding: null,
    });
    expect(defaultRestoreOptions()).toEqual({
      clean: false, ifExists: false, create: false, singleTransaction: false, jobs: null, schemaOnly: false, dataOnly: false,
      noOwner: false, noPrivileges: false, exitOnError: false,
    });
  });
});

describe("backupOptionStates", () => {
  it("her format x her anahtar için görünürlük tablosu", () => {
    for (const f of FORMATS) {
      const st = backupOptionStates(f, bo());
      expect(Object.keys(st).sort()).toEqual([...B_KEYS].sort());
      for (const k of ["clean", "inserts", "columnInserts", "ifExists"] as const) expect(st[k].visible).toBe(f === "plain");
      expect(st.compress.visible).toBe(f !== "tar");
      expect(st.jobs.visible).toBe(f === "directory");
      for (const k of ["schemaOnly", "dataOnly", "noOwner", "noPrivileges", "create", "encoding"] as const) {
        expect(st[k]).toMatchObject({ visible: true, enabled: true });
      }
    }
  });

  it("create plain dışında açıklama taşır, plain'de taşımaz", () => {
    expect(backupOptionStates("plain", bo()).create.reason).toBeNull();
    for (const f of ["custom", "directory", "tar"] as const) {
      expect(backupOptionStates(f, bo()).create).toEqual({ visible: true, enabled: true, reason: "Arşive yazılır; geri yüklemede kullanılır." });
    }
  });

  it("ifExists clean kapalıyken devre dışı, gerekçe arka uçla aynı", () => {
    expect(backupOptionStates("plain", bo()).ifExists).toEqual({ visible: true, enabled: false, reason: "--if-exists, temizle (--clean) seçeneğini gerektirir." });
    expect(backupOptionStates("plain", bo({ clean: true })).ifExists).toMatchObject({ enabled: true, reason: null });
  });

  it("dışlamalar karşılıklı ve gerekçeler birebir", () => {
    const so = "Yalnız şema ile yalnız veri birlikte seçilemez.";
    expect(backupOptionStates("custom", bo({ schemaOnly: true })).dataOnly).toEqual({ visible: true, enabled: false, reason: so });
    expect(backupOptionStates("custom", bo({ dataOnly: true })).schemaOnly).toEqual({ visible: true, enabled: false, reason: so });
    const ins = "--inserts ile --column-inserts birlikte seçilemez.";
    expect(backupOptionStates("plain", bo({ inserts: true })).columnInserts).toEqual({ visible: true, enabled: false, reason: ins });
    expect(backupOptionStates("plain", bo({ columnInserts: true })).inserts).toEqual({ visible: true, enabled: false, reason: ins });
    const cd = "Temizle (--clean) yalnız veri ile birlikte kullanılamaz.";
    expect(backupOptionStates("plain", bo({ dataOnly: true })).clean).toEqual({ visible: true, enabled: false, reason: cd });
    expect(backupOptionStates("plain", bo({ clean: true })).dataOnly).toEqual({ visible: true, enabled: false, reason: cd });
  });
});

describe("normalizeBackup", () => {
  it("plain -> custom geçişinde plain'e özgüler temizlenir", () => {
    const o = bo({ clean: true, ifExists: true, inserts: true, noOwner: true });
    expect(normalizeBackup("custom", o)).toMatchObject({ clean: false, ifExists: false, inserts: false, columnInserts: false, noOwner: true });
  });
  it("tar'da compress null, directory dışında jobs null", () => {
    expect(normalizeBackup("tar", bo({ compress: 5, jobs: 4 }))).toMatchObject({ compress: null, jobs: null });
    expect(normalizeBackup("custom", bo({ compress: 5, jobs: 4 }))).toMatchObject({ compress: 5, jobs: null });
    expect(normalizeBackup("directory", bo({ compress: 5, jobs: 4 }))).toMatchObject({ compress: 5, jobs: 4 });
  });
  it("çelişen çiftte yalnız biri kalır", () => {
    const n = normalizeBackup("plain", bo({ schemaOnly: true, dataOnly: true }));
    expect(n.schemaOnly !== n.dataOnly).toBe(true);
    const m = normalizeBackup("plain", bo({ clean: true, dataOnly: true }));
    expect(m.clean !== m.dataOnly).toBe(true);
    const k = normalizeBackup("plain", bo({ inserts: true, columnInserts: true }));
    expect(k.inserts !== k.columnInserts).toBe(true);
  });
  it("ifExists, clean yokken temizlenir", () => {
    expect(normalizeBackup("plain", bo({ ifExists: true })).ifExists).toBe(false);
    expect(normalizeBackup("plain", bo({ ifExists: true, clean: true })).ifExists).toBe(true);
  });
  it("create ve encoding korunur", () => {
    expect(normalizeBackup("tar", bo({ create: true, encoding: "UTF8" }))).toMatchObject({ create: true, encoding: "UTF8" });
  });
});

describe("restoreOptionStates", () => {
  it("4 kaynak x 2 hedef x her anahtar: görünürlük", () => {
    for (const src of SOURCES) {
      for (const target of ["existing", "create_from_archive"] as const) {
        const st = restoreOptionStates(src, target, ro());
        expect(Object.keys(st).sort()).toEqual([...R_KEYS].sort());
        for (const k of R_KEYS) {
          const expected = src === "plain" ? k === "singleTransaction" : true;
          expect(st[k].visible, `${src}/${target}/${k}`).toBe(expected);
        }
      }
    }
  });
  it("jobs tar'da devre dışı, singleTransaction ile de", () => {
    expect(restoreOptionStates("tar", "existing", ro()).jobs).toEqual({ visible: true, enabled: false, reason: "Tar arşivinde paralel geri yükleme yapılamaz." });
    expect(restoreOptionStates("custom", "existing", ro()).jobs).toMatchObject({ enabled: true, reason: null });
    expect(restoreOptionStates("custom", "existing", ro({ singleTransaction: true })).jobs).toEqual({
      visible: true, enabled: false, reason: "Paralel iş ile tek transaction birlikte kullanılamaz.",
    });
  });
  it("singleTransaction create_from_archive'da ve jobs doluyken devre dışı", () => {
    expect(restoreOptionStates("custom", "create_from_archive", ro()).singleTransaction).toEqual({
      visible: true, enabled: false, reason: "Oluştur (--create) ile tek transaction birlikte kullanılamaz.",
    });
    expect(restoreOptionStates("custom", "existing", ro({ jobs: 4 })).singleTransaction).toEqual({
      visible: true, enabled: false, reason: "Paralel iş ile tek transaction birlikte kullanılamaz.",
    });
    expect(restoreOptionStates("custom", "existing", ro()).singleTransaction).toMatchObject({ enabled: true });
    expect(restoreOptionStates("plain", "existing", ro()).singleTransaction).toMatchObject({ visible: true, enabled: true });
  });
  it("ifExists clean gerektirir; schemaOnly/dataOnly ve clean/dataOnly dışlaması", () => {
    expect(restoreOptionStates("custom", "existing", ro()).ifExists.reason).toBe("--if-exists, temizle (--clean) seçeneğini gerektirir.");
    expect(restoreOptionStates("custom", "existing", ro({ clean: true })).ifExists.enabled).toBe(true);
    expect(restoreOptionStates("custom", "existing", ro({ schemaOnly: true })).dataOnly.reason).toBe("Yalnız şema ile yalnız veri birlikte seçilemez.");
    expect(restoreOptionStates("custom", "existing", ro({ dataOnly: true })).schemaOnly.enabled).toBe(false);
    expect(restoreOptionStates("custom", "existing", ro({ dataOnly: true })).clean.reason).toBe("Temizle (--clean) yalnız veri ile birlikte kullanılamaz.");
    expect(restoreOptionStates("custom", "existing", ro({ clean: true })).dataOnly.enabled).toBe(false);
  });
});

describe("normalizeRestore", () => {
  it("plain'de singleTransaction dışında hepsi false/null", () => {
    const n = normalizeRestore("plain", "existing", ro({ clean: true, ifExists: true, jobs: 4, schemaOnly: true, noOwner: true, exitOnError: true, singleTransaction: true }));
    expect(n).toEqual({ ...defaultRestoreOptions(), singleTransaction: true });
  });
  it("create hedeften türetilir", () => {
    expect(normalizeRestore("custom", "create_from_archive", ro()).create).toBe(true);
    expect(normalizeRestore("custom", "existing", ro({ create: true })).create).toBe(false);
  });
  it("create_from_archive'da singleTransaction temizlenir", () => {
    expect(normalizeRestore("custom", "create_from_archive", ro({ singleTransaction: true })).singleTransaction).toBe(false);
  });
  it("tar'da jobs null; jobs ve singleTransaction birlikte olmaz", () => {
    expect(normalizeRestore("tar", "existing", ro({ jobs: 4 })).jobs).toBeNull();
    const n = normalizeRestore("custom", "existing", ro({ jobs: 4, singleTransaction: true }));
    expect(n.jobs !== null && n.singleTransaction).toBe(false);
  });
  it("ifExists clean yokken temizlenir", () => {
    expect(normalizeRestore("custom", "existing", ro({ ifExists: true })).ifExists).toBe(false);
    expect(normalizeRestore("custom", "existing", ro({ ifExists: true, clean: true })).ifExists).toBe(true);
  });
});

const flags = { connected: false, partitioned: false, system: false, primaryKey: false, notNull: false, fromExtension: false, locked: false };
const node = (id: string, kind: NodeKind, label: string): TreeNode => ({ id, kind, label, detail: null, hasChildren: true, serverId: "s1", flags, color: null, tag: null });
function tree(spec: [string, NodeKind, string, string | null][]) {
  const nodes = new Map<string, TreeNode>();
  const parent = new Map<string, string>();
  for (const [id, kind, label, p] of spec) {
    nodes.set(id, node(id, kind, label));
    parent.set(id, p ?? "root");
  }
  return { nodes, parent };
}
const T = tree([
  ["srv", "server", "Yerel", null],
  ["dbs", "databaseCollection", "Veritabanları", "srv"],
  ["db", "database", "müşteri", "dbs"],
  ["scs", "schemaCollection", "Şemalar", "db"],
  ["sc", "schema", "satış", "scs"],
  ["scp", "schema", "public", "scs"],
  ["tbls", "tableCollection", "Tablolar", "scp"],
  ["t", "table", 'a"b', "tbls"],
  ["mvs", "materializedViewCollection", "MV", "scp"],
  ["mv", "materializedView", "özet", "mvs"],
  ["sqs", "sequenceCollection", "Sequence", "scp"],
  ["sq", "sequence", "seq_1", "sqs"],
  ["pc", "partitionCollection", "Bölümler", "t"],
  ["pt", "table", "a_p1", "pc"],
  ["col", "column", "id", "t"],
]);

describe("backupScopeOf", () => {
  it("veritabanı", () => {
    expect(backupScopeOf(T, "db")).toEqual({ scope: { kind: "database" }, dbNodeId: "db", dbName: "müşteri", label: "müşteri" });
  });
  it("şema adı değiştirilmeden", () => {
    const r = backupScopeOf(T, "sc");
    expect(r?.scope).toEqual({ kind: "schema", name: "satış" });
    expect(r?.dbNodeId).toBe("db");
    expect(r?.dbName).toBe("müşteri");
  });
  it("tablo, MV ve sequence table kapsamı", () => {
    expect(backupScopeOf(T, "t")?.scope).toEqual({ kind: "table", schema: "public", name: 'a"b' });
    expect(backupScopeOf(T, "mv")?.scope).toEqual({ kind: "table", schema: "public", name: "özet" });
    expect(backupScopeOf(T, "sq")?.scope).toEqual({ kind: "table", schema: "public", name: "seq_1" });
    expect(backupScopeOf(T, "t")?.dbName).toBe("müşteri");
  });
  it("partitionCollection altı, bilinmeyen tür ve bilinmeyen düğüm null", () => {
    expect(backupScopeOf(T, "pt")).toBeNull();
    expect(backupScopeOf(T, "col")).toBeNull();
    expect(backupScopeOf(T, "tbls")).toBeNull();
    expect(backupScopeOf(T, "yok")).toBeNull();
  });
});

describe("dosya adı ve uzantı", () => {
  it("fileExt", () => {
    expect(fileExt("custom", null)).toBe(".backup");
    expect(fileExt("custom", 5)).toBe(".backup");
    expect(fileExt("plain", null)).toBe(".sql");
    expect(fileExt("plain", 0)).toBe(".sql");
    expect(fileExt("plain", 5)).toBe(".sql.gz");
    expect(fileExt("tar", null)).toBe(".tar");
    expect(fileExt("directory", null)).toBe("");
  });
  it("suggestFileName", () => {
    const now = new Date(2026, 9, 31, 14, 5);
    expect(suggestFileName(["satış", "public", 'a"b'], "custom", null, now)).toBe("satış_public_a_b_20261031_1405.backup");
    expect(suggestFileName(["db"], "directory", null, now)).toBe("db_20261031_1405");
    expect(suggestFileName(["db"], "plain", 6, now)).toBe("db_20261031_1405.sql.gz");
    expect(suggestFileName(['a<b>c:d"e/f\\g|h?i*j\u0001k'], "tar", null, now)).toBe("a_b_c_d_e_f_g_h_i_j_k_20261031_1405.tar");
  });
  it("swapExtension", () => {
    const c = { format: "custom", compress: null } as const;
    expect(swapExtension("C:\\a\\x.backup", c, { format: "plain", compress: null })).toBe("C:\\a\\x.sql");
    expect(swapExtension("C:\\a\\x.sql", { format: "plain", compress: 0 }, { format: "plain", compress: 6 })).toBe("C:\\a\\x.sql.gz");
    expect(swapExtension("C:\\a\\x.sql.gz", { format: "plain", compress: 6 }, { format: "plain", compress: 0 })).toBe("C:\\a\\x.sql");
    expect(swapExtension("C:\\a\\x.backup", c, { format: "directory", compress: null })).toBe("C:\\a\\x");
    expect(swapExtension("C:\\a\\x", { format: "directory", compress: null }, { format: "tar", compress: null })).toBe("C:\\a\\x.tar");
    expect(swapExtension("C:\\a\\x.dmp", c, { format: "plain", compress: null })).toBe("C:\\a\\x.dmp");
    expect(swapExtension("C:\\a\\x.BACKUP", c, { format: "plain", compress: null })).toBe("C:\\a\\x.sql");
    expect(swapExtension("C:\\a\\x.backup", c, c)).toBe("C:\\a\\x.backup");
    expect(swapExtension("", c, { format: "plain", compress: null })).toBe("");
  });
  it("saveFilters", () => {
    expect(saveFilters("custom", null)[0].extensions).toContain("backup");
    expect(saveFilters("plain", 0)[0].extensions).toEqual(["sql"]);
    expect(saveFilters("plain", 5)[0].extensions).toEqual(["sql.gz"]);
    expect(saveFilters("tar", null)[0].extensions).toEqual(["tar"]);
    expect(saveFilters("directory", null)).toEqual([]);
  });
});

describe("splitFieldErrors", () => {
  const fields = ["outputPath", "options.clean", "scope.name"];
  it("görünür alanlar byField, kalan general", () => {
    const r = splitFieldErrors(
      [
        { path: "outputPath", message: "Bu yol Windows kod sayfasında güvenle temsil edilemiyor." },
        { path: "request", message: "r" },
        { path: "tool", message: "t" },
        { path: "database", message: "d" },
        { path: "ssl.rootCert", message: "s" },
      ],
      fields,
    );
    expect(r.byField).toEqual({ outputPath: "Bu yol Windows kod sayfasında güvenle temsil edilemiyor." });
    expect(r.general).toEqual(["r", "t", "d", "s"]);
  });
  it("aynı alanda ilk ileti kazanır", () => {
    const r = splitFieldErrors([{ path: "outputPath", message: "ilk" }, { path: "outputPath", message: "ikinci" }], fields);
    expect(r.byField.outputPath).toBe("ilk");
    expect(r.general).toEqual([]);
  });
});

describe("araç ve sunucu sürümü", () => {
  it("serverMajor", () => {
    expect(serverMajor("16.4")).toBe(16);
    expect(serverMajor("9.6.24")).toBe(9);
    expect(serverMajor("18beta1")).toBe(18);
    expect(serverMajor("PostgreSQL 15.2")).toBe(15);
    expect(serverMajor(null)).toBeNull();
    expect(serverMajor("")).toBeNull();
    expect(serverMajor("bilinmiyor")).toBeNull();
  });
  const ver = (major: number) => ({ major, versionNum: major * 10000, text: String(major) });
  const set = (dir: string, major: number) => ({ dir, version: ver(major), pgDump: true, pgRestore: true, psql: true });
  const det: ToolDetection = {
    sets: [set("C:\\pg16\\bin", 16), set("C:\\pg18\\bin", 18)],
    problem: null,
    chosenDump: "C:\\pg16\\bin",
    chosenRestore: "c:/pg18/bin/",
    chosenPsql: "C:\\yok\\bin",
  };
  it("chosenTool seçili klasörün kümesini bulur", () => {
    expect(chosenTool(det, "dump")).toEqual({ dir: "C:\\pg16\\bin", version: ver(16) });
    expect(chosenTool(det, "restore")?.version.major).toBe(18);
    expect(chosenTool(det, "psql")).toBeNull();
    expect(chosenTool({ ...det, chosenDump: null }, "dump")).toBeNull();
  });
  it("toolAdvice", () => {
    expect(toolAdvice(ver(18), 16, "restore")).toEqual({ tone: "warning", text: tb.advice.restoreOld(18, 16) });
    expect(toolAdvice(ver(18), 16, "backup")).toEqual({ tone: "info", text: tb.advice.info(16, 18) });
    expect(toolAdvice(ver(16), 16, "restore")).toBeNull();
    expect(toolAdvice(ver(18), 16, "psql")).toBeNull();
    expect(toolAdvice(ver(18), null, "restore")).toBeNull();
    expect(toolAdvice(null, 16, "restore")).toBeNull();
    expect(toolAdvice(ver(16), 18, "restore")?.tone).toBe("info");
    expect(toolAdvice(ver(17), 16, "restore")?.tone).toBe("warning");
    expect(toolAdvice(ver(17), 17, "restore")).toBeNull();
  });
  it("uyarı metinleri spec ile birebir", () => {
    expect(tb.advice.info(16, 18)).toBe("Sunucuyla aynı ana sürümden (16) araç önerilir; seçili: 18.");
    expect(tb.advice.restoreOld(18, 16)).toBe(
      'Seçili pg_restore 18, sunucu 16: araç sunucudan yeni olduğundan SET transaction_timeout satırı hata verir ("hatada dur" kapalıyken atlanır ve iş "tamamlandı, hatalarla" biter). Sunucuyla aynı ana sürümden (16) PostgreSQL istemci araçlarını kurup Araç ayarlarından seçin.',
    );
  });
});

describe("günlük ve ilerleme", () => {
  const finished = (o: Partial<Extract<BackupEvent, { event: "finished" }>> = {}): BackupEvent => ({
    event: "finished", outcome: "success", exitCode: 0, elapsedMs: 1000, warnings: 0, errors: 0, outputPath: "C:\\x.backup", hint: null, truncatedLines: 0, ...o,
  });
  it("started önizlemeyi koyar", () => {
    expect(applyEvent(emptyLog(), { event: "started", preview: "pg_dump ..." }).preview).toBe("pg_dump ...");
  });
  it("2001 satır olduğu gibi tutulur, girdi değişmez", () => {
    let log = emptyLog();
    const before = log;
    for (let i = 0; i < 2000; i++) log = applyEvent(log, { event: "line", severity: "info", text: `s${i}` });
    log = applyEvent(log, { event: "line", severity: "warning", text: "… 5 satır daha gösterilmedi" });
    expect(log.lines).toHaveLength(2001);
    expect(log.lines[2000]).toEqual({ severity: "warning", text: "… 5 satır daha gösterilmedi" });
    expect(before.lines).toHaveLength(0);
  });
  it("progress son değeri tutar", () => {
    let log = applyEvent(emptyLog(), { event: "progress", done: 1, total: 5, bytes: null });
    log = applyEvent(log, { event: "progress", done: 3, total: 5, bytes: 2048 });
    expect(log.progress).toEqual({ done: 3, total: 5, bytes: 2048 });
  });
  it("finished summary'yi koyar ve sonrası yok sayılır", () => {
    let log = applyEvent(emptyLog(), finished({ outcome: "failed", exitCode: 1, hint: "ipucu" }));
    expect(log.summary).toEqual({ outcome: "failed", exitCode: 1, elapsedMs: 1000, warnings: 0, errors: 0, outputPath: "C:\\x.backup", hint: "ipucu", truncatedLines: 0 });
    const frozen = log;
    log = applyEvent(log, { event: "line", severity: "error", text: "geç" });
    log = applyEvent(log, { event: "progress", done: 9, total: null, bytes: null });
    log = applyEvent(log, finished({ outcome: "success" }));
    expect(log).toEqual(frozen);
  });

  it("progressView: done total ile sınırlanır", () => {
    expect(progressView({ done: 7, total: 5, bytes: null }, "backup")).toEqual({ percent: 100, text: "5 / 5 tablo" });
    expect(progressView({ done: 1, total: 3, bytes: null }, "backup").percent).toBe(33);
  });
  it("progressView: belirsiz, bayt, restore birimi, boş", () => {
    expect(progressView({ done: 7, total: null, bytes: 1536 }, "backup")).toEqual({ percent: null, text: "7 tablo · 1,5 KB" });
    expect(progressView({ done: 7, total: null, bytes: null }, "restore")).toEqual({ percent: null, text: "7 öğe" });
    expect(progressView({ done: 2, total: 4, bytes: 1536 }, "backup").text).toBe("2 / 4 tablo · 1,5 KB");
    expect(progressView({ done: 2, total: 0, bytes: null }, "restore")).toEqual({ percent: null, text: "2 öğe" });
    expect(progressView(null, "backup")).toEqual({ percent: null, text: tb.progress.starting });
  });
  it("formatBytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1536)).toBe("1,5 KB");
    expect(formatBytes(5 * 1024 ** 2)).toBe("5,0 MB");
    expect(formatBytes(3 * 1024 ** 3)).toBe("3,0 GB");
    expect(formatBytes(1024 ** 2 - 1)).toBe("1,0 MB");
    expect(formatBytes(-5)).toBe("0 B");
  });
  it("elapsedLabel", () => {
    expect(elapsedLabel(0)).toBe("00:00");
    expect(elapsedLabel(65000)).toBe("01:05");
    expect(elapsedLabel(3_700_000)).toBe("1:01:40");
    expect(elapsedLabel(59_999)).toBe("00:59");
  });
});

describe("resultView", () => {
  const s = (p: Partial<JobSummary> = {}): JobSummary => ({ outcome: "success", exitCode: 0, elapsedMs: 65000, warnings: 0, errors: 0, outputPath: null, hint: null, truncatedLines: 0, ...p });
  it("success", () => {
    expect(resultView(s(), "backup", false)).toEqual({ tone: "success", title: "Tamamlandı (01:05)", lines: [] });
    expect(resultView(s({ warnings: 2 }), "backup", false).lines).toEqual(["2 uyarı"]);
  });
  it("completed_with_errors", () => {
    const r = resultView(s({ outcome: "completed_with_errors", errors: 3, hint: "ipucu 4" }), "restore", false);
    expect(r.tone).toBe("warning");
    expect(r.title).toBe("Tamamlandı, 3 hata — veri yüklenmiş olabilir");
    expect(r.lines).toContain("ipucu 4");
    expect(r.lines).toContain("3 hata");
  });
  it("failed hint ile", () => {
    const r = resultView(s({ outcome: "failed", exitCode: 1, hint: "Parola reddedildi.", errors: 1 }), "backup", false);
    expect(r.tone).toBe("error");
    expect(r.title).toBe("Başarısız");
    expect(r.lines[0]).toBe("Parola reddedildi.");
  });
  it("cancelled; yarım kalma satırı yalnız restore ve tek transaction kapalıyken", () => {
    const c = s({ outcome: "cancelled", exitCode: null });
    const half = "Veritabanı yarım kalmış olabilir (tek transaction kapalıydı).";
    expect(resultView(c, "restore", false)).toMatchObject({ tone: "neutral", title: "İptal edildi" });
    expect(resultView(c, "restore", false).lines).toContain(half);
    expect(resultView(c, "restore", true).lines).not.toContain(half);
    expect(resultView(c, "backup", false).lines).not.toContain(half);
  });
  it("truncatedLines", () => {
    expect(resultView(s({ truncatedLines: 17 }), "backup", false).lines).toContain("17 satır gösterilmedi");
    expect(resultView(s(), "backup", false).lines.join()).not.toContain("gösterilmedi");
  });
});

describe("kapılar", () => {
  const plan = (p: Partial<JobPlan> = {}): JobPlan => ({ preview: "pg_dump", risk: "safe", notes: [], errors: [], ...p });
  const bIn = { running: false, hasPath: true, pending: false, planCurrent: true, plan: plan() };
  it("backupGate sırası", () => {
    expect(backupGate({ ...bIn, running: true, hasPath: false, pending: true, planCurrent: false })).toEqual({ kind: "blocked", reason: "running" });
    expect(backupGate({ ...bIn, hasPath: false, pending: true, planCurrent: false })).toEqual({ kind: "blocked", reason: "noPath" });
    expect(backupGate({ ...bIn, pending: true, planCurrent: false })).toEqual({ kind: "blocked", reason: "pending" });
    expect(backupGate({ ...bIn, planCurrent: false })).toEqual({ kind: "blocked", reason: "stale" });
    expect(backupGate({ ...bIn, plan: plan({ errors: [{ path: "outputPath", message: "x" }] }) })).toEqual({ kind: "blocked", reason: "invalid" });
    expect(backupGate({ ...bIn, plan: null })).toEqual({ kind: "blocked", reason: "invalid" });
    expect(backupGate(bIn)).toEqual({ kind: "ready" });
  });
  const rIn = {
    locked: false, running: false, inspecting: false, hasSource: true, hasTarget: true, pending: false, planCurrent: true,
    plan: plan({ risk: "destructive" }), production: false, serverName: "Üretim-1",
  };
  it("restoreGate sırası", () => {
    const all = { ...rIn, locked: true, running: true, inspecting: true, hasSource: false, hasTarget: false, pending: true, planCurrent: false };
    const order: [string, Partial<typeof rIn>][] = [
      ["locked", {}],
      ["running", { locked: false }],
      ["inspecting", { locked: false, running: false }],
      ["noSource", { locked: false, running: false, inspecting: false }],
      ["noTarget", { locked: false, running: false, inspecting: false, hasSource: true }],
      ["pending", { locked: false, running: false, inspecting: false, hasSource: true, hasTarget: true }],
      ["stale", { locked: false, running: false, inspecting: false, hasSource: true, hasTarget: true, pending: false }],
    ];
    for (const [reason, patch] of order) expect(restoreGate({ ...all, ...patch })).toEqual({ kind: "blocked", reason });
    expect(restoreGate({ ...rIn, plan: plan({ errors: [{ path: "target", message: "x" }] }) })).toEqual({ kind: "blocked", reason: "invalid" });
    expect(restoreGate({ ...rIn, plan: null })).toEqual({ kind: "blocked", reason: "invalid" });
  });
  it("restoreGate risk", () => {
    expect(restoreGate(rIn)).toEqual({ kind: "needsConfirm" });
    expect(restoreGate({ ...rIn, plan: plan({ risk: "safe" }) })).toEqual({ kind: "ready" });
    expect(restoreGate({ ...rIn, plan: plan({ risk: "data_loss" }) })).toEqual({ kind: "needsConfirm" });
    expect(restoreGate({ ...rIn, plan: plan({ risk: "data_loss" }), production: true })).toEqual({ kind: "needsProductionName", expected: "Üretim-1" });
    expect(restoreGate({ ...rIn, production: true })).toEqual({ kind: "needsConfirm" });
  });
  it("blocked anahtarları tb.blocked içinde", () => {
    for (const r of ["running", "noPath", "pending", "stale", "invalid", "locked", "inspecting", "noSource", "noTarget"]) expect(tb.blocked).toHaveProperty(r);
  });
  it("canStart / confirmFlags", () => {
    const blocked = { kind: "blocked", reason: "running" } as const;
    expect(canStart({ kind: "ready" }, { checked: false, typedName: "" })).toBe(true);
    expect(canStart(blocked, { checked: true, typedName: "x" })).toBe(false);
    expect(canStart({ kind: "needsConfirm" }, { checked: false, typedName: "" })).toBe(false);
    expect(canStart({ kind: "needsConfirm" }, { checked: true, typedName: "" })).toBe(true);

    const g = { kind: "needsProductionName", expected: "Üretim-1" } as const;
    expect(canStart(g, { checked: true, typedName: "Üretim-1" })).toBe(true);
    expect(canStart(g, { checked: true, typedName: "  Üretim-1 " })).toBe(true);
    expect(canStart(g, { checked: true, typedName: "üretim-1" })).toBe(false);
    expect(canStart(g, { checked: true, typedName: "Üretim" })).toBe(false);
    expect(canStart(g, { checked: false, typedName: "Üretim-1" })).toBe(false);
    const empty = { kind: "needsProductionName", expected: "" } as const;
    expect(canStart(empty, { checked: true, typedName: "" })).toBe(false);
    expect(canStart(empty, { checked: true, typedName: "  " })).toBe(false);

    expect(confirmFlags({ kind: "ready" }, { checked: true, typedName: "" })).toEqual({ confirmed: false, confirmedProduction: false });
    expect(confirmFlags({ kind: "needsConfirm" }, { checked: true, typedName: "" })).toEqual({ confirmed: true, confirmedProduction: false });
    expect(confirmFlags({ kind: "needsConfirm" }, { checked: false, typedName: "" })).toEqual({ confirmed: false, confirmedProduction: false });
    expect(confirmFlags(g, { checked: true, typedName: "Üretim-1" })).toEqual({ confirmed: true, confirmedProduction: true });
    expect(confirmFlags(g, { checked: true, typedName: "yanlış" })).toEqual({ confirmed: true, confirmedProduction: false });
    expect(confirmFlags(empty, { checked: true, typedName: "" })).toEqual({ confirmed: true, confirmedProduction: false });
    expect(confirmFlags(blocked, { checked: true, typedName: "x" })).toEqual({ confirmed: false, confirmedProduction: false });
  });
});

describe("errorText", () => {
  it("password_required yönlendirme ekler", () => {
    expect(errorText({ code: "password_required", message: "Parola gerekli.", detail: null, hint: null })).toBe(`Parola gerekli. ${tb.passwordAdvice}`);
    expect(tb.passwordAdvice).toBe("Sunucu bağlantısını kesip yeniden bağlanın; parola istenir.");
  });
  it("diğer kodlarda message ve varsa hint", () => {
    expect(errorText({ code: "busy", message: "Meşgul.", detail: "gizli", hint: null })).toBe("Meşgul.");
    expect(errorText({ code: "x", message: "Hata.", detail: null, hint: "Şunu yapın." })).toBe("Hata. Şunu yapın.");
  });
});
