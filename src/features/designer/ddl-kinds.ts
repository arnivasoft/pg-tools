import type { NodeKind } from "../../ipc/bindings/NodeKind";

/**
 * Nesne diyaloğu (designer) menü eylemlerinin göründüğü düğüm türleri. Yalnızca menü görünürlüğü içindir: hedef yine
 * `ddl_target_of` ile alınır ve arka uç desteklemediği düğümü reddeder. Gezgin bunu statik içe aktarır; bu dosya bilerek
 * bağımlılıksızdır (designer parçasının geri kalanını ana pakete çekmez).
 */
export const DDL_OBJECT_KINDS: NodeKind[] = [
  "schema", "sequence", "extension", "index", "table", "view", "materializedView", "database", "role",
  "function", "procedure", "triggerFunction", "type", "domain", "trigger",
];
/** …Collection: yukarıdaki her tür (ör. functionCollection, triggerFunctionCollection, triggerCollection). */
export const DDL_CREATE_COLLECTIONS = DDL_OBJECT_KINDS.map((k) => `${k}Collection` as NodeKind);

/** "Oluştur…" bu koleksiyonda gösterilir mi? */
export const canCreateIn = (kind: NodeKind) => DDL_CREATE_COLLECTIONS.includes(kind);
/** "Düzenle… / Yeniden adlandır… / Sil…" bu nesnede gösterilir mi? */
export const canEditObject = (kind: NodeKind) => DDL_OBJECT_KINDS.includes(kind);
/**
 * Tablo satırları: yalnızca "Düzenle…" (tablo diyaloğu ilgili kolona/kısıta odaklanır). Silme ve yeniden adlandırma
 * tablo diyaloğunun içinden yapılır (açıcı `focus` doluyken drop/rename'i ayrıca reddeder).
 */
export const DDL_ROW_KINDS: NodeKind[] = ["column", "constraint"];
export const isTableRowKind = (kind: NodeKind) => DDL_ROW_KINDS.includes(kind);
/**
 * Plan 22 (spec §8): "Yetkiler…" menüsünün göründüğü (ACL destekleyen) nesne düğümleri. Kolon/kısıt (tablo odak
 * hedefleri), indeks, trigger, eklenti ve rol yoktur. Salt okunur kararını (aggregate, sistem şeması…) arka uç verir.
 */
export const ACL_KINDS: NodeKind[] = [
  "table", "view", "materializedView", "sequence", "function", "procedure", "triggerFunction", "schema", "database", "type", "domain",
];
/** "Varsayılan yetkiler…" yalnız veritabanı ve şema düğümlerinde. */
export const DEFAULT_ACL_KINDS: NodeKind[] = ["database", "schema"];
