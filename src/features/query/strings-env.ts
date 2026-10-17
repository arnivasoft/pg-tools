// Ortam etiketi sorusu (EnvDialog) metinleri. Yalnızca tembel `tx-dialogs` parçası içe aktarır: ana pakette durmasın
// diye ayrı modüldedir (Plan 22 ana paket telafisi; eskiden ana `t` nesnesinin `query.env` bloğu).
export const tenv = {
  title: "Bu sunucunun ortamı?",
  message: (name: string) => `‘${name}’ için bir ortam etiketi seçin. Üretim sunucusunda yazma ifadeleri onay ister ve sekmeler Manuel modda açılır.`,
  skip: "Atla",
  save: "Kaydet",
  lock: "Salt okunur kilidi aç (önerilir)",
} as const;
