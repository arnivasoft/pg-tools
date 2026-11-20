// Policy formunun saf modeli (tembel parça; Solid içe aktarmaz): komuta göre ifade alanı kuralları (Karar 2), yeniden
// kurma nedenleri (Karar 3; spec §9.3 aynası) ve rol çipi yardımcıları (Karar 5). SQL üretmez; arka uç yetkilidir.
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PolicyCommand } from "../../ipc/bindings/PolicyCommand";
import type { PolicyDef } from "../../ipc/bindings/PolicyDef";

export function asPolicy(d: ObjectDef | null): (PolicyDef & { kind: "policy" }) | null {
  return d && d.kind === "policy" ? d : null;
}

/** Taslağa alan güncellemesi uygular (yeni tanım; `kind` korunur). Tür uyuşmazlığında aynı nesne. */
export function policyPatch(d: ObjectDef, patch: Partial<PolicyDef>): ObjectDef {
  return d.kind === "policy" ? { ...d, ...patch, kind: "policy" } : d;
}

/** Karar 2: INSERT'te USING, SELECT/DELETE'te WITH CHECK kullanılmaz. */
export function expressionAvailability(command: PolicyCommand): { using: boolean; withCheck: boolean } {
  return { using: command !== "insert", withCheck: command !== "select" && command !== "delete" };
}

/** Komutu yazar ve yasak hâle gelen ifadeyi `null` yapar; girdi değişmez. */
export function policyCommandPatch(d: PolicyDef, command: PolicyCommand): PolicyDef {
  const a = expressionAvailability(command);
  return { ...d, command, using: a.using ? d.using : null, withCheck: a.withCheck ? d.withCheck : null };
}

export type RebuildReason = "command" | "permissive" | "usingRemoved" | "withCheckRemoved";

const blank = (s: string | null) => s === null || s.trim() === "";

/** Karar 3: alter'da DROP + CREATE gerektiren nedenler (spec §9.3); create (`original === null`) → []. */
export function policyRebuildReasons(original: PolicyDef | null, d: PolicyDef): RebuildReason[] {
  if (!original) return [];
  const out: RebuildReason[] = [];
  if (original.command !== d.command) out.push("command");
  if (original.permissive !== d.permissive) out.push("permissive");
  if (!blank(original.using) && blank(d.using)) out.push("usingRemoved");
  if (!blank(original.withCheck) && blank(d.withCheck)) out.push("withCheckRemoved");
  return out;
}

/** Rol ekler; boş/yalnız boşluk ve yinelenen ad yok sayılır (aynı nesne döner). */
export function addRole(d: PolicyDef, name: string): PolicyDef {
  if (name.trim() === "" || d.roles.includes(name)) return d;
  return { ...d, roles: [...d.roles, name] };
}

/** İndeksteki rolü kaldırır; indeks dışı → aynı nesne. */
export function removeRole(d: PolicyDef, i: number): PolicyDef {
  if (!Number.isInteger(i) || i < 0 || i >= d.roles.length) return d;
  return { ...d, roles: d.roles.filter((_, k) => k !== i) };
}

/** Seçilebilir roller: lookup − mevcut, `public` hariç, ada göre sıralı. */
export function roleChoices(lookup: readonly string[], current: readonly string[]): string[] {
  return lookup.filter((r) => r !== "public" && !current.includes(r)).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}
