// Dashboard metinleri (`strings.ts`'in `dashboard` bölümü). Yalnızca tembel dashboard parçası içe aktarır: ana pakette
// durmasın diye ayrı modüldedir (ana paketin tek dashboard metni `t.menu.openDashboard`).
export const td = {
  tabTitle: (server: string, db?: string | null) => `Dashboard · ${db ? `${server}/${db}` : server}`,
  loading: "Yükleniyor…",
  waiting: "Veri bekleniyor…",
  status: {
    connecting: "Bağlanıyor…",
    ready: "Canlı",
    paused: "Duraklatıldı",
    lost: "Bağlantı kesildi",
    gone: "Sunucu silindi",
  },
  lostNote: "Dashboard oturumu kapandı; yenileme durdu.",
  goneNote: "Bu sunucunun kaydı silindi; dashboard yeniden açılamaz.",
  reopen: "Yeniden aç",
  noPrivilege: "yetki yok",
  partial: "Bazı oturumlar görünmüyor (yetki).",
  sectionsAria: "Dashboard bölümleri",
  sections: {
    overview: "Genel",
    sessions: "Oturumlar",
    locks: "Kilitler",
    settings: "Yapılandırma",
    statements: "Sorgu istatistikleri",
  },
  bar: {
    interval: "Yenileme aralığı",
    seconds: (n: number) => `${n} sn`,
    pause: "Duraklat",
    resume: "Sürdür",
    refresh: "Yenile (F5)",
  },
  signal: {
    cancel: "İptal",
    terminate: "Sonlandır",
    cancelTitle: "Sorguyu iptal et",
    terminateTitle: "Oturumu sonlandır",
    warn: {
      self: "Bu, dashboard'un kendi bağlantısı.",
      pgtoolsQuery: "Bu, pg-tools'un kendi sorgu sekmesi.",
      idleInTx: "Açık transaction geri alınacak.",
      terminate: "Oturum kapanır; bağlı istemci hata alır.",
    },
    noPermission: "Bu oturumu durdurma yetkiniz yok (yalnızca kendi oturumlarınız ya da pg_signal_backend üyeliği).",
    sent: (kind: "cancel" | "terminate") =>
      kind === "cancel" ? "İptal isteği gönderildi." : "Sonlandırma isteği gönderildi.",
    notFound: "Oturum artık yok.",
    changed: "Bu pid artık başka bir oturuma ait; işlem yapılmadı.",
    unverified: "Oturum doğrulanamadı (liste kesildi); işlem yapılmadı.",
    ownSession: "Kendi dashboard oturumunuzu durduramazsınız.",
  },
} as const;
