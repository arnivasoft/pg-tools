// Trigger formunun saf modeli (tembel parça; Solid içe aktarmaz): daraltma/güncelleme, seçeneklerin açıklığı (Karar 6:
// yalnız açıkça geçersiz birleşimler kapatılır), kapanan seçeneklerin değerlerinin temizlenmesi, fonksiyon seçimi (R1:
// `value` = ad, `detail` = şema) ve yeniden kurma notu (spec §8.7). SQL üretmez; diğer kurallar arka ucun alan hatasıdır.
import { ttrg } from "../../app/strings-trigger";
import type { LookupItem } from "../../ipc/bindings/LookupItem";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TriggerDef } from "../../ipc/bindings/TriggerDef";
import type { TriggerEvents } from "../../ipc/bindings/TriggerEvents";

export type TriggerDraft = TriggerDef & { kind: "trigger" };

export function asTrigger(d: ObjectDef | null): TriggerDraft | null {
  return d && d.kind === "trigger" ? d : null;
}

/** Taslağa alan güncellemesi uygular (yeni tanım; `kind` korunur). Tür uyuşmazlığında aynı nesne. */
export function triggerPatch(d: ObjectDef, patch: Partial<TriggerDef>): ObjectDef {
  return d.kind === "trigger" ? { ...d, ...patch, kind: "trigger" } : d;
}

export interface TriggerAvailability {
  /** FOR EACH ROW (TRUNCATE → yalnız STATEMENT). */
  row: boolean;
  /** FOR EACH STATEMENT (constraint → yalnız ROW). */
  statement: boolean;
  /** REFERENCING: AFTER, constraint değil, UPDATE OF yok, TRUNCATE yok, tek olay (INSERT/UPDATE/DELETE). */
  referencing: boolean;
  /** OLD TABLE: REFERENCING açık ve UPDATE ya da DELETE. */
  oldTable: boolean;
  /** NEW TABLE: REFERENCING açık ve INSERT ya da UPDATE. */
  newTable: boolean;
  /** DEFERRABLE / INITIALLY DEFERRED yalnız constraint trigger'da. */
  deferrable: boolean;
  /** UPDATE OF kolonları yalnız UPDATE olayıyla. */
  updateColumns: boolean;
  /** BEFORE (constraint → yalnız AFTER). */
  before: boolean;
  /** TRUNCATE olayı (constraint trigger satır düzeyidir). */
  truncate: boolean;
  /** Constraint trigger kutusu (TRUNCATE seçiliyken kapalı). */
  constraint: boolean;
}

const dmlCount = (e: TriggerEvents) => [e.insert, e.update, e.delete].filter(Boolean).length;

export function availability(d: TriggerDef): TriggerAvailability {
  const e = d.events;
  const referencing =
    d.timing === "after" && !d.constraint && e.updateColumns.length === 0 && !e.truncate && dmlCount(e) === 1;
  return {
    row: !e.truncate,
    statement: !d.constraint,
    referencing,
    oldTable: referencing && (e.update || e.delete),
    newTable: referencing && (e.insert || e.update),
    deferrable: d.constraint,
    updateColumns: e.update,
    before: !d.constraint,
    truncate: !d.constraint,
    constraint: !e.truncate,
  };
}

/**
 * Kapanan seçeneğin değerini temizler (her düzenlemeden sonra uygulanır; geçerli tanımda içerik değişmez). Sıra:
 * constraint (AFTER + ROW, TRUNCATE ve geçiş tabloları kapanır) → UPDATE yoksa `updateColumns` boş → TRUNCATE ise
 * STATEMENT → constraint değilse DEFERRABLE/INITIALLY DEFERRED false (DEFERRABLE yoksa INITIALLY DEFERRED false) →
 * REFERENCING kapalıysa (ya da olaya uymuyorsa) OLD/NEW TABLE `null`.
 */
export function normalizeTrigger<T extends TriggerDef>(d: T): T {
  const out: T = { ...d, events: { ...d.events } };
  if (out.constraint) {
    out.timing = "after";
    out.level = "row";
    out.events.truncate = false;
  }
  if (!out.events.update && out.events.updateColumns.length > 0) out.events.updateColumns = [];
  if (out.events.truncate) out.level = "statement";
  if (!out.constraint) {
    out.deferrable = false;
    out.initiallyDeferred = false;
  }
  if (!out.deferrable) out.initiallyDeferred = false;
  const a = availability(out);
  if (!a.oldTable) out.oldTable = null;
  if (!a.newTable) out.newTable = null;
  return out;
}

/** `normalizeTrigger`'ın kapatarak temizlediği, alan yeniden açılınca geri yüklenecek değerler (form örneği başına). */
export interface TriggerStash {
  updateColumns?: string[];
  oldTable?: string;
  newTable?: string;
}

/**
 * Kapanıp açılan alanın değerini korur (saf; girişler değişmez). `next` normalize edilmiş tanımdır. `prev`'de dolu olup
 * `next`'te kapandığı için boşalan UPDATE OF / OLD TABLE / NEW TABLE saklanır; `next`'te açık ve boş olan alanın saklı
 * değeri geri yüklenir ve kaydı silinir. Önce UPDATE OF geri gelir (REFERENCING açıklığını etkiler), sonra geçiş tabloları.
 * Açıkken elle boşaltılan alan saklanmaz.
 */
export function restoreCleared<T extends TriggerDef>(prev: TriggerDef, next: T, stash: TriggerStash): { def: T; stash: TriggerStash } {
  const s: TriggerStash = { ...stash };
  const def: T = { ...next, events: { ...next.events } };
  const a0 = availability(def);
  if (prev.events.updateColumns.length > 0 && def.events.updateColumns.length === 0 && !a0.updateColumns) {
    s.updateColumns = [...prev.events.updateColumns];
  }
  if (prev.oldTable !== null && def.oldTable === null && !a0.oldTable) s.oldTable = prev.oldTable;
  if (prev.newTable !== null && def.newTable === null && !a0.newTable) s.newTable = prev.newTable;

  if (a0.updateColumns && def.events.updateColumns.length === 0 && s.updateColumns) {
    def.events.updateColumns = s.updateColumns;
    delete s.updateColumns;
  }
  const a1 = availability(def);
  // Geri gelen UPDATE OF REFERENCING'i kapatırsa dolu geçiş tablosu da beklemeye alınır (tanım normalize kalır).
  if (!a1.oldTable && def.oldTable !== null) {
    s.oldTable = def.oldTable;
    def.oldTable = null;
  }
  if (!a1.newTable && def.newTable !== null) {
    s.newTable = def.newTable;
    def.newTable = null;
  }
  if (a1.oldTable && def.oldTable === null && s.oldTable !== undefined) {
    def.oldTable = s.oldTable;
    delete s.oldTable;
  }
  if (a1.newTable && def.newTable === null && s.newTable !== undefined) {
    def.newTable = s.newTable;
    delete s.newTable;
  }
  return { def, stash: s };
}

/** Lookup öğesinden fonksiyon alanları (R1: `value` = ad, `detail` = şema; şema yoksa yedek). */
export function functionFromLookup(item: LookupItem, fallbackSchema = ""): { functionSchema: string; functionName: string } {
  return { functionSchema: item.detail ?? fallbackSchema, functionName: item.value };
}

/** Taslaktaki şema + ad listede hangi öğe (seçim kutusunun seçili seçeneği); yoksa -1. */
export function functionOptionIndex(items: readonly LookupItem[], d: Pick<TriggerDef, "functionSchema" | "functionName">): number {
  return items.findIndex((i) => i.value === d.functionName && i.detail === d.functionSchema);
}

const sameList = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/** Arka ucun `structure_changed` aynası: `CREATE` ifadesine yazılan alanlar (ad/tablo/etkinlik/yorum hariç). */
export function structureChanged(o: TriggerDef, n: TriggerDef): boolean {
  const a = o.events;
  const b = n.events;
  return (
    o.timing !== n.timing ||
    a.insert !== b.insert ||
    a.update !== b.update ||
    a.delete !== b.delete ||
    a.truncate !== b.truncate ||
    !sameList(a.updateColumns, b.updateColumns) ||
    o.level !== n.level ||
    o.when !== n.when ||
    o.functionSchema !== n.functionSchema ||
    o.functionName !== n.functionName ||
    !sameList(o.args, n.args) ||
    o.oldTable !== n.oldTable ||
    o.newTable !== n.newTable ||
    o.constraint !== n.constraint ||
    o.deferrable !== n.deferrable ||
    o.initiallyDeferred !== n.initiallyDeferred
  );
}

/**
 * Alter'da yapı değiştiyse yeniden kurma notu (spec §8.7): PG 14+ `CREATE OR REPLACE TRIGGER` (Güvenli; yorum korunur,
 * etkinlik ORIGIN'e döner ve arka uç gerekirse yeniden yazar), daha eski sunucuda ya da constraint trigger'da (her iki
 * taraf) `DROP + CREATE` (Yıkıcı). `versionHint` (`server_version_num`) arayüzde bilinmez (`null`): iki durum da anlatılır.
 */
export function rebuildNote(old: TriggerDef | null, d: TriggerDef, versionHint: number | null = null): string | null {
  if (!old || !structureChanged(old, d)) return null;
  if (old.constraint || d.constraint) return ttrg.rebuild.constraint;
  if (versionHint === null) return ttrg.rebuild.both;
  return versionHint >= 140000 ? ttrg.rebuild.replace : ttrg.rebuild.dropCreate;
}

/** Listede bir öğeyi değiştirir (değişmez; sınır dışı → aynı dizi). */
export function setAt(list: string[], i: number, v: string): string[] {
  if (!Number.isInteger(i) || i < 0 || i >= list.length) return list;
  return list.map((x, k) => (k === i ? v : x));
}

/**
 * UPDATE OF kolon çipi aç/kapa: seçilen kolon sona eklenir, kaldırılan çıkarılır (yüklü sıra korunur — sıra değişimi
 * yeniden kurma sayılır).
 */
export function toggleUpdateColumn(list: readonly string[], col: string, on: boolean): string[] {
  const has = list.includes(col);
  if (on) return has ? [...list] : [...list, col];
  return list.filter((c) => c !== col);
}

/** Çip seçenekleri: tablonun kolonları + listede olup tabloda bulunmayan (ör. yazım/yüklü) seçili kolonlar başta. */
export function updateColumnChoices(tableColumns: readonly string[], selected: readonly string[]): string[] {
  const extra = selected.filter((c) => !tableColumns.includes(c));
  return [...extra, ...tableColumns];
}
