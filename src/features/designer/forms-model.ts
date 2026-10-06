// Designer formlarının saf yardımcıları (tembel parça): alan yolu sabitleri, boş → null normalleştirme, taslak alan
// güncelleyicileri ve eklenti listesi/sürüm notu. SQL üretmez; adlar yazıldığı gibi gider (yalnızca boş → null).
import type { ExtensionDef } from "../../ipc/bindings/ExtensionDef";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { SchemaDef } from "../../ipc/bindings/SchemaDef";

/** Formun bildiği `FieldError.path` kökleri (dışındakiler genel hata şeridine düşer). */
export const SCHEMA_PATHS: ReadonlySet<string> = new Set(["name", "owner", "comment"]);
export const EXTENSION_PATHS: ReadonlySet<string> = new Set(["name", "schema", "version"]);

/**
 * İsteğe bağlı alan: boş ya da yalnızca boşluk → `null`; aksi halde metin OLDUĞU GİBİ (kırpılmaz — kullanıcının yazdığı
 * gönderilir, geçersizse arka uç alan hatası döner).
 */
export function normalizeEmpty(s: string): string | null {
  return s.trim() === "" ? null : s;
}

/** Taslak şema mı (aksi halde null; form yalnızca kendi türünü düzenler). */
export function asSchema(d: ObjectDef | null): (SchemaDef & { kind: "schema" }) | null {
  return d && d.kind === "schema" ? d : null;
}
export function asExtension(d: ObjectDef | null): (ExtensionDef & { kind: "extension" }) | null {
  return d && d.kind === "extension" ? d : null;
}

/** Şema taslağına alan güncellemesi uygular (yeni tanım döner; tür uyuşmazsa tanım değişmez). */
export function schemaEditPatch(d: ObjectDef, patch: Partial<SchemaDef>): ObjectDef {
  return d.kind === "schema" ? { ...d, ...patch } : d;
}

/** Eklenti taslağına alan güncellemesi uygular (yeni tanım döner; tür uyuşmazsa tanım değişmez). */
export function extensionEditPatch(d: ObjectDef, patch: Partial<ExtensionDef>): ObjectDef {
  return d.kind === "extension" ? { ...d, ...patch } : d;
}

/** `datalist` seçeneğinin yan metni: arka ucun `detail` metni (yoksa değerin kendisi). */
export function extensionOptionLabel(item: LookupItem): string {
  const d = item.detail?.trim();
  return d ? d : item.value;
}

/** Yazılan ada karşılık gelen kurulabilir eklenti (tam eşleşme, büyük/küçük harf duyarlı). */
export function findExtension(items: readonly LookupItem[], name: string): LookupItem | null {
  return items.find((i) => i.value === name) ?? null;
}

/**
 * Alter'da seçilen sürüm yüklenenden farklıysa `UPDATE TO` çalışır → not gösterilir. Boş sürüm (`null`) = değişmez;
 * create'te (yüklenen tanım yok) not yok.
 */
export function extensionVersionNote(original: ObjectDef | null, draft: ExtensionDef): boolean {
  if (!original || original.kind !== "extension") return false;
  return draft.version !== null && draft.version !== original.version;
}
