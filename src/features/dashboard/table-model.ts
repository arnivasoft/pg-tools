// Dashboard tablo modeli (saf): oturum süzgeci/sıralaması ve ayar süzgeci. Sıralama kararlı; null her zaman sonda.
import type { ActivityRow } from "../../ipc/bindings/ActivityRow";
import type { Setting } from "../../ipc/bindings/Setting";
import { formatDuration } from "./dash-model";

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

// ---- Görünüm yardımcıları (saf)

export type StateTone = "active" | "idle" | "warn" | "danger" | "muted";

/** Durum rozeti tonu: etkin → vurgu, boşta → soluk, boşta-tx → uyarı, iptal edilmiş tx/devre dışı → tehlike. */
export function stateTone(state: string | null): StateTone {
  if (state == null) return "muted";
  if (state === "active") return "active";
  if (state === "idle") return "idle";
  if (state.startsWith("idle in transaction")) return state.includes("aborted") ? "danger" : "warn";
  if (state === "disabled") return "danger";
  return "muted"; // fastpath function call, vb.
}

export type DurationTone = "none" | "normal" | "warn" | "danger";
export const DURATION_WARN_SECS = 30;
export const DURATION_DANGER_SECS = 300;

/** Süre kovası: null → none; < 30 sn normal; < 5 dk uyarı; üstü tehlike. */
export function durationTone(secs: number | null): DurationTone {
  if (secs == null || !Number.isFinite(secs) || secs < 0) return "none";
  if (secs < DURATION_WARN_SECS) return "normal";
  return secs < DURATION_DANGER_SECS ? "warn" : "danger";
}

/** Boşluklar tek boşluğa; en çok `max` karakter (taşarsa "…"). null → "". */
export function oneLine(q: string | null, max = 240): string {
  if (q == null) return "";
  const s = q.replace(/\s+/g, " ").trim();
  return s.length > max ? `${s.slice(0, max)}…` : s;
}

/** Sorgunun ilk `n` satırı (boş satırlar atlanır); kesildiyse `truncated`. */
export function firstLines(q: string | null, n: number): { text: string; truncated: boolean } {
  if (q == null) return { text: "", truncated: false };
  const lines = q.split(/\r?\n/).filter((l) => l.trim() !== "");
  return { text: lines.slice(0, n).join("\n"), truncated: lines.length > n };
}

export type NavKey = "ArrowUp" | "ArrowDown" | "Home" | "End" | "PageUp" | "PageDown";
const NAV = new Set<string>(["ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"]);
export const isNavKey = (k: string): k is NavKey => NAV.has(k);

/** Klavyeyle satır seçimi: yeni dizin. `cur` −1 = seçim yok. Sınırda durur; boş listede −1. */
export function navIndex(cur: number, key: NavKey, count: number, page = 10): number {
  if (count <= 0) return -1;
  const last = count - 1;
  const from = cur < 0 || cur > last ? -1 : cur;
  const step = Math.max(1, page);
  switch (key) {
    case "ArrowDown": return from < 0 ? 0 : Math.min(last, from + 1);
    case "ArrowUp": return from < 0 ? last : Math.max(0, from - 1);
    case "PageDown": return from < 0 ? 0 : Math.min(last, from + step);
    case "PageUp": return from < 0 ? last : Math.max(0, from - step);
    case "Home": return 0;
    case "End": return last;
  }
}

export interface SortState<K extends string = string> { key: K; dir: SortDir }

/** Başlığa tıklama: aynı sütun → yön çevrilir; yeni sütun → artan. */
export function nextSort<K extends string>(cur: SortState<K> | null, key: K): SortState<K> {
  return cur && cur.key === key ? { key, dir: cur.dir === "asc" ? "desc" : "asc" } : { key, dir: "asc" };
}

/** `aria-sort` değeri. */
export function ariaSort(cur: SortState | null, key: string): "ascending" | "descending" | "none" {
  return cur && cur.key === key ? (cur.dir === "asc" ? "ascending" : "descending") : "none";
}

/** Bekleme: "Lock:relation"; yalnızca biri varsa o; ikisi de yoksa "". */
export function waitLabel(r: Pick<ActivityRow, "waitEventType" | "waitEvent">): string {
  return [r.waitEventType, r.waitEvent].filter((x) => x != null && x !== "").join(":");
}

/** Hazır transaction bu süreden (sn) uzun açıksa "uzun süredir açık" rozeti gösterilir. */
export const PREPARED_OLD_SECS = 3600;

/** Hazır transaction yaşı: ≥ 1 saat → `"old"`; aksi halde (ya da geçersiz yaş) `null`. */
export function preparedAgeBadge(ageSecs: number): "old" | null {
  return Number.isFinite(ageSecs) && ageSecs >= PREPARED_OLD_SECS ? "old" : null;
}

/** Arabellek isabet oranı (0–1): `hit / (hit + read)`; hiç erişim yoksa ya da değer geçersizse `null`. */
export function hitRatio(hit: number, read: number): number | null {
  if (!Number.isFinite(hit) || !Number.isFinite(read) || hit < 0 || read < 0) return null;
  const total = hit + read;
  return total > 0 ? hit / total : null;
}

/** Oran (0–1) → "%97,3"; `null` → "—". */
export function formatRatio(r: number | null): string {
  return r == null || !Number.isFinite(r) ? "—" : `%${(Math.round(r * 1000) / 10).toFixed(1).replace(".", ",")}`;
}

/** Milisaniye: < 1 sn → "0,42 ms" / "12,3 ms" / "850 ms"; ≥ 1 sn → `formatDuration` ("12 sn", "3 dk 05 sn"). Geçersiz → "—". */
export function formatMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "—";
  if (ms >= 1000) return formatDuration(ms / 1000);
  const digits = ms < 10 ? 2 : ms < 100 ? 1 : 0;
  const text = String(Math.round(ms * 10 ** digits) / 10 ** digits).replace(".", ",");
  return `${text} ms`;
}

/** Ayar değeri + birim ("128 MB"); `value == null` (yetkisiz) → "—". */
export function formatSettingValue(s: Pick<Setting, "value" | "unit">): string {
  if (s.value == null) return "—";
  return s.unit ? `${s.value} ${s.unit}` : s.value;
}
