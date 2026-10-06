// CSV dışa aktarma (saf kural + bağımlılıkları enjekte edilen akış). Arka uç sözleşmesi:
// docs/query-tool-plan6-notes.md ("Diğer"): ayrı imleç, yalnızca tek SELECT/VALUES/TABLE/WITH, hedef yeniden
// adlandırmayla ezilir, Excel kilidi ancak sonda görülür, `session_reset` dönebilir.
// Üzerine yazma onayı: Windows'ta yerel kaydet diyaloğu (rfd 0.17, IFileSaveDialog varsayılan FOS_OVERWRITEPROMPT)
// var olan dosya için kendisi sorar (spec §8); bu yüzden ayrıca kendi onayımız ve `fs_exists` yoktur.
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
export function exportTarget(plans: StmtPlan[], hasSession: boolean, preferredIndex?: number | null): StmtPlan | null {
  if (!hasSession) return null;
  // Açıkça seçilmiş ifade (şeritte seçili) varsa o; select değilse dışa aktarma kapalı. Yoksa eski kural.
  if (preferredIndex != null) {
    const p = plans.find((x) => x.index === preferredIndex);
    if (p) return p.kind === "select" ? p : null;
  }
  const selects = plans.filter((p) => p.kind === "select");
  return selects.length === 1 ? selects[0] : null;
}

/** Seçenek diyaloğunda gösterilen etiket: "İfade N: <ilk 80 karakter>" (boşluklar tek satıra indirilir). */
export function statementLabel(index: number, sql: string): string {
  const flat = Array.from(sql.replace(/\s+/g, " ").trim());
  const head = flat.slice(0, 80).join("") + (flat.length > 80 ? "…" : "");
  return t.query.export.statementLabel(index + 1, head);
}

/** Aktarılacak SQL: yalnızca hedef ifadenin metni (betiğin diğer ifadeleri değil). */
export const exportSql = (text: string, plan: StmtPlan) => stmtText(text, plan);

const pad = (n: number) => String(n).padStart(2, "0");

const RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;
const MAX_BASE = 100;

/** Windows dosya adı için güvenli temel ad: yasak karakterler, ayrılmış adlar, sondaki nokta/boşluk, uzunluk. */
export function sanitizeFileBase(raw: string): string {
  let base = raw.replace(/[\x5c/:*?"<>|\u0000-\u001f]/g, "_");
  base = Array.from(base).slice(0, MAX_BASE).join("").replace(/[. ]+$/, "").trim();
  if (base === "") base = t.query.export.defaultName;
  return RESERVED.test(base) ? `_${base}` : base;
}

/** `<tablo|sorgu>-<YYYYMMDD-HHmm>.csv`; yerel saat. */
export function defaultFileName(relation: string | null, now: Date): string {
  const base = sanitizeFileBase(relation?.trim() || t.query.export.defaultName);
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}`;
  return `${base}-${stamp}.csv`;
}

export interface ExportDeps {
  /** Seçenek diyaloğu (`label`: aktarılan ifade); `null` = vazgeçildi. */
  askOptions: (label: string) => Promise<CsvOptions | null>;
  /** Kaydet diyaloğu; `null` = vazgeçildi. */
  pickPath: (defaultName: string) => Promise<string | null>;
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

/** Dışa aktarma hatası için ipucu. Dosya/yeniden adlandırma hataları `Error::Config` → kod `config` (Excel kilidi ipucu). */
export function exportErrorHint(error: ErrorPayload): string | null {
  const known = hintFor(error);
  if (known) return known;
  if (error.code === "busy") return t.query.export.busy;
  return error.code === "config" ? t.query.export.lockedTarget : null;
}

/**
 * Akış: seçenekler → kaydet diyaloğu (üzerine yazmayı işletim sistemi sorar) → dışa aktar. Kullanıcı iptali (`qtCancel` →
 * 57014 iletisi) `cancelled` olur. Çağıran iptal düğmesinde `qtCancel(session)` çağırır.
 */
export async function runExport(
  deps: ExportDeps,
  input: { session: string; sql: string; relation: string | null; stmtIndex: number; now: Date },
): Promise<ExportOutcome> {
  const options = await deps.askOptions(statementLabel(input.stmtIndex, input.sql));
  if (!options) return { kind: "aborted" };
  const path = await deps.pickPath(defaultFileName(input.relation, input.now));
  if (!path) return { kind: "aborted" };
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
