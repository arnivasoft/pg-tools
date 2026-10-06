// İptal/sonlandırma güvenliği (saf): onay öncesi uyarılar ve onay anında satırın hâlâ aynı oturum olduğunun doğrulanması.
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
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

/** Onay anında canlı listede aynı pid hâlâ aynı oturum mu? (pid + user + application + backendType; pid yeniden kullanımına karşı.) */
export function revalidate(shown: ActivityRow, live: readonly ActivityRow[]): Revalidate {
  const cur = live.find((r) => r.pid === shown.pid);
  if (!cur) return { ok: false, reason: "gone" };
  if (cur.user !== shown.user || cur.application !== shown.application || cur.backendType !== shown.backendType) {
    return { ok: false, reason: "changed" };
  }
  return { ok: true };
}
