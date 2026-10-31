// Görsel EXPLAIN (plan paneli) metinleri. Yalnızca tembel QueryTab parçası içe aktarır: ana pakette durmasın diye
// ayrı modüldedir (spec §1 "Paket"; `app/strings` içine eklenmez).
export const tplan = {
  errInvalidJson: "Plan JSON'u çözümlenemedi.",
  errNoPlan: "JSON bir EXPLAIN planı içermiyor.",
} as const;
