// Domain formu metinleri. Yalnızca DomainForm.tsx içe aktarır (tembel parça); strings-designer.ts'e eklenmez.
// PostgreSQL terimleri İngilizce kalır.
export const tdom = {
  sections: {
    general: "Genel",
    definition: "Tanım",
    checks: "Domain kısıtları (CHECK)",
  },
  general: {
    schema: "Şema",
    schemaCreateHint: "Domain'in oluşturulacağı şema.",
    schemaAlterHint: "Değişirse domain bu şemaya taşınır (SET SCHEMA).",
    name: "Ad",
    rename: "Ad (değişirse yeniden adlandırılır)",
    namePlaceholder: "domain_adı",
  },
  definition: {
    baseType: "Temel tip",
    collation: "Collation",
    collationPlaceholder: "(varsayılan)",
    createOnlyHint: "Temel tip ve collation oluşturulduktan sonra değiştirilemez.",
    default: "DEFAULT ifadesi",
    defaultPlaceholder: "(yok)",
    notNull: "NOT NULL",
    notNullHint: "Mevcut veride NULL varsa uygulama başarısız olur (Yıkıcı).",
    pg17Note:
      "PostgreSQL 17+: adlı bir NOT NULL kısıtının adı korunmaz; NOT NULL olarak yüklenir, yeniden yazılırsa adı sunucu verir.",
  },
  escapeHint: "Ters bölü içeren metin sabitlerini E'…' biçiminde yazın; yüklenen düz \\ içeren ifade düzenlenemez.",
  checks: {
    add: "Kısıt ekle",
    empty: "CHECK kısıtı yok.",
    name: "Kısıt adı",
    namePlaceholderCreate: "(isteğe bağlı — adı sunucu verir)",
    namePlaceholderAlter: "kısıt_adı",
    nameRequired: "Düzenlemede yeni kısıtın adı zorunludur (sonradan eşlenebilmesi için).",
    renamedFrom: (old: string) => `Yüklü ad: ${old} (RENAME CONSTRAINT)`,
    expression: "İfade",
    expressionPlaceholder: "VALUE > 0",
    exprChanged: "İfade değişti: kısıt kaldırılıp yeniden eklenir.",
    notValid: "NOT VALID (mevcut veriyi denetlemeden ekle)",
    validate: "Doğrula (VALIDATE CONSTRAINT)",
    validateHint: "Mevcut veriyi bu kısıta göre denetler.",
    notValidated: "doğrulanmamış",
    notValidatedSr: "Bu kısıt NOT VALID: mevcut veri henüz denetlenmedi.",
    newBadge: "yeni",
    removeHint: "Yüklü kısıtı silmek DROP CONSTRAINT üretir (Yıkıcı).",
  },
};
