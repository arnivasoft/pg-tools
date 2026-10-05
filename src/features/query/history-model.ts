// Geçmiş modeli (saf): çalıştırma sonrası geçmişe yazılacak ifadeler, arama ve editöre ekleme.
import type { HistoryEntry } from "../../ipc/bindings/HistoryEntry";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { stmtText } from "./offsets";
import type { ResultState } from "./result-model";

export interface HistoryContext {
  /** Çalıştırılan betiğin tam metni (planların `charOffset`/`charLen` aralıkları bu metne göredir). */
  text: string;
  serverId: string;
  serverName: string;
  database: string | null;
  atMs: number;
  newId: () => string;
}

/**
 * `scriptDone` sonrası geçmişe yazılacak girdiler: yalnızca gerçekten çalışmış (ok/failed/cancelled) ifadeler,
 * ifade başına bir girdi. Arka uç planında `sensitive` olanlar hiç gönderilmez. Başarısız/iptal `ok:false`;
 * `failed` olayı süre taşımadığından süre 0 olur. Planı olmayan ifade atlanır.
 */
export function historyEntriesFor(plans: StmtPlan[], result: ResultState, ctx: HistoryContext): HistoryEntry[] {
  const byIndex = new Map(plans.map((p) => [p.index, p]));
  const out: HistoryEntry[] = [];
  for (const s of result.stmts) {
    if (s.status === "running") continue;
    const plan = byIndex.get(s.index);
    if (!plan || plan.sensitive) continue;
    const sql = stmtText(ctx.text, plan);
    if (sql.trim() === "") continue;
    out.push({
      id: ctx.newId(),
      sql,
      serverId: ctx.serverId,
      serverName: ctx.serverName,
      database: ctx.database,
      atMs: ctx.atMs,
      elapsedMs: s.status === "ok" ? (s.elapsedMs ?? 0) : 0,
      ok: s.status === "ok",
    });
  }
  return out;
}

/** Arama kutusu metni → `qt_history_list` sorgusu (boşluk → `null` = tümü). */
export function searchQuery(input: string): string | null {
  const q = input.trim();
  return q === "" ? null : q;
}

/** Listede gösterilen SQL en çok bu kadar karakter (ekleme tam metni kullanır). */
export const DISPLAY_SQL_CHARS = 300;

export function displaySql(sql: string): string {
  return sql.length > DISPLAY_SQL_CHARS ? sql.slice(0, DISPLAY_SQL_CHARS) + "…" : sql;
}

/**
 * SQL'i imleç konumuna (ya da `[from, to)` seçiminin yerine) ekler. Konumlar UTF-16 (CodeMirror).
 * İmleçten önceki metin satır başında bitmiyorsa araya satır sonu konur. Dönen `cursor` eklenen metnin sonudur.
 */
export function insertAt(doc: string, from: number, to: number, sql: string): { doc: string; cursor: number } {
  const a = Math.max(0, Math.min(from, doc.length));
  const b = Math.max(a, Math.min(to, doc.length));
  const before = doc.slice(0, a);
  const lead = before !== "" && !before.endsWith("\n") ? "\n" : "";
  const inserted = lead + sql;
  return { doc: before + inserted + doc.slice(b), cursor: a + inserted.length };
}
