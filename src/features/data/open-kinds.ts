import type { NodeKind } from "../../ipc/bindings/NodeKind";

/**
 * "Verileri Görüntüle/Düzenle" menüsünün göründüğü düğümler (bölümler ve bölümlü ana tablo da `table` düğümüdür;
 * kolon/kısıt/koleksiyon düğümleri değil). Gezgin bunu statik içe aktarır; bu dosya bilerek bağımlılıksızdır (veri
 * parçasının geri kalanını ana pakete çekmez).
 */
export const DATA_KINDS: NodeKind[] = ["table", "view", "materializedView"];

export function canOpenData(kind: NodeKind): boolean {
  return DATA_KINDS.includes(kind);
}
