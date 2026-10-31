// Yedekleme / geri yükleme diyaloglarının saf modeli (Plan 27). Ana paket modülü içe aktarmaz; yalnız tür ve tembel metin modülü kullanır.
// Kaynak: spec 2026-10-18-backup-restore-design.md §1 (seçenek kuralları) ve §5 (alan iletileri, olaylar, kapılar).
// Arka uç tek doğruluk kaynağıdır: buradaki kurallar yalnız görünürlük ve devre dışılık içindir; plan `errors` her zaman gösterilir.
import { tb } from "../../app/strings-backup";
import type { BackupEvent } from "../../ipc/bindings/BackupEvent";
import type { BackupOptions } from "../../ipc/bindings/BackupOptions";
import type { BackupScope } from "../../ipc/bindings/BackupScope";
import type { DumpFormat } from "../../ipc/bindings/DumpFormat";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { JobPlan } from "../../ipc/bindings/JobPlan";
import type { JobSummary } from "../../ipc/bindings/JobSummary";
import type { LineSeverity } from "../../ipc/bindings/LineSeverity";
import type { RestoreOptions } from "../../ipc/bindings/RestoreOptions";
import type { RestoreTarget } from "../../ipc/bindings/RestoreTarget";
import type { SourceFormat } from "../../ipc/bindings/SourceFormat";
import type { ToolDetection } from "../../ipc/bindings/ToolDetection";
import type { ToolVersion } from "../../ipc/bindings/ToolVersion";
import type { TreeNode } from "../../ipc/bindings/TreeNode";

export type OptionState = { visible: boolean; enabled: boolean; reason: string | null };
export type BackupKey = keyof BackupOptions;
export type RestoreKey = Exclude<keyof RestoreOptions, "create">;

const SHOWN: OptionState = { visible: true, enabled: true, reason: null };
const HIDDEN: OptionState = { visible: false, enabled: false, reason: null };
const off = (reason: string): OptionState => ({ visible: true, enabled: false, reason });

export function defaultBackupOptions(): BackupOptions {
  return {
    schemaOnly: false, dataOnly: false, noOwner: false, noPrivileges: false, ifExists: false, clean: false, create: false,
    inserts: false, columnInserts: false, compress: null, jobs: null, encoding: null,
  };
}

export function defaultRestoreOptions(): RestoreOptions {
  return {
    clean: false, ifExists: false, create: false, singleTransaction: false, jobs: null, schemaOnly: false, dataOnly: false,
    noOwner: false, noPrivileges: false, exitOnError: false,
  };
}

// ---- Yedek seçenekleri -------------------------------------------------------------------------------------------

export function backupOptionStates(format: DumpFormat, o: BackupOptions): Record<BackupKey, OptionState> {
  const plain = format === "plain";
  const r = tb.reason;
  return {
    schemaOnly: o.dataOnly ? off(r.schemaDataExclusive) : SHOWN,
    dataOnly: o.schemaOnly ? off(r.schemaDataExclusive) : plain && o.clean ? off(r.cleanDataOnly) : SHOWN,
    noOwner: SHOWN,
    noPrivileges: SHOWN,
    clean: !plain ? HIDDEN : o.dataOnly ? off(r.cleanDataOnly) : SHOWN,
    ifExists: !plain ? HIDDEN : o.clean ? SHOWN : off(r.ifExistsNeedsClean),
    // `create` her formatta görünür; plain dışında yalnız açıklama taşır (devre dışı değildir).
    create: plain ? SHOWN : { visible: true, enabled: true, reason: r.createInArchive },
    inserts: !plain ? HIDDEN : o.columnInserts ? off(r.insertsExclusive) : SHOWN,
    columnInserts: !plain ? HIDDEN : o.inserts ? off(r.insertsExclusive) : SHOWN,
    compress: format === "tar" ? HIDDEN : SHOWN,
    jobs: format === "directory" ? SHOWN : HIDDEN,
    encoding: SHOWN,
  };
}

/** Görünmez ya da devre dışı kalan seçenekleri varsayılana döndürür. Çelişen çiftte bir tanesi (listedeki ilk işlenen) temizlenir. */
export function normalizeBackup(format: DumpFormat, o: BackupOptions): BackupOptions {
  const out = { ...o };
  const defaults = defaultBackupOptions();
  // Sıra önemli: dışlama çiftlerinde önce işlenen (dataOnly, columnInserts) çelişkide temizlenir; ifExists clean'den sonra gelir.
  const order: BackupKey[] = ["dataOnly", "clean", "ifExists", "columnInserts", "inserts", "schemaOnly", "create", "noOwner", "noPrivileges", "compress", "jobs", "encoding"];
  for (const k of order) {
    const s = backupOptionStates(format, out)[k];
    if (!s.visible || !s.enabled) (out as Record<BackupKey, unknown>)[k] = defaults[k];
  }
  return out;
}

// ---- Geri yükleme seçenekleri ----------------------------------------------------------------------------------

export function restoreOptionStates(src: SourceFormat, target: RestoreTarget, o: RestoreOptions): Record<RestoreKey, OptionState> {
  const r = tb.reason;
  const plain = src === "plain";
  const hid = (s: OptionState): OptionState => (plain ? HIDDEN : s);
  const singleTransaction: OptionState =
    target === "create_from_archive" ? off(r.createSingleTransaction) : o.jobs !== null && !plain ? off(r.jobsSingleTransaction) : SHOWN;
  const jobs: OptionState = src === "tar" ? off(r.tarNoParallel) : o.singleTransaction ? off(r.jobsSingleTransaction) : SHOWN;
  return {
    clean: hid(o.dataOnly ? off(r.cleanDataOnly) : SHOWN),
    ifExists: hid(o.clean ? SHOWN : off(r.ifExistsNeedsClean)),
    singleTransaction,
    jobs: hid(jobs),
    schemaOnly: hid(o.dataOnly ? off(r.schemaDataExclusive) : SHOWN),
    dataOnly: hid(o.schemaOnly ? off(r.schemaDataExclusive) : o.clean ? off(r.cleanDataOnly) : SHOWN),
    noOwner: hid(SHOWN),
    noPrivileges: hid(SHOWN),
    exitOnError: hid(SHOWN),
  };
}

/** `create` hedeften türetilir (arka uç `--create`'i hedeften de çıkarır; spec §5). */
export function normalizeRestore(src: SourceFormat, target: RestoreTarget, o: RestoreOptions): RestoreOptions {
  const out: RestoreOptions = { ...o, create: target === "create_from_archive" };
  const defaults = defaultRestoreOptions();
  const order: RestoreKey[] = ["dataOnly", "clean", "ifExists", "schemaOnly", "singleTransaction", "jobs", "noOwner", "noPrivileges", "exitOnError"];
  for (const k of order) {
    const s = restoreOptionStates(src, target, out)[k];
    if (!s.visible || !s.enabled) (out as Record<RestoreKey, unknown>)[k] = defaults[k];
  }
  return out;
}

// ---- Kapsam ------------------------------------------------------------------------------------------------------

type TreeLike = { nodes: Map<string, TreeNode>; parent: Map<string, string> };

function ancestors(tree: TreeLike, id: string): TreeNode[] {
  const out: TreeNode[] = [];
  const seen = new Set<string>([id]);
  let cur = tree.parent.get(id);
  while (cur !== undefined && !seen.has(cur)) {
    seen.add(cur);
    const n = tree.nodes.get(cur);
    if (!n) break;
    out.push(n);
    cur = tree.parent.get(cur);
  }
  return out;
}

/** Düğümden yedek kapsamı. Bölüm (partition) altı ve desteklenmeyen türler null (şema atadan güvenle okunamaz). */
export function backupScopeOf(
  tree: TreeLike,
  nodeId: string,
): { scope: BackupScope; dbNodeId: string; dbName: string; label: string } | null {
  const node = tree.nodes.get(nodeId);
  if (!node) return null;
  const up = ancestors(tree, nodeId);
  if (up.some((a) => a.kind === "partitionCollection")) return null;
  const db = node.kind === "database" ? node : up.find((a) => a.kind === "database");
  if (!db) return null;
  const base = { dbNodeId: db.id, dbName: db.label };
  switch (node.kind) {
    case "database":
      return { scope: { kind: "database" }, ...base, label: db.label };
    case "schema":
      return { scope: { kind: "schema", name: node.label }, ...base, label: node.label };
    case "table":
    case "materializedView":
    case "sequence": {
      const schema = up.find((a) => a.kind === "schema");
      if (!schema) return null;
      return { scope: { kind: "table", schema: schema.label, name: node.label }, ...base, label: `${schema.label}.${node.label}` };
    }
    default:
      return null;
  }
}

// ---- Dosya adı ve uzantı -----------------------------------------------------------------------------------------

export function fileExt(format: DumpFormat, compress: number | null): string {
  switch (format) {
    case "custom": return ".backup";
    case "plain": return compress !== null && compress > 0 ? ".sql.gz" : ".sql";
    case "tar": return ".tar";
    case "directory": return "";
  }
}

// eslint-disable-next-line no-control-regex
const INVALID_NAME_CHARS = /[<>:"/\\|?*\u0000-\u001f]/g;
const pad = (n: number, w = 2) => String(n).padStart(w, "0");

/** `<parça>_<parça>…_<YYYYMMDD>_<HHmm><uzantı>`; Windows'ta geçersiz karakterler `_` olur. */
export function suggestFileName(parts: string[], format: DumpFormat, compress: number | null, now: Date): string {
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}`;
  const body = parts.map((p) => p.replace(INVALID_NAME_CHARS, "_")).join("_");
  return `${body}_${stamp}${fileExt(format, compress)}`;
}

type FormatSpec = { format: DumpFormat; compress: number | null };

/**
 * Yol bizim önerdiğimiz (eski) uzantıyla bitiyorsa uzantı yeni formata çevrilir; directory'ye geçişte atılır.
 * Directory'den geçişte eski uzantı olmadığından yeni uzantı eklenir. Kullanıcının kendi uzantısı korunur.
 */
export function swapExtension(path: string, from: FormatSpec, to: FormatSpec): string {
  if (path === "") return path;
  const oldExt = fileExt(from.format, from.compress);
  const newExt = fileExt(to.format, to.compress);
  if (oldExt === newExt) return path;
  const lower = path.toLowerCase();
  if (oldExt === "") return newExt !== "" && !lower.endsWith(newExt) ? path + newExt : path;
  if (!lower.endsWith(oldExt)) return path;
  return path.slice(0, path.length - oldExt.length) + newExt;
}

export function saveFilters(format: DumpFormat, compress: number | null): { name: string; extensions: string[] }[] {
  switch (format) {
    case "custom": return [{ name: tb.filters.custom, extensions: ["backup", "dump"] }];
    case "plain": return compress !== null && compress > 0 ? [{ name: tb.filters.plainGz, extensions: ["sql.gz"] }] : [{ name: tb.filters.plain, extensions: ["sql"] }];
    case "tar": return [{ name: tb.filters.tar, extensions: ["tar"] }];
    case "directory": return [];
  }
}

// ---- Hata ve uyarılar --------------------------------------------------------------------------------------------

/** `fields` görünür alan yollarıdır; kalan her yol (`tool`, `database`, `request`, `ssl.*` …) `general`'e gider. Aynı alanda ilk ileti kazanır. */
export function splitFieldErrors(errors: FieldError[], fields: readonly string[]): { byField: Record<string, string>; general: string[] } {
  const visible = new Set(fields);
  const byField: Record<string, string> = {};
  const general: string[] = [];
  for (const e of errors) {
    if (visible.has(e.path)) {
      if (!(e.path in byField)) byField[e.path] = e.message;
    } else general.push(e.message);
  }
  return { byField, general };
}

/** Sunucu düğümünün `detail` metninden ana sürüm ("16.4" → 16, "9.6.24" → 9, "18beta1" → 18). */
export function serverMajor(detail: string | null): number | null {
  if (detail === null) return null;
  const m = /^\s*(?:PostgreSQL\s+)?(\d+)/i.exec(detail);
  return m ? Number(m[1]) : null;
}

export function toolAdvice(tool: ToolVersion | null, serverMajorVersion: number | null, job: "backup" | "restore" | "psql"): { tone: "info" | "warning"; text: string } | null {
  if (!tool || serverMajorVersion === null || job === "psql") return null;
  if (tool.major === serverMajorVersion) return null;
  if (job === "restore" && tool.major >= 17 && serverMajorVersion < 17) return { tone: "warning", text: tb.advice.restoreOld(tool.major, serverMajorVersion) };
  return { tone: "info", text: tb.advice.info(serverMajorVersion, tool.major) };
}

const normDir = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();

/** Seçili klasörün kümesini bulur; sürüm o kümeden gelir. */
export function chosenTool(det: ToolDetection, which: "dump" | "restore" | "psql"): { dir: string; version: ToolVersion } | null {
  const dir = which === "dump" ? det.chosenDump : which === "restore" ? det.chosenRestore : det.chosenPsql;
  if (dir === null) return null;
  const set = det.sets.find((s) => s.dir === dir) ?? det.sets.find((s) => normDir(s.dir) === normDir(dir));
  return set ? { dir, version: set.version } : null;
}

export function errorText(p: ErrorPayload): string {
  if (p.code === "password_required") return `${p.message} ${tb.passwordAdvice}`;
  return p.hint ? `${p.message} ${p.hint}` : p.message;
}

// ---- Günlük ve ilerleme ------------------------------------------------------------------------------------------

export type LogLine = { severity: LineSeverity; text: string };
export interface JobLog {
  preview: string | null;
  lines: LogLine[];
  progress: { done: number; total: number | null; bytes: number | null } | null;
  summary: JobSummary | null;
}

export function emptyLog(): JobLog {
  return { preview: null, lines: [], progress: null, summary: null };
}

/** Olayı günlüğe işler. `finished` son olaydır; sonrasındakiler yok sayılır. Satırlar olduğu gibi tutulur (arka uç 2000 + taşma satırı sınırlar). */
export function applyEvent(log: JobLog, ev: BackupEvent): JobLog {
  if (log.summary !== null) return log;
  switch (ev.event) {
    case "started":
      return { ...log, preview: ev.preview };
    case "line":
      return { ...log, lines: [...log.lines, { severity: ev.severity, text: ev.text }] };
    case "progress":
      return { ...log, progress: { done: ev.done, total: ev.total, bytes: ev.bytes } };
    case "finished":
      return {
        ...log,
        summary: {
          outcome: ev.outcome, exitCode: ev.exitCode, elapsedMs: ev.elapsedMs, warnings: ev.warnings, errors: ev.errors,
          outputPath: ev.outputPath, hint: ev.hint, truncatedLines: ev.truncatedLines,
        },
      };
  }
}

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  if (n < 1024) return `${Math.floor(n)} B`;
  let v = n;
  let i = 0;
  while (i < UNITS.length - 1 && Number(v.toFixed(1)) >= 1024) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1).replace(".", ",")} ${UNITS[i]}`;
}

/** mm:ss; bir saat ve üzeri h:mm:ss. */
export function elapsedLabel(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

export function progressView(p: JobLog["progress"], job: "backup" | "restore"): { percent: number | null; text: string } {
  if (p === null) return { percent: null, text: tb.progress.starting };
  const unit = job === "backup" ? tb.progress.unitBackup : tb.progress.unitRestore;
  const bytes = p.bytes !== null ? ` · ${formatBytes(p.bytes)}` : "";
  if (p.total !== null && p.total > 0) {
    const done = Math.min(Math.max(p.done, 0), p.total);
    return { percent: Math.floor((done / p.total) * 100), text: tb.progress.of(done, p.total, unit) + bytes };
  }
  return { percent: null, text: tb.progress.count(p.done, unit) + bytes };
}

// ---- Sonuç -------------------------------------------------------------------------------------------------------

export type ResultTone = "success" | "warning" | "error" | "neutral";

export function resultView(s: JobSummary, job: "backup" | "restore", singleTransaction: boolean): { tone: ResultTone; title: string; lines: string[] } {
  const r = tb.result;
  const lines: string[] = [];
  if (s.hint) lines.push(s.hint);
  if (s.outcome !== "success") lines.push(r.elapsed(elapsedLabel(s.elapsedMs)));
  if (s.warnings > 0) lines.push(r.warnings(s.warnings));
  if (s.errors > 0) lines.push(r.errors(s.errors));
  if (s.outcome === "cancelled" && job === "restore" && !singleTransaction) lines.push(r.halfRestore);
  if (s.truncatedLines > 0) lines.push(r.truncated(s.truncatedLines));
  switch (s.outcome) {
    case "success": return { tone: "success", title: r.success(elapsedLabel(s.elapsedMs)), lines };
    case "completed_with_errors": return { tone: "warning", title: r.completedWithErrors(s.errors), lines };
    case "failed": return { tone: "error", title: r.failed, lines };
    case "cancelled": return { tone: "neutral", title: r.cancelled, lines };
  }
}

// ---- Kapılar -----------------------------------------------------------------------------------------------------

export type Gate =
  | { kind: "blocked"; reason: keyof typeof tb.blocked }
  | { kind: "ready" }
  | { kind: "needsConfirm" }
  | { kind: "needsProductionName"; expected: string };

const blocked = (reason: keyof typeof tb.blocked): Gate => ({ kind: "blocked", reason });
const planInvalid = (plan: JobPlan | null) => plan === null || plan.errors.length > 0;

export function backupGate(i: { running: boolean; hasPath: boolean; pending: boolean; planCurrent: boolean; plan: JobPlan | null }): Gate {
  if (i.running) return blocked("running");
  if (!i.hasPath) return blocked("noPath");
  if (i.pending) return blocked("pending");
  if (!i.planCurrent) return blocked("stale");
  if (planInvalid(i.plan)) return blocked("invalid");
  return { kind: "ready" };
}

export function restoreGate(i: {
  locked: boolean; running: boolean; inspecting: boolean; hasSource: boolean; hasTarget: boolean; pending: boolean; planCurrent: boolean;
  plan: JobPlan | null; production: boolean; serverName: string;
}): Gate {
  if (i.locked) return blocked("locked");
  if (i.running) return blocked("running");
  if (i.inspecting) return blocked("inspecting");
  if (!i.hasSource) return blocked("noSource");
  if (!i.hasTarget) return blocked("noTarget");
  if (i.pending) return blocked("pending");
  if (!i.planCurrent) return blocked("stale");
  if (i.plan === null || i.plan.errors.length > 0) return blocked("invalid");
  switch (i.plan.risk) {
    case "safe": return { kind: "ready" };
    case "destructive": return { kind: "needsConfirm" };
    case "data_loss": return i.production ? { kind: "needsProductionName", expected: i.serverName } : { kind: "needsConfirm" };
  }
}

type Confirm = { checked: boolean; typedName: string };
// Kırpılmış tam eşleşme, büyük/küçük harf duyarlı; boş beklenen ad asla eşleşmez (designer applyGate kuralının küçük kopyası).
const nameMatches = (expected: string, typed: string) => expected !== "" && typed.trim() === expected;

export function canStart(g: Gate, c: Confirm): boolean {
  switch (g.kind) {
    case "ready": return true;
    case "needsConfirm": return c.checked;
    case "needsProductionName": return c.checked && nameMatches(g.expected, c.typedName);
    case "blocked": return false;
  }
}

export function confirmFlags(g: Gate, c: Confirm): { confirmed: boolean; confirmedProduction: boolean } {
  switch (g.kind) {
    case "needsConfirm": return { confirmed: c.checked, confirmedProduction: false };
    case "needsProductionName": return { confirmed: c.checked, confirmedProduction: c.checked && nameMatches(g.expected, c.typedName) };
    default: return { confirmed: false, confirmedProduction: false };
  }
}
