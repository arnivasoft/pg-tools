// Hücre görüntüleyici (biçim sekmeleri) metinleri. Yalnızca tembel QueryTab parçası (QueryTab, CellPanel, CellViewers)
// içe aktarır; ana pakete girmez.
export const tcell = {
  tabs: { text: "Metin", hex: "Onaltılık", image: "Görsel", json: "JSON", xml: "XML", html: "HTML", pdf: "PDF" },
  ariaLabel: "Hücre görünümü",
  reason: {
    isNull: "Değer NULL.",
    notImage: "Tanınan bir görsel biçimi değil (PNG, JPEG, GIF, WebP, BMP, ICO, SVG).",
    notJson: "Geçerli JSON değil.",
    notXml: "XML işaretlemesi değil.",
    notMarkup: "HTML/XML işaretlemesi değil.",
    notPdf: "PDF değil (%PDF- ile başlamıyor).",
    onlyBytea: "Yalnız bytea sütunlarında.",
    onlyText: "Yalnız metin sütunlarında.",
    badHex: "bytea değeri çözülemedi.",
  },
  tooLarge: (size: string, limit: string) =>
    `Değer bu görünüm için çok büyük (${size} MB; sınır ${limit} MB). Metin ya da Onaltılık sekmesini kullanın ya da Kaydet… ile dosyaya yazın.`,
  tooLargeText: (size: string, limit: string) =>
    `Değer bu görünüm için çok büyük (${size} MB; sınır ${limit} MB). Metin ya da Onaltılık sekmesini kullanın.`,
  hex: {
    ariaLabel: "Onaltılık görünüm",
    total: (n: number) => `${n.toLocaleString("tr-TR")} bayt`,
    partial: (shown: string, total: string) => `Yalnız ilk ${shown} MB gösteriliyor (toplam ${total} MB).`,
  },
  image: {
    fit: "Sığdır",
    actual: "Gerçek boyut",
    alt: (kind: string, w: number, h: number) => `Hücre görseli (${kind}, ${w} × ${h})`,
    info: (kind: string, size: string | null, bytes: string) => [kind, size, bytes].filter(Boolean).join(" · "),
    broken: "Görsel çözülemedi (bozuk ya da desteklenmeyen veri).",
  },
  html: {
    source: "Kaynak",
    preview: "Önizleme",
    format: "Biçimlendir",
    frameTitle: "HTML önizlemesi (korumalı)",
    previewNote: "Betikler çalışmaz; uzak görsel ve stiller yüklenmez.",
    previewTooLarge: (limit: string) => `Önizleme için çok büyük (sınır ${limit} MB).`,
  },
  pdf: {
    info: (version: string | null, bytes: string) => `${version ? `PDF ${version}` : "PDF"} · ${bytes}`,
    open: "Varsayılan uygulamada aç",
    opening: "Açılıyor…",
    warn: "PDF dışarıda açılır; güvenmediğiniz belgeleri açmayın.",
  },
  save: "Kaydet…",
  saving: "Kaydediliyor…",
  saved: "Kaydedildi.",
  saveTitle: "Hücre değerini kaydet",
  openFailed: "PDF açılamadı.",
  saveFailed: "Dosya kaydedilemedi.",
  loading: "Görünüm yükleniyor…",
  loadFailed: "Görünüm yüklenemedi.",
  retry: "Yeniden dene",
  widen: "Paneli genişlet",
} as const;
