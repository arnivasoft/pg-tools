// Dashboard tablo modeli (saf): oturum süzgeci/sıralaması ve ayar süzgeci. Sıralama kararlı; null her zaman sonda.
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { Setting } from "../../ipc/bindings/Setting";

export interface ActivityFilter {
  search: string;
  /** Boş küme = durum süzgeci yok. */
  states: Set<string>;
  onlyActive: boolean;
  /** Dolu ise yalnızca `idle in transaction*` olup `stateAgeSecs >= eşik` olanlar. */
  idleInTxOverSecs: number | null;
  database: string | null;
}

/** Arama pid, sorgu metni, kullanıcı, uygulama ve veritabanında (büyük/küçük harf duyarsız); süzgeçler AND. */
export function filterActivity(rows: readonly ActivityRow[], f: ActivityFilter): ActivityRow[] {
  const q = f.search.trim().toLowerCase();
  return rows.filter((r) => {
    if (f.onlyActive && r.state !== "active") return false;
    if (f.states.size > 0 && (r.state == null || !f.states.has(r.state))) return false;
    if (f.database != null && r.database !== f.database) return false;
    if (f.idleInTxOverSecs != null) {
      if (!r.state?.startsWith("idle in transaction")) return false;
      if (r.stateAgeSecs == null || r.stateAgeSecs < f.idleInTxOverSecs) return false;
    }
    if (q) {
      const hay = [String(r.pid), r.query, r.user, r.application, r.database];
      if (!hay.some((h) => h != null && h.toLowerCase().includes(q))) return false;
    }
    return true;
  });
}

export type ActivityKey =
  | "pid" | "database" | "user" | "application" | "client" | "state" | "wait" | "xactAge" | "queryAge" | "stateAge";
export type SortDir = "asc" | "desc";

const collator = new Intl.Collator("tr", { sensitivity: "base", numeric: true });

function keyOf(r: ActivityRow, key: ActivityKey): string | number | null {
  switch (key) {
    case "pid": return r.pid;
    case "database": return r.database;
    case "user": return r.user;
    case "application": return r.application;
    case "client": return r.clientAddr;
    case "state": return r.state;
    case "wait": return r.waitEventType == null && r.waitEvent == null ? null : `${r.waitEventType ?? ""}:${r.waitEvent ?? ""}`;
    case "xactAge": return r.xactAgeSecs;
    case "queryAge": return r.queryAgeSecs;
    case "stateAge": return r.stateAgeSecs;
  }
}

/** Yeni dizi döner. `null` değerler yönden bağımsız sonda; eşitlikte pid artan (yön çevrilince de). */
export function sortActivity(rows: readonly ActivityRow[], key: ActivityKey, dir: SortDir): ActivityRow[] {
  const sign = dir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const x = keyOf(a, key);
    const y = keyOf(b, key);
    if (x == null || y == null) {
      if (x != null) return -1;
      if (y != null) return 1;
      return a.pid - b.pid;
    }
    const c = typeof x === "number" && typeof y === "number" ? x - y : collator.compare(String(x), String(y));
    return c !== 0 ? sign * c : a.pid - b.pid;
  });
}

export interface SettingsFilter { search: string; modifiedOnly: boolean; pendingRestartOnly: boolean }

/** Arama ad, kategori ve açıklamada; `modifiedOnly` = `source != 'default'`. */
export function filterSettings(rows: readonly Setting[], f: SettingsFilter): Setting[] {
  const q = f.search.trim().toLowerCase();
  return rows.filter((r) => {
    if (f.modifiedOnly && r.source === "default") return false;
    if (f.pendingRestartOnly && !r.pendingRestart) return false;
    if (q && !`${r.name}\n${r.category}\n${r.description}`.toLowerCase().includes(q)) return false;
    return true;
  });
}
