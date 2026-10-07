// Ortak ayar düzenleyicisinin saf modeli (tembel parça): satır işlemleri, kaldırılanlar/geri al ve istemci ön uyarıları.
// Uyarılar hata değildir; asıl doğrulama arka uçtadır (Plan 14). SQL üretmez.
import { tset } from "../../app/strings-settings";
import type { ConfigSetting } from "../../ipc/bindings/ConfigSetting";

/** Arka uçla aynı küme (pg_dump `variable_is_guc_list_quote`). */
const LIST_GUCS: ReadonlySet<string> = new Set([
  "local_preload_libraries", "search_path", "session_preload_libraries", "shared_preload_libraries", "temp_tablespaces",
  "unix_socket_directories",
]);

export function addSetting(list: readonly ConfigSetting[]): ConfigSetting[] {
  return [...list, { name: "", value: "" }];
}

export function updateSetting(list: readonly ConfigSetting[], i: number, patch: Partial<ConfigSetting>): ConfigSetting[] {
  if (!Number.isInteger(i) || i < 0 || i >= list.length) return list as ConfigSetting[];
  return list.map((s, k) => (k === i ? { ...s, ...patch } : s));
}

export function removeSetting(list: readonly ConfigSetting[], i: number): ConfigSetting[] {
  if (!Number.isInteger(i) || i < 0 || i >= list.length) return list as ConfigSetting[];
  return list.filter((_, k) => k !== i);
}

/** Taslakta olmayan yüklü ayarlar (ad ile, özgün sırayla). */
export function removedSettings(original: readonly ConfigSetting[] | null, list: readonly ConfigSetting[]): ConfigSetting[] {
  if (!original) return [];
  const names = new Set(list.map((s) => s.name));
  return original.filter((s) => !names.has(s.name));
}

/** Kaldırılan ayarı özgün değeriyle, özgün sıradaki yerine (önceki özgün komşusunun ardına) geri koyar. */
export function restoreSetting(list: readonly ConfigSetting[], original: readonly ConfigSetting[], name: string): ConfigSetting[] {
  const idx = original.findIndex((s) => s.name === name);
  if (idx < 0 || list.some((s) => s.name === name)) return list as ConfigSetting[];
  let at = 0;
  for (let k = idx - 1; k >= 0; k--) {
    const pos = list.findIndex((s) => s.name === original[k]!.name);
    if (pos >= 0) {
      at = pos + 1;
      break;
    }
  }
  const next = [...list];
  next.splice(at, 0, { ...original[idx]! });
  return next;
}

const NAME_PART = /^[a-z_][a-z0-9_$]*$/;
const NAME_PART_DOTTED = /^[A-Za-z_][A-Za-z0-9_$]*$/;

/** İstemci ön uyarısı (hata değil): boş ad, büyük harf, izin verilmeyen karakter, yinelenen ad (ASCII harfe duyarsız). */
export function settingNameWarning(list: readonly ConfigSetting[], i: number): string | null {
  const s = list[i];
  if (!s) return null;
  if (s.name.trim() === "") return tset.warn.empty;
  const dotted = s.name.includes(".");
  const parts = s.name.split(".");
  if (!dotted && !NAME_PART.test(s.name) && NAME_PART.test(s.name.toLowerCase())) return tset.warn.upper;
  if (!parts.every((p) => (dotted ? NAME_PART_DOTTED : NAME_PART).test(p))) return tset.warn.chars;
  const key = s.name.toLowerCase();
  if (list.some((o, j) => j < i && o.name.toLowerCase() === key)) return tset.warn.duplicate;
  return null;
}

export function isListSetting(name: string): boolean {
  return LIST_GUCS.has(name);
}

/** Liste ayarında tırnaksız büyük harfli öğe → sunucuda tırnaklı (büyük/küçük harf korunur) anlamına gelir. */
export function listItemCaseWarning(name: string, value: string): string | null {
  if (!isListSetting(name)) return null;
  for (const raw of value.split(",")) {
    const item = raw.trim();
    if (item !== "" && !item.startsWith('"') && /[A-Z]/.test(item)) return tset.warn.listCase(item);
  }
  return null;
}
