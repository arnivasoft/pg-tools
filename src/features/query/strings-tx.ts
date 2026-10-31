// Transaction kapatma/toplu kapatma diyalogları (CloseTxDialog, BulkTxDialog) metinleri. Yalnızca tembel `tx-dialogs` parçası
// içe aktarır: ana pakette durmasın diye ayrı modüldedir (Plan 27 ana paket telafisi; eskiden `t.query.close`/`t.query.bulk`).
// `close.failed/closing` ve `bulk.serverMessage/exitMessage` sorgu denetleyicisi (ana paket) kullandığı için `app/strings`te kalır.
export const ttx = {
  close: {
    title: "Açık transaction",
    message: (title: string) => `‘${title}’ sekmesinde açık bir transaction var. Sekme kapanmadan önce ne yapılsın?`,
    failedMessage: (title: string) => `‘${title}’ sekmesindeki transaction hatalı; yalnızca geri alınabilir.`,
    busyNote: "Çalışan sorgu iptal edilip geri alınacak (en çok ~10 sn sürebilir).",
    commit: "Kaydet (COMMIT)",
    rollback: "At (ROLLBACK)",
  },
  bulk: {
    title: "Açık transaction'lar",
    commitAll: "Tümünü kaydet",
    rollbackAll: "Tümünü at",
    failedNote: "Hatalı ya da çalışan sekmeler her durumda geri alınır.",
    tx: { inTransaction: "açık", failed: "hatalı", idle: "" },
    busy: "çalışıyor",
  },
} as const;
