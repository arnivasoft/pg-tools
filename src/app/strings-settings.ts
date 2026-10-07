// Ortak ayar düzenleyicisi (ALTER DATABASE|ROLE … SET) metinleri. Yalnızca SettingsEditor.tsx ve settings-model.ts içe
// aktarır (tembel parça); strings-designer.ts'e eklenmez. PostgreSQL terimleri İngilizce kalır.
export const tset = {
  title: "Ayarlar",
  note: "Ayarlar yeni oturumlarda etkili olur.",
  empty: "Ayar yok.",
  nameLabel: "Ayar adı",
  valueLabel: "Değer",
  namePlaceholder: "work_mem",
  valuePlaceholder: "değer",
  add: "Ayar ekle",
  remove: (n: string) => (n === "" ? "Satırı sil" : `${n} ayarını sil`),
  removed: "Kaldırılacaklar",
  restore: (n: string) => `${n} ayarını geri al`,
  removedNote: "Bu ayarlar RESET ile sıfırlanır.",
  listHint: 'Virgülle ayırın; özel adları çift tırnaklayın (ör. "MySchema", public).',
  rowLabel: (i: number) => `Ayar ${i + 1}`,
  warn: {
    empty: "Ayar adı boş.",
    upper: "Noktasız ayar adları yalnızca küçük harfle yazılır.",
    chars: "Ayar adında yalnızca harf, rakam, '_' ve '$' (parçalar '.' ile) kullanılabilir.",
    duplicate: "Bu ayar birden çok kez tanımlanmış.",
    listCase: (item: string) => `Sunucuda "${item}" (büyük/küçük harf korunur) anlamına gelir; küçük harf istiyorsanız yazımı düzeltin.`,
  },
} as const;
