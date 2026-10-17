// Sonuç ızgarası ve hücre paneli metinleri. Yalnızca tembel QueryTab parçası (ResultGrid, CellPanel) içe aktarır: ana
// pakette durmasın diye ayrı modüldedir (Plan 19 ana paket telafisi; eskiden ana `t` nesnesinin `query.grid`/`query.cell` blokları).
export const tgrid = {
  grid: {
    ariaLabel: "Sorgu sonucu",
    nullText: "[NULL]",
    rowsLoaded: (n: number) => `${n.toLocaleString("tr-TR")} satır yüklendi`,
    loadMore: (n: number) => `Daha fazla yükle (${n.toLocaleString("tr-TR")})`,
    limitWarning: "Satır sınırına ulaşıldı; daha fazla satır var olabilir.",
    dropped: "Bellek sınırı nedeniyle bu ifadenin satırları atıldı; sonucu görmek için sorguyu yeniden çalıştırın.",
    rowNumber: "Satır no",
    noRows: "Satır yok.",
    resize: "Sütunu yeniden boyutlandır",
  },
  cell: {
    title: "Hücre değeri",
    empty: "Bir hücre seçin.",
    copy: "Kopyala",
    copied: "Kopyalandı",
    nullValue: "NULL",
    emptyString: "(boş metin)",
    clipped: "Değer 1 MB'ı aştığı için kırpıldı.",
    length: (n: number) => `${n.toLocaleString("tr-TR")} karakter`,
  },
} as const;
