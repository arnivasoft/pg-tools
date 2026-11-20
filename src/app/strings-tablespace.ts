// Tablespace formu metinleri. Yalnızca TablespaceForm.tsx içe aktarır (tembel parça); strings-designer.ts'e eklenmez.
// PostgreSQL terimleri İngilizce kalır.
export const ttsp = {
  sections: { general: "Genel", options: "Seçenekler" },
  createNote: "Tablespace oluşturmak süper kullanıcı yetkisi ister.",
  general: {
    name: "Ad",
    rename: "Yeniden adlandır",
    namePlaceholder: "tablespace_adı",
    nameHint: "pg_ ile başlayamaz.",
  },
  location: {
    label: "Konum (LOCATION)",
    placeholder: "D:\pgdata\ts1",
    createHint:
      "Süper kullanıcı gerekir; dizin sunucuda var olmalı, boş ve PostgreSQL hizmet kullanıcısına ait olmalı. " +
      "örn. D:\pgdata\ts1 ya da /var/lib/postgresql/ts1",
    alterHint: "Konum oluşturulduktan sonra değiştirilemez.",
    empty: "—",
  },
  options: {
    unset: "ayarsız (sunucu varsayılanı)",
    maintenanceHint: "PG 13+",
    seqPageCost: "seq_page_cost",
    randomPageCost: "random_page_cost",
    effectiveIoConcurrency: "effective_io_concurrency",
    maintenanceIoConcurrency: "maintenance_io_concurrency",
  },
  warnings: {
    notNumber: "Sayı değil.",
    negative: "Negatif olamaz.",
    notInteger: "Tamsayı olmalı.",
    outOfRange: "0–1000 aralığı dışında.",
  },
} as const;
