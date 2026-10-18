// QueryTab çalışma alanı metinleri (Mesajlar paneli). Yalnızca tembel QueryTab parçası içe aktarır: ana pakette durmasın
// diye ayrı modüldedir (Plan 17 ana paket telafisi; eskiden ana `t` nesnesinin `query.*` blokları).
export const tqt = {
  editorAria: "SQL editörü",
  resizeEditor: "Editör yüksekliği",
  messages: {
    ariaLabel: "Mesajlar",
    empty: "Mesaj yok.",
    statement: (n: number) => `İfade ${n}`,
    position: (n: number) => `Konum: ${n}`,
    detail: "Ayrıntı",
    hint: "İpucu",
    level: { notice: "Bildirim", error: "Hata", cancelled: "İptal", info: "Tamam" },
  },
  check: {
    error: "Hata",
    info: "Bilgi",
    infoHint: "Betikte önceki bir ifadenin oluşturduğu nesne olabilir.",
    toggle: "Yazarken hata denetimi",
    toggleOn: "Yazarken hata denetimi açık — bu sekmede kapatmak için tıklayın",
    toggleOff: "Yazarken hata denetimi bu sekmede kapalı — açmak için tıklayın",
    unavailable: "Yazarken hata denetimi bağlı oturum ister",
  },
  format: {
    failed: "Biçimlendirilemedi; metin değiştirilmedi.",
    unsafeSelection: "Seçim bir dize, yorum ya da ad içinde başlıyor/bitiyor; biçimlendirilmedi.",
  },
  confirm: {
    title: "Çalıştırmayı onayla",
    intro: "Aşağıdaki ifadeler onay gerektiriyor:",
    run: "Çalıştır",
    more: (n: number) => `… ve ${n} ifade daha`,
    empty: "Çalıştırılacak ifade yok.",
    productionBanner: "Üretim sunucusu — değişiklikler canlı veriyi etkiler.",
    production: "Üretim sunucusunda yazma",
    danger: {
      updateWithoutWhere: "WHERE olmayan UPDATE: tablodaki tüm satırlar değişir.",
      deleteWithoutWhere: "WHERE olmayan DELETE: tablodaki tüm satırlar silinir.",
      truncate: "TRUNCATE: tablo verisi tamamen silinir.",
      drop: "DROP: nesne kalıcı olarak silinir.",
      dropColumn: "DROP COLUMN: sütun ve verisi silinir.",
    },
    locked: {
      write: "Sunucu kilitli: yazma ifadesi reddedildi.",
      ddl: "Sunucu kilitli: şema (DDL) ifadesi reddedildi.",
      readOnlySetting: "Sunucu kilitli: salt okunur ayarını değiştiren ifade reddedildi.",
    },
    lockedSummary: "Sunucu salt okunur kilitli; betik çalıştırılmadı.",
  },
} as const;
