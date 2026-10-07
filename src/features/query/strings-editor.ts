/** Editör ayarları metinleri: yalnızca tembel QueryTab parçasındaki EditorSettings kullanır (ana paket bütçesi). */
export const tedit = {
  title: "Editör",
  checkToggle: "Yazarken hata denetimi",
  keywordCase: "Anahtar sözcükler",
  keywordCases: { upper: "BÜYÜK", lower: "küçük", preserve: "Koru" },
  indent: "Girinti",
  indentLabel: (n: number) => `${n} boşluk`,
  comma: "Virgül",
  commas: { trailing: "Sonda", leading: "Başta" },
  maxLine: "Satır uzunluğu",
  formatHint: "Biçimlendir: Ctrl+Shift+F (seçim ya da tümü)",
} as const;
