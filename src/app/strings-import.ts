// Sunucu içe aktarma diyaloğunun metinleri. Yalnızca tembel `ImportDialog` parçası (ve `import-model`) içe aktarır:
// ana pakette durmasın diye ayrı modüldedir (ana paketteki giriş metinleri `t.import`).
export const timp = {
  choose: "Dosya seç…",
  reading: "Okunuyor…",
  selectRecommended: "Önerilenleri seç",
  ungrouped: "(grupsuz)",
  selectNone: "Hiçbirini seçme",
  duplicate: "zaten var",
  cancel: "Vazgeç",
  doImport: (n: number) => `İçe aktar (${n})`,
  done: (n: number) => `${n} sunucu içe aktarıldı. Parolalar ilk bağlantıda sorulacak.`,
  summary: (s: { total: number; importable: number; duplicates: number; errors: number }) =>
    `${s.total} kayıt · ${s.importable} aktarılabilir · ${s.duplicates} zaten var · ${s.errors} hatalı`,
  warnings: "Uyarılar",
  warnBadge: (n: number) => `⚠ ${n}`,
  showWarnings: (n: number) => `${n} uyarıyı göster`,
  hideWarnings: (n: number) => `${n} uyarıyı gizle`,
  fileFilter: "pgAdmin servers.json",
  colName: "Ad",
  colGroup: "Grup",
  colHost: "Host:Port",
  colUser: "Kullanıcı",
  colStatus: "Durum",
} as const;
