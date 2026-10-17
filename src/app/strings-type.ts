// Type formu metinleri. Yalnızca TypeForm.tsx içe aktarır (tembel parça); strings-designer.ts'e eklenmez.
// PostgreSQL terimleri İngilizce kalır.
export const ttyp = {
  sections: {
    general: "Genel",
    kind: "Tip türü",
    values: "Enum değerleri",
    attributes: "Composite öznitelikleri",
    range: "Range tanımı",
  },
  general: {
    schema: "Şema",
    schemaCreateHint: "Tipin oluşturulacağı şema.",
    schemaAlterHint: "Değişirse tip bu şemaya taşınır (SET SCHEMA).",
    name: "Ad",
    rename: "Ad (değişirse yeniden adlandırılır)",
    namePlaceholder: "tip_adı",
  },
  kind: {
    label: "Tür",
    enum: "Enum",
    composite: "Composite",
    range: "Range",
    other: "Diğer (base, array, pseudo, shell ya da multirange)",
    alterHint: "Oluşturulmuş tipin türü değiştirilemez.",
    createHint: "Tür değişince diğer türün alanları saklanır; geri dönünce geri gelir.",
  },
  readOnlyKind:
    "Bu tip türü (base, array, pseudo, shell ya da multirange) bu diyalogda düzenlenemez; yalnızca genel bilgiler gösterilir.",
  values: {
    label: "Etiket",
    placeholder: "değer",
    add: "Değer ekle",
    empty: "Değer yok.",
    newBadge: "yeni",
    renamedFrom: (o: string) => `Yüklü etiket: ${o} (RENAME VALUE)`,
    addHint: "Yeni değer imleçteki satırın altına eklenir (satır seçilmemişse sona).",
    loadedHint:
      "Yüklü değerler silinemez ve yeniden sıralanamaz (PostgreSQL silmeyi/yeniden sıralamayı desteklemez); yalnızca yeniden adlandırılabilir. Yeni değerler yüklü değerlerin arasına taşınabilir.",
    pre12Note:
      "PG 12 öncesinde her ADD VALUE transaction dışında çalışır: bir ifade başarısız olursa öncekiler uygulanmış kalır.",
  },
  attributes: {
    name: "Ad",
    namePlaceholder: "öznitelik_adı",
    type: "Tip",
    collation: "Collation",
    collationPlaceholder: "varsayılan",
    add: "Öznitelik ekle",
    empty: "Öznitelik yok.",
    newBadge: "yeni",
    renamedFrom: (o: string) => `Yüklü ad: ${o} (RENAME ATTRIBUTE)`,
    orderHint: "Yüklü öznitelikler yeniden sıralanamaz; yeni öznitelikler yalnızca sona eklenir.",
    usedHint:
      "Tip bir tablo kolonunda kullanılıyorsa PostgreSQL tip değişimini reddeder; typed table kullanan tiplerde öznitelik değişimi CASCADE ister (desteklenmez).",
    retypeWarning: "Tip/collation değişimi (DataLoss): tipi kullanan tablolardaki değerler dönüştürülür ya da kaybolur.",
    lossTitle: "Veri kaybı (DataLoss)",
    dropped: (n: string) => `Silinecek öznitelik: ${n} — tipi kullanan tablolardaki değerleri kaybolur.`,
    retyped: (n: string) => `Tipi değişecek öznitelik: ${n}`,
    restore: (n: string) => `${n} özniteliğini geri al`,
    restoreShort: "Geri al",
  },
  range: {
    subtype: "Alt tip (SUBTYPE)",
    opclass: "Alt tip opclass (SUBTYPE_OPCLASS)",
    collation: "Collation",
    canonical: "CANONICAL fonksiyonu",
    subtypeDiff: "SUBTYPE_DIFF fonksiyonu",
    multirange: "Multirange tip adı (MULTIRANGE_TYPE_NAME)",
    optional: "boş = varsayılan",
    fnHint: "ad ya da şema.ad (gerekirse çift tırnak).",
    multirangeHint:
      "PG 14+; boş = PostgreSQL otomatik ad üretir. Multirange tipi range ile aynı şemaya kurulur (şeması ayrıca seçilemez).",
    alterHint: "Range tipinin alanları oluşturulduktan sonra değiştirilemez.",
  },
} as const;
