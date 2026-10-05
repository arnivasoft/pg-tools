// Sorgu çalışma alanı kararları (saf): açma bağlamı, ortam istemi, kapanış/çıkış diyalogları, hata yönlendirme,
// taslak dönüşümü ve debounce, sekme ↔ editör durumu defteri. CodeMirror/ızgara içe aktarmaz (ana pakette kalır).
import { t } from "../../app/strings";
import type { ClosePolicy } from "../../ipc/bindings/ClosePolicy";
import type { DraftTab } from "../../ipc/bindings/DraftTab";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { Scope } from "../../ipc/bindings/Scope";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { SessionOptions } from "../../ipc/bindings/SessionOptions";
import type { TxState } from "../../ipc/bindings/TxState";
import { qualifiedParts } from "../explorer/actions";
import { ancestorOfKind, serverOf, type TreeState } from "../explorer/tree-model";
import { isProductionLabel, suggestLock } from "../servers/server-form";
import { canOpenQueryTool } from "./open-kinds";
import { closeChoices, type CloseChoice, type RunState } from "./run-model";

// ---------------------------------------------------------------- Açma

export { QUERY_TOOL_KINDS, canOpenQueryTool } from "./open-kinds";
const RELATION_KINDS: readonly NodeKind[] = ["table", "view", "materializedView"];

export interface OpenContext {
  nodeId: string;
  serverId: string;
  /** Ağaçtaki veritabanı adı; `null` = sunucu düğümü (bakım veritabanı). */
  database: string | null;
  /** Tablo/görünüm için tırnaklanacak `[şema, ad]`; bölüm (partition) altında ya da diğer düğümlerde `null`. */
  relation: [string, string] | null;
}

/** Düğümden açma bağlamı. Kimlik ayrıştırılmaz: sunucu ve veritabanı ağaç atalarından okunur. */
export function openContext(s: TreeState, nodeId: string): OpenContext | null {
  const node = s.nodes.get(nodeId);
  if (!node || !canOpenQueryTool(node.kind)) return null;
  const server = serverOf(s, nodeId);
  if (!server?.serverId) return null;
  const db = ancestorOfKind(s, nodeId, ["database"]);
  let relation: [string, string] | null = null;
  if (RELATION_KINDS.includes(node.kind)) {
    const q = qualifiedParts(s, nodeId);
    if (q && q.parts.length === 2) relation = [q.parts[0], q.parts[1]];
  }
  return { nodeId, serverId: server.serverId, database: db?.label ?? null, relation };
}

/** "Yeni sorgu": seçili düğümün veritabanı atası, yoksa sunucusu (boş taslak). */
export function newQueryNode(s: TreeState, selected: string | null): string | null {
  if (selected === null) return null;
  return ancestorOfKind(s, selected, ["database"])?.id ?? serverOf(s, selected)?.id ?? null;
}

/** Tabloda/görünümde `SELECT * FROM <nitelikli ad> LIMIT 100;`, aksi halde boş. `quoted` arka uçta tırnaklanmış adlardır. */
export function initialSql(quoted: string[] | null): string {
  return quoted && quoted.length > 0 ? `SELECT * FROM ${quoted.join(".")} LIMIT 100;\n` : "";
}

/** Üretim etiketinde varsayılan kip Manuel. */
export function defaultOptions(label: string | null | undefined): SessionOptions {
  return { mode: isProductionLabel(label) ? "manual" : "autocommit", statementTimeoutMs: 0 };
}

export const TIMEOUT_CHOICES = [30_000, 300_000, 0] as const;

// ---------------------------------------------------------------- Ortam istemi

export type EnvChoice = "production" | "test" | "development";
export const ENV_CHOICES: readonly EnvChoice[] = ["production", "test", "development"];

export function envLabel(choice: EnvChoice): string {
  const [prod, test, dev] = t.server.labelSuggestions;
  return choice === "production" ? prod : choice === "test" ? test : dev;
}

/** Etiketsiz sunucuda (ve daha önce "Atla" denmemişse) açılışta ortam sorulur. */
export function needsEnvPrompt(config: Pick<ServerConfig, "id" | "label">, skipped: ReadonlySet<string>): boolean {
  return (config.label ?? "").trim() === "" && !skipped.has(config.id);
}

/** Seçime göre önerilen kilit (sunucu formundaki `suggestLock` ile aynı kural). */
export function envLockDefault(config: Pick<ServerConfig, "label" | "readOnlyLock">, choice: EnvChoice): boolean {
  return suggestLock(config.label ?? "", envLabel(choice), config.readOnlyLock, false);
}

export function applyEnvChoice(config: ServerConfig, choice: EnvChoice, lock: boolean): ServerConfig {
  return { ...config, label: envLabel(choice), readOnlyLock: choice === "production" ? lock : config.readOnlyLock };
}

/**
 * Arka uç `save_registered` ile aynı: kayıtlı sunucuda parola yazıldıysa ya da hedef, SSL, kilit veya üretim olma
 * durumu değiştiyse oturumlar kapanır (yeni kayıtta oturum yoktur).
 */
export function saveClosesSessions(prev: ServerConfig | null, next: ServerConfig, passwordTyped = false): boolean {
  if (!prev) return false;
  return (
    passwordTyped ||
    prev.host !== next.host ||
    prev.port !== next.port ||
    prev.username !== next.username ||
    prev.maintenanceDb !== next.maintenanceDb ||
    JSON.stringify(prev.ssl) !== JSON.stringify(next.ssl) ||
    prev.readOnlyLock !== next.readOnlyLock ||
    isProductionLabel(prev.label) !== isProductionLabel(next.label)
  );
}

// ---------------------------------------------------------------- Kapanış

const isRunning = (s: RunState) => s.status === "running" || s.status === "cancelling";

/**
 * Sekme kapanışı:
 * - oturum yok → doğrudan kapat (`policy: null`);
 * - açık/hatalı transaction → diyalog (`closeChoices`; hatalıda/meşgulde COMMIT yok);
 * - meşgul ama transaction yok → `rollback` (çalışan sorgu iptal edilir; ≤ ~10 sn) ve bilgi;
 * - boşta → `refuse` (durum bayatsa arka uç `open_transaction` döner, o zaman diyalog gösterilir).
 */
export type ClosePlan =
  | { kind: "close"; policy: ClosePolicy | null; busy: boolean }
  | { kind: "confirm"; choices: CloseChoice[]; busy: boolean };

export function closePlan(session: string | null, run: RunState): ClosePlan {
  if (!session || run.status === "lost" || run.status === "noSession") return { kind: "close", policy: null, busy: false };
  const busy = isRunning(run);
  const choices = closeChoices(run);
  if (choices.length > 0) return { kind: "confirm", choices, busy };
  return { kind: "close", policy: busy ? "rollback" : "refuse", busy };
}

export function choicePolicy(choice: Exclude<CloseChoice, "cancel">): ClosePolicy {
  return choice === "commit" ? "commit" : "rollback";
}

export interface PendingTx {
  id: string;
  title: string;
  tx: TxState;
  busy: boolean;
}

export interface TabTxInfo {
  id: string;
  title: string;
  serverId: string | null;
  session: string | null;
  run: RunState;
}

/** Açık ya da hatalı transaction'ı olan oturumlu sekmeler (isteğe bağlı olarak tek sunucu). */
export function pendingTransactions(tabs: TabTxInfo[], serverId?: string): PendingTx[] {
  return tabs
    .filter((x) => x.session !== null && (serverId === undefined || x.serverId === serverId))
    .filter((x) => x.run.status !== "lost" && (x.run.tx === "inTransaction" || x.run.tx === "failed"))
    .map((x) => ({ id: x.id, title: x.title, tx: x.run.tx, busy: isRunning(x.run) }));
}

/** "Tümünü kaydet" yalnızca en az bir sekme COMMIT edilebiliyorsa sunulur. */
export function canCommitAny(list: PendingTx[]): boolean {
  return list.some((p) => p.tx === "inTransaction" && !p.busy);
}

/** Toplu seçimde sekme başına politika: hatalı ya da meşgul sekme her durumda geri alınır. */
export function bulkPolicy(p: PendingTx, choice: "commit" | "rollback"): ClosePolicy {
  return choice === "commit" && p.tx === "inTransaction" && !p.busy ? "commit" : "rollback";
}

// ---------------------------------------------------------------- Hata yönlendirme

/**
 * - `toast`: `busy` (rutin, modal yok, sekme boşta kalır)
 * - `reset`: `session_reset` (Mesajlar'da açıklama + "Tekrar çalıştır")
 * - `lost`: oturum/sunucu yok (`not_connected`, `unknown_server`, `not_found`) → "Yeniden bağlan"
 * - `message`: diğerleri Mesajlar'da (ipucu `hintFor` ile)
 */
export type ErrorRoute = "toast" | "reset" | "lost" | "message";

export function routeError(e: Pick<ErrorPayload, "code">): ErrorRoute {
  switch (e.code) {
    case "busy":
      return "toast";
    case "session_reset":
      return "reset";
    case "not_connected":
    case "unknown_server":
    case "not_found":
      return "lost";
    default:
      return "message";
  }
}

export function sameScope(a: Scope | null, b: Scope | null): boolean {
  return a !== null && b !== null && JSON.stringify(a) === JSON.stringify(b);
}

// ---------------------------------------------------------------- Taslaklar

export interface DraftSource {
  id: string;
  title: string;
  serverId: string | null;
  databaseOid: number | null;
  database: string | null;
  options: SessionOptions;
}

export function toDraft(x: DraftSource): DraftTab {
  return {
    id: x.id,
    title: x.title,
    serverId: x.serverId,
    databaseOid: x.databaseOid,
    database: x.database,
    options: { mode: x.options.mode, statementTimeoutMs: x.options.statementTimeoutMs },
  };
}

export interface RestoredTab extends DraftSource {
  serverName: string;
  /** Sunucu kaydı silinmiş: salt okunur taslak ("sunucu yok"). */
  serverGone: boolean;
  text: string;
}

/** Taslaktan sekme (bağlanmadan). Veritabanı adı yoksa bakım veritabanı ya da OID gösterilir. */
export function restoreDraft(d: DraftTab, text: string, servers: ServerConfig[]): RestoredTab {
  const server = d.serverId ? servers.find((s) => s.id === d.serverId) : undefined;
  const database = d.database ?? (d.databaseOid === null ? (server?.maintenanceDb ?? null) : `OID ${d.databaseOid}`);
  return {
    id: d.id,
    title: d.title,
    serverId: d.serverId,
    databaseOid: d.databaseOid,
    database,
    options: { mode: d.options.mode, statementTimeoutMs: d.options.statementTimeoutMs },
    serverName: server?.name ?? t.query.ctx.noServer,
    serverGone: !server,
    text,
  };
}

export interface Timers {
  set: (fn: () => void, ms: number) => unknown;
  clear: (handle: unknown) => void;
}

const realTimers: Timers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

/** Kimlik başına debounce: `touch` sessizlik süresini yeniden başlatır; `flush` bekleyeni hemen çalıştırır. */
export function createDebouncer(delayMs: number, run: (id: string) => void | Promise<void>, timers: Timers = realTimers) {
  const pending = new Map<string, unknown>();
  const fire = (id: string) => {
    pending.delete(id);
    return run(id);
  };
  return {
    touch(id: string) {
      const h = pending.get(id);
      if (h !== undefined) timers.clear(h);
      pending.set(id, timers.set(() => void fire(id), delayMs));
    },
    async flush(id: string) {
      const h = pending.get(id);
      if (h === undefined) return;
      timers.clear(h);
      await fire(id);
    },
    async flushAll() {
      await Promise.all([...pending.keys()].map((id) => this.flush(id)));
    },
    cancel(id: string) {
      const h = pending.get(id);
      if (h !== undefined) timers.clear(h);
      pending.delete(id);
    },
    isPending: (id: string) => pending.has(id),
  };
}

export const DRAFT_DEBOUNCE_MS = 5000;

// ---------------------------------------------------------------- Transaction süresi

/** Transaction açıldığı an: boştan açık/hatalıya geçişte `now`, boşa dönüşte `null`, aksi halde aynı. */
export function nextTxSince(prev: TxState, next: TxState, since: number | null, now: number): number | null {
  if (next === "idle") return null;
  if (prev === "idle" || since === null) return now;
  return since;
}

export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const p = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${p(m)}:${p(s)}` : `${p(m)}:${p(s)}`;
}

// ---------------------------------------------------------------- Sekme ↔ editör durumu

export interface SavedView<S> {
  state: S;
  scrollTop: number;
  scrollLeft: number;
}

/**
 * Tek editör, sekme başına bir durum. `from` sekmesinin anlık durumu kaydedilir; `to` için kayıtlı durum ya da
 * `create()` döner. Aynı sekmeye dönüşte yalnızca kaydırma geri yüklenir (durum değişmez).
 */
export function swapEditorState<S>(
  book: Map<string, SavedView<S>>,
  from: string | null,
  current: SavedView<S> | null,
  to: string,
  create: () => S,
): SavedView<S> {
  if (from !== null && current !== null) book.set(from, current);
  const saved = book.get(to);
  if (saved) return saved;
  const fresh = { state: create(), scrollTop: 0, scrollLeft: 0 };
  book.set(to, fresh);
  return fresh;
}

/** Sekme/şerit üst çizgisinin rengi: sunucu rengi; renk yoksa üretimde tehlike rengi, değilse yok. */
export function serverLineColor(config: Pick<ServerConfig, "color" | "label"> | null): string | null {
  if (config?.color) return `var(--srv-${config.color})`;
  return isProductionLabel(config?.label) ? "var(--danger)" : null;
}
