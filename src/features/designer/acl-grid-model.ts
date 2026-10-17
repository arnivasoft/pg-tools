// Yetki ızgarasının saf modeli (tembel parça; Solid içe aktarmaz). Her işlem yeni dizi döndürür, girdi değiştirilmez.
// `privileges` dizisi kanonik (`available`) sırada tutulur: arka uç hata yolu `grants[i].privileges[j]` bu sıraya göre
// hücreye eşlenir. `available` dışı (bilinmeyen) yetkiler korunur ve kanonik olanların ardına, özgün sırayla eklenir.
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import type { PlanView } from "../../ipc/bindings/PlanView";
import type { Privilege } from "../../ipc/bindings/Privilege";

/** Hücre durumu: yok / var / var + GRANT OPTION. */
export type CellState = "none" | "granted" | "grantable";

export type FocusKey = "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Home" | "End";

export function cellState(g: AclGrant, p: Privilege): CellState {
  const f = g.privileges.find((x) => x.privilege === p);
  if (!f) return "none";
  return f.grantOption ? "grantable" : "granted";
}

/** Döngü: yok → var → (devredilebilir) → yok. GRANT OPTION izinli değilse "var" adımından sonra yok'a döner. */
export function nextCellState(s: CellState, allowGrantOption: boolean): CellState {
  if (s === "none") return "granted";
  if (s === "granted") return allowGrantOption ? "grantable" : "none";
  return "none";
}

/** `row` satırındaki `p` hücresini `s` yapar; yetkiler kanonik sıradadır. Satır boşalsa da kalır. Geçersiz satır → kopya. */
export function setCell(grants: AclGrant[], row: number, p: Privilege, s: CellState, available: Privilege[]): AclGrant[] {
  const out = grants.map((g) => ({ ...g, privileges: g.privileges.map((x) => ({ ...x })) }));
  const g = out[row];
  if (!g) return out;
  const rest = g.privileges.filter((x) => x.privilege !== p);
  const next = s === "none" ? rest : [...rest, { privilege: p, grantOption: s === "grantable" }];
  const rank = (x: Privilege) => available.indexOf(x);
  const known = next.filter((x) => rank(x.privilege) >= 0).sort((a, b) => rank(a.privilege) - rank(b.privilege));
  const unknown = next.filter((x) => rank(x.privilege) < 0);
  g.privileges = [...known, ...unknown];
  return out;
}

/** Alıcıyı sona ekler (boş yetkiyle); yinelenen (PUBLIC dahil) ya da boş ad → null. */
export function addGrantee(grants: AclGrant[], grantee: string | null): AclGrant[] | null {
  const name = grantee === null ? null : grantee.trim();
  if (name === "") return null;
  if (grants.some((g) => g.grantee === name)) return null;
  return [...grants.map((g) => ({ ...g })), { grantee: name, privileges: [] }];
}

/**
 * Form "Alıcı ekle" girdisi: kırpılır; boş → `empty`; `public` (harf duyarsız; datalist'teki "PUBLIC" seçeneği) →
 * PUBLIC satırı (`grantee = null`); yinelenen → `duplicate`. Not: `"PUBLIC"`/`"Public"` adlı gerçek roller bu alandan
 * eklenemez (PUBLIC sayılır; spec E2'deki kenar durum, yüklü satırlar etkilenmez).
 */
export function addGranteeInput(grants: AclGrant[], input: string): { grants: AclGrant[] } | { error: "empty" | "duplicate" } {
  const name = input.trim();
  if (name === "") return { error: "empty" };
  const next = addGrantee(grants, name.toLowerCase() === "public" ? null : name);
  return next ? { grants: next } : { error: "duplicate" };
}

/** Planda REVOKE var mı (CASCADE onay kutusu yalnız o zaman anlamlıdır; arka uç REVOKE yokken `cascade`'i yok sayar). */
export function planHasRevoke(view: PlanView | null): boolean {
  return !!view && view.statements.some((s) => /^REVOKE\s/i.test(s.sql));
}

export function removeGrantee(grants: AclGrant[], row: number): AclGrant[] {
  if (!Number.isInteger(row) || row < 0 || row >= grants.length) return grants.map((g) => ({ ...g }));
  return [...grants.slice(0, row), ...grants.slice(row + 1)].map((g) => ({ ...g }));
}

/** PUBLIC ve sahip satırında GRANT OPTION adımı yoktur. */
export function allowGrantOption(g: AclGrant, owner: string): boolean {
  return g.grantee !== null && g.grantee !== owner;
}

/** Satırda olup `available` dışında kalan yetkiler (salt okunur gösterim). */
export function unknownPrivileges(g: AclGrant, available: Privilege[]): Privilege[] {
  return g.privileges.map((x) => x.privilege).filter((p) => !available.includes(p));
}

/** Hücrenin hata yolu: `<base>[row].privileges[j]`; hücrede yetki yoksa (yok durumu) null. */
export function cellPath(base: string, row: number, grants: AclGrant[], p: Privilege): string | null {
  const j = grants[row]?.privileges.findIndex((x) => x.privilege === p) ?? -1;
  return j < 0 ? null : `${base}[${row}].privileges[${j}]`;
}

/** Gezgin tabindex: bir sonraki hücre (kenarlarda sıkıştırılır, sarmaz). Home/End satır içinde. */
export function nextCellFocus(row: number, col: number, rows: number, cols: number, key: FocusKey): [number, number] {
  const clamp = (v: number, n: number) => Math.max(0, Math.min(v, Math.max(n - 1, 0)));
  switch (key) {
    case "ArrowUp": return [clamp(row - 1, rows), clamp(col, cols)];
    case "ArrowDown": return [clamp(row + 1, rows), clamp(col, cols)];
    case "ArrowLeft": return [clamp(row, rows), clamp(col - 1, cols)];
    case "ArrowRight": return [clamp(row, rows), clamp(col + 1, cols)];
    case "Home": return [clamp(row, rows), 0];
    case "End": return [clamp(row, rows), clamp(cols - 1, cols)];
  }
}
