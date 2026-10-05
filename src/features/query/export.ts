// CSV dışa aktarma (saf kural + bağımlılıkları enjekte edilen akış). Arka uç sözleşmesi:
// docs/query-tool-plan6-notes.md ("Diğer"): ayrı imleç, yalnızca tek SELECT/VALUES/TABLE/WITH, hedef yeniden
// adlandırmayla ezilir (onay), Excel kilidi ancak sonda görülür, `session_reset` dönebilir.
import { t } from "../../app/strings";
import type { CsvOptions } from "../../ipc/bindings/CsvOptions";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import type { ExportSummary } from "../../ipc/bindings/ExportSummary";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { toPayload } from "../../ipc/errors";
import { hintFor } from "./errors";
import { stmtText } from "./offsets";
import { isUserCancel } from "./result-model";

export const DEFAULT_CSV_OPTIONS: CsvOptions = { delimiter: "comma", header: true, bom: true };

/** Aktarılacak ifade: seçimde (analiz sonucu) tam olarak bir satır döndüren (`select`) ifade ve bir oturum. */
export function exportTarget(plans: StmtPlan[], hasSession: boolean): StmtPlan | null {
  if (!hasSession) return null;
  const selects = plans.filter((p) => p.kind === "select");
  return selects.length === 1 ? selects[0] : null;
}

export const canExport = (plans: StmtPlan[], hasSession: boolean) => exportTarget(plans, hasSession) !== null;

/** Aktarılacak SQL: yalnızca hedef ifadenin metni (betiğin diğer ifadeleri değil). */
export const exportSql = (text: string, plan: StmtPlan) => stmtText(text, plan);

const pad = (n: number) => String(n).padStart(2, "0");

/** `<tablo|sorgu>-<YYYYMMDD-HHmm>.csv`; yerel saat. Dosya adına uygun olmayan karakterler `_` olur. */
export function defaultFileName(relation: string | null, now: Date): string {
  const base = (relation?.trim() || t.query.export.defaultName).replace(/[\/:*?"<>|\u0000-\u001f]/g, "_");
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}`;
  return `${base}-${stamp}.csv`;
}

export interface ExportDeps {
  /** Seçenek diyaloğu; `null` = vazgeçildi. */
  askOptions: () => Promise<CsvOptions | null>;
  /** Kaydet diyaloğu; `null` = vazgeçildi. */
  pickPath: (defaultName: string) => Promise<string | null>;
  exists: (path: string) => Promise<boolean>;
  confirmOverwrite: (path: string) => Promise<boolean>;
  exportCsv: (session: string, sql: string, path: string, options: CsvOptions, onProgress: (rows: number) => void) => Promise<ExportSummary>;
  onProgress: (rows: number) => void;
  /** İlerleme göstergesini aç/kapat (başlarken true, biterken false). */
  setRunning: (running: boolean) => void;
}

export type ExportOutcome =
  | { kind: "aborted" }
  | { kind: "done"; summary: ExportSummary; path: string }
  | { kind: "cancelled" }
  | { kind: "error"; error: ErrorPayload; hint: string | null };

/** Dışa aktarma hatası için ipucu: kod → ipucu; dosya (kod `query` değil) hataları Excel kilidini anar. */
export function exportErrorHint(error: ErrorPayload): string | null {
  const known = hintFor(error);
  if (known) return known;
  if (error.code === "busy") return t.query.export.busy;
  if (["query", "not_found", "not_connected", "unknown_server", "confirmation_required", "tx_failed"].includes(error.code)) return null;
  return t.query.export.lockedTarget;
}

/**
 * Akış: seçenekler → kaydet diyaloğu → hedef varsa ezme onayı → dışa aktar. Kullanıcı iptali (`qtCancel` →
 * 57014 iletisi) `cancelled` olur. Çağıran iptal düğmesinde `qtCancel(session)` çağırır.
 */
export async function runExport(
  deps: ExportDeps,
  input: { session: string; sql: string; relation: string | null; now: Date },
): Promise<ExportOutcome> {
  const options = await deps.askOptions();
  if (!options) return { kind: "aborted" };
  const path = await deps.pickPath(defaultFileName(input.relation, input.now));
  if (!path) return { kind: "aborted" };
  try {
    if ((await deps.exists(path)) && !(await deps.confirmOverwrite(path))) return { kind: "aborted" };
  } catch (e) {
    return { kind: "error", error: toPayload(e), hint: null };
  }
  deps.setRunning(true);
  try {
    const summary = await deps.exportCsv(input.session, input.sql, path, options, deps.onProgress);
    return { kind: "done", summary, path };
  } catch (e) {
    const error = toPayload(e);
    if (isUserCancel(error)) return { kind: "cancelled" };
    return { kind: "error", error, hint: exportErrorHint(error) };
  } finally {
    deps.setRunning(false);
  }
}
