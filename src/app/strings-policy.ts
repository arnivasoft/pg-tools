// Policy formu metinleri. Yalnızca PolicyForm.tsx ve policy-model.ts içe aktarır (tembel parça);
// strings-designer.ts'e eklenmez. PostgreSQL terimleri İngilizce kalır.
export const tpol = {
  sections: {
    general: "Genel",
    command: "Komut ve tür",
    roles: "Roller",
    expressions: "İfadeler",
    rls: "Satır güvenliği durumu",
  },
  general: {
    table: "Tablo",
    tableHint: "Politikanın tablosu değiştirilemez.",
    name: "Ad",
    rename: "Yeniden adlandır",
    namePlaceholder: "politika_adı",
  },
  command: {
    label: "Komut (FOR)",
    all: "ALL",
    select: "SELECT",
    insert: "INSERT",
    update: "UPDATE",
    delete: "DELETE",
    kind: "Tür",
    permissive: "PERMISSIVE",
    permissiveHint: "izin veren; politikalar VEYA ile birleşir",
    restrictive: "RESTRICTIVE",
    restrictiveHint: "kısıtlayan; VE ile birleşir",
  },
  roles: {
    label: "Roller (TO)",
    add: "Rol ekle",
    addPick: "Rol seçin…",
    publicNote: "PUBLIC (herkes)",
    hint: "Liste boşsa politika PUBLIC'e, yani tüm rollere uygulanır.",
    list: "Seçili roller",
    removeRole: (name: string) => `${name} rolünü kaldır`,
  },
  expr: {
    using: "USING ifadesi",
    usingPlaceholder: "ör. owner_id = current_user_id()",
    usingClosed: "INSERT politikasında USING kullanılmaz.",
    withCheck: "WITH CHECK ifadesi",
    withCheckPlaceholder: "ör. owner_id = current_user_id()",
    withCheckClosed: "SELECT ve DELETE politikasında WITH CHECK kullanılmaz.",
    hint: "USING mevcut satırları, WITH CHECK yazılan satırları sınar. Boş bırakılan ifade yok sayılır.",
  },
  rls: {
    title: "Tablo satır güvenliği",
    enabled: "açık",
    disabled: "kapalı",
    forced: "zorlanmış",
    notForced: "zorlanmamış",
    line: (enabled: boolean, forced: boolean) =>
      `Tablo satır güvenliği: ${enabled ? "açık" : "kapalı"}, ${forced ? "zorlanmış" : "zorlanmamış"}`,
    offWarning: "Tabloda satır güvenliği kapalı; politika etkili olmaz.",
  },
  rebuild: {
    title: "Politika aynı transaction içinde yeniden kurulur (Yıkıcı)",
    reasons: {
      command: "Komut değişti",
      permissive: "PERMISSIVE/RESTRICTIVE değişti",
      usingRemoved: "USING ifadesi kaldırıldı",
      withCheckRemoved: "WITH CHECK ifadesi kaldırıldı",
    },
  },
} as const;
