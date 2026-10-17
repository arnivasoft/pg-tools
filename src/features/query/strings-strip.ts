// İfade sonuç şeridi metinleri (ResultStrip, süre biçimi). Yalnızca tembel QueryTab parçası (ResultStrip, MessagesView,
// HistoryView, ExportUi) içe aktarır: ana pakette durmasın diye ayrı modüldedir (Plan 22 ana paket telafisi; eskiden ana
// `t` nesnesinin `query.strip` bloğu).
export const tstrip = {
  ariaLabel: "İfade sonuçları",
  status: { running: "Çalışıyor", ok: "Başarılı", failed: "Hata", cancelled: "İptal edildi" },
  kind: { select: "SELECT", write: "Yazma", ddl: "DDL", tx: "Transaction", other: "Diğer" },
  rows: (n: number) => `${n.toLocaleString("tr-TR")} satır`,
  affected: (n: number) => `${n.toLocaleString("tr-TR")} etkilendi`,
  ms: (n: number) => (n < 1000 ? `${n} ms` : `${(n / 1000).toFixed(2)} sn`),
} as const;
