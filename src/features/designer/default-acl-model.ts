// "Varsayılan yetkiler…" formunun saf modeli (Plan 22 Task 5; tembel parça, Solid içe aktarmaz).
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import type { DefaultAclDef } from "../../ipc/bindings/DefaultAclDef";
import type { DefaultAclObjectType } from "../../ipc/bindings/DefaultAclObjectType";
import type { Privilege } from "../../ipc/bindings/Privilege";

/**
 * Rol seçicide değişim kararı: aynı/boş rol ya da meşgul sekme (yükleme/uygulama) → yok say; kirli taslak → önce
 * "değişiklikleri at" onayı; aksi hâlde doğrudan `setAclRole`.
 */
export function roleSwitchDecision(input: { current: string; next: string; dirty: boolean; busy: boolean }): "ignore" | "confirm" | "switch" {
  const next = input.next.trim();
  if (next === "" || next === input.current || input.busy) return "ignore";
  return input.dirty ? "confirm" : "switch";
}

export interface DefaultAclSectionView {
  objectType: DefaultAclObjectType;
  /** Hata yolu kökü (`sections[i].grants`). */
  basePath: string;
  grants: AclGrant[];
  available: Privilege[];
  /** Bölüm ızgarasının sahibi kapsamın rolüdür (GRANT OPTION adımı yok). */
  owner: string;
  readOnly: boolean;
  /** Yalnız şema kapsamında: genel kapsamın etkin değeri, her zaman salt okunur. */
  baseline: { grants: AclGrant[]; readOnly: true; basePath: string } | null;
}

/** Bölüm görünümleri (arka uç sırasıyla). Salt okunur tanım (`editable = false`) ya da form → tüm ızgaralar salt okunur. */
export function defaultAclSections(def: DefaultAclDef, readOnly: boolean): DefaultAclSectionView[] {
  const ro = readOnly || !def.editable;
  return def.sections.map((s, i) => ({
    objectType: s.objectType,
    basePath: `sections[${i}].grants`,
    grants: s.grants,
    available: s.available,
    owner: def.role,
    readOnly: ro,
    baseline: def.schema === null ? null : { grants: s.baseline, readOnly: true, basePath: `sections[${i}].baseline` },
  }));
}
