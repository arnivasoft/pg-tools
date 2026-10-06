// İptal/sonlandırma güvenliği (saf): onay öncesi uyarılar ve onay anında satırın hâlâ aynı oturum olduğunun doğrulanması.
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { BlockSession } from "../../ipc/bindings/BlockSession";
import type { SignalKind } from "../../ipc/bindings/SignalKind";

export interface SignalWarning { kind: "self" | "pgtoolsQuery" | "idleInTx" | "terminate"; }

/** pg-tools'un kendi Query Tool bağlantılarının `application_name`'i. */
const PGTOOLS_QUERY_APP = "pg-tools query";

/** Sabit sırada: self, pgtoolsQuery, idleInTx, terminate. */
export function signalWarnings(row: ActivityRow, kind: SignalKind): SignalWarning[] {
  const w: SignalWarning[] = [];
  if (row.isSelf) w.push({ kind: "self" });
  if (row.application === PGTOOLS_QUERY_APP) w.push({ kind: "pgtoolsQuery" });
  if (row.state?.startsWith("idle in transaction")) w.push({ kind: "idleInTx" });
  if (kind === "terminate") w.push({ kind: "terminate" });
  return w;
}

export type Revalidate = { ok: true } | { ok: false; reason: "gone" | "changed" };

/** `BlockSession` backendType bilmez (autovacuum worker gibi engelleyiciler olabilir): boş değer "bilinmiyor" demektir. */
export const UNKNOWN_BACKEND_TYPE = "";

/**
 * Etkinlik listesinde satırı olmayan kilit düğümü için asgari satır. `backendType` bilinmediğinden `UNKNOWN_BACKEND_TYPE`
 * bırakılır (`revalidate` o alanı karşılaştırmaz; pid + user + application yine doğrulanır). `isSelf` arka uç kilit
 * sonucundan zaten dışlandığı için `false`.
 */
export function minimalRow(s: BlockSession): ActivityRow {
  return {
    pid: s.pid, leaderPid: null, database: null, user: s.user, application: s.application, clientAddr: null,
    backendType: UNKNOWN_BACKEND_TYPE, state: s.state, waitEventType: null, waitEvent: null, backendAgeSecs: null,
    xactAgeSecs: null, queryAgeSecs: null, stateAgeSecs: s.waitSecs, query: s.query, isSelf: false,
  };
}

/**
 * Kilit düğümünün sinyal satırı. Etkinlik listesi yalnızca Oturumlar bölümü açıkken yenilenir (bayat olabilir): gerçek satır
 * yalnızca kullanıcı + uygulama eşleşirse kullanılır ve taze alanlar (durum, sorgu) `BlockSession`'dan üstüne yazılır;
 * aksi halde asgari satır.
 */
export function resolveSignalRow(s: BlockSession, activity: readonly ActivityRow[] | null | undefined): ActivityRow {
  const real = activity?.find((r) => r.pid === s.pid);
  if (real && real.user === s.user && real.application === s.application) return { ...real, state: s.state, query: s.query };
  return minimalRow(s);
}

/** Onay anında canlı listede aynı pid hâlâ aynı oturum mu? (pid + user + application + backendType; pid yeniden kullanımına karşı.) */
export function revalidate(shown: ActivityRow, live: readonly ActivityRow[]): Revalidate {
  const cur = live.find((r) => r.pid === shown.pid);
  if (!cur) return { ok: false, reason: "gone" };
  if (
    cur.user !== shown.user ||
    cur.application !== shown.application ||
    (shown.backendType !== UNKNOWN_BACKEND_TYPE && cur.backendType !== shown.backendType)
  ) {
    return { ok: false, reason: "changed" };
  }
  return { ok: true };
}
