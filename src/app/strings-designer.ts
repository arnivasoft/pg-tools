// Nesne diyaloğu (designer) metinleri. Yalnızca tembel designer parçası içe aktarır: ana pakette durmasın diye ayrı
// modüldedir (ana paketin designer metinleri `t.menu.*`). PostgreSQL terimleri İngilizce kalır.
export const tdes = {
  apply: {
    applied: (n: number) => `Değişiklikler uygulandı (${n} ifade).`,
    committed: (n: number) => `Hata öncesi ${n} ifade kalıcı oldu; nesne kısmen değişmiş olabilir.`,
    failed: (index: number, text: string) => `${index + 1}. ifade başarısız: ${text}`,
    lockTimeout: "Kilit beklerken zaman aşımı (lock_timeout). Nesneyi kullanan oturumlar bitince yeniden deneyin.",
  },
  gate: {
    planChanged: "Nesne değişti; yeniden yükleyin.",
    invalidDef: "Tanım geçersiz; önizleme yenilendi.",
    locked: "Sunucu salt okunur kilitli.",
    confirmationRequired: "Bu değişiklik onay gerektiriyor; önizlemeyi kontrol edip onaylayın.",
  },
  status: {
    gone: "Nesne artık yok.",
    serverGone: "Sunucu kaydı silindi.",
    dropped: "Nesne silindi.",
  },
} as const;
