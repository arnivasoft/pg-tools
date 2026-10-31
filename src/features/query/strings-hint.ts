// Sorgu hata ipuçları (errors.ts `hintFor`). Yalnızca tembel parçalar (errors.ts tüketicileri) içe aktarır: ana pakette
// durmasın diye ayrı modüldedir (Plan 27 ana paket telafisi; eskiden `t.query.hint`). `locked` ana pakette (`actions.ts`) kaldı.
export const thint = {
  autocommit: "Bu ifade transaction bloğunda çalışamaz; sekmeyi autocommit kipine alın.",
  rollback: "Transaction hata durumunda; yalnızca ROLLBACK yapılabilir.",
  sessionReset:
    "Bağlantı yeniden kuruldu; oturum durumu (SET, geçici tablolar, advisory kilitler) kaybolmuş olabilir. Sorguyu tekrar çalıştırın.",
} as const;
