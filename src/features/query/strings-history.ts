// Sorgu geçmişi metinleri. Yalnızca tembel QueryTab parçası (HistoryView) içe aktarır: ana pakette durmasın diye ayrı
// modüldedir (Plan 17 ana paket telafisi; eskiden ana `t` nesnesinin `query.history` bloğu).
export const thist = {
  ariaLabel: "Sorgu geçmişi",
  search: "Geçmişte ara",
  empty: "Geçmiş boş.",
  loading: "Yükleniyor…",
  failed: "Başarısız",
  insertHint: "Editöre ekle",
  saveToggle: "Sorgu geçmişini kaydet",
  clear: "Geçmişi/taslakları temizle",
  clearTitle: "Geçmişi temizle",
  clearMessage: "Sorgu geçmişi ve kapalı sekmelerin taslakları silinecek; açık sekmeler korunur. Bu geri alınamaz.",
  clearConfirm: "Temizle",
} as const;
