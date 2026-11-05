// Asistan metinleri. Yalnız tembel AiPane ve QueryTab içe aktarır; ana pakete girmez. Şema farkı ve dashboard
// girişlerinin metinleri kendi dosyalarındadır (bu dosya ortak parçaya bölünmesin).
export const tai = {
  pane: "Asistan",
  ariaLabel: "Yapay zekâ asistanı",
  promptLabel: "İstek",
  promptPlaceholder: "Örn. son 30 günde en çok sipariş veren 10 müşteri",
  generate: "Gönder",
  followUpPlaceholder: "Devam sorusu (önceki yanıtları bilir)…",
  newChat: "Yeni sohbet",
  historyAria: "Sohbet geçmişi",
  youAria: "Siz",
  assistantAria: "Asistan",
  sentSql: "Gönderilen SQL",
  notCommitted: "Bu yanıt sohbet geçmişine eklenmedi.",
  fallbackNote: "Yanıt yedek modelle tamamlandı.",
  explain: "Seçimi açıkla",
  fix: "Hatayı düzelt",
  cancel: "Durdur",
  insert: "Editöre ekle",
  copy: "Kopyala",
  copied: "Kopyalandı",
  thinking: "Düşünüyor…",
  gathering: "Bağlam hazırlanıyor…",
  empty: "İsteğinizi yazın ya da editörde bir ifade seçip açıklatın; devam sorularında asistan önceki yanıtları bilir. Üretilen SQL editöre eklenir; asla kendiliğinden çalıştırılmaz.",
  privacy:
    "Seçili SQL, son hata ve katalog bilgisi (tablo/kolon adları, türler, kısıtlar, indeksler, açıklamalar, görünüm tanımları, tahmini satır sayıları) Anthropic'e (Claude API) gönderilir. Satır verisi gönderilmez; asistan veritabanında yalnız sabit katalog sorguları çalıştırabilir.",
  usage: (inp: number, cache: number, out: number) => {
    const n = (x: number) => x.toLocaleString("tr-TR");
    return cache > 0
      ? `${n(inp)} girdi (+${n(cache)} önbellekten) / ${n(out)} çıktı belirteci`
      : `${n(inp)} girdi / ${n(out)} çıktı belirteci`;
  },
  tool: {
    list_relations: "Tablolar listelendi",
    describe_relation: "Tablo incelendi",
    search_columns: "Kolon arandı",
  } as Record<string, string>,
  toolsAria: "Asistanın baktığı katalog bilgileri",
  /** Başka ekranlardaki "Asistana sor" girişleri. */
  entry: {
    fix: "Asistanla düzelt",
    fixTitle: "Son hatayı ve hata veren ifadeyi asistana gönderir",
    plan: "Planı açıkla",
    planTitle: "Planı ve sorguyu asistana gönderir: darboğaz, tahmin hataları ve öneriler",
  },
  maxTokens: "Yanıt uzunluk sınırında kesildi.",
  refused: "Asistan bu isteği yanıtlamadı.",
  noError: "Bu sekmede düzeltilecek bir hata yok: önce sorguyu çalıştırın.",
  noTab: "Önce bir sorgu sekmesi açın.",
  key: {
    title: "Anthropic API anahtarı",
    intro: "Asistan Claude API'sini kullanır. Anahtar Windows Kimlik Bilgisi Yöneticisi'nde saklanır ve arayüze geri gönderilmez.",
    placeholder: "sk-ant-…",
    save: "Kaydet",
    clear: "Anahtarı sil",
    saved: "API anahtarı kaydedildi.",
    cleared: "API anahtarı silindi.",
  },
};
