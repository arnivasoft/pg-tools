import type { NodeKind } from "../../ipc/bindings/NodeKind";

/**
 * Nesne diyaloğu (designer) menü eylemlerinin göründüğü düğüm türleri. Yalnızca menü görünürlüğü içindir: hedef yine
 * `ddl_target_of` ile alınır ve arka uç desteklemediği düğümü reddeder. Gezgin bunu statik içe aktarır; bu dosya bilerek
 * bağımlılıksızdır (designer parçasının geri kalanını ana pakete çekmez).
 */
export const DDL_OBJECT_KINDS: NodeKind[] = ["schema", "sequence", "extension", "index"];
/** schemaCollection, sequenceCollection, extensionCollection, indexCollection. */
export const DDL_CREATE_COLLECTIONS = DDL_OBJECT_KINDS.map((k) => `${k}Collection` as NodeKind);

/** "Oluştur…" bu koleksiyonda gösterilir mi? */
export const canCreateIn = (kind: NodeKind) => DDL_CREATE_COLLECTIONS.includes(kind);
/** "Düzenle… / Yeniden adlandır… / Sil…" bu nesnede gösterilir mi? */
export const canEditObject = (kind: NodeKind) => DDL_OBJECT_KINDS.includes(kind);
