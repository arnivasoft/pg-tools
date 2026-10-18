// Bakım diyaloğunun metinleri (Plan 25). Yalnızca tembel `MaintenanceDialog` parçası ve saf model içe aktarır; ana pakette
// durmasın diye ayrı modüldedir (ana paketteki tek metin `t.menu.maintenance`).
export const tm = {
  title: "Bakım",
  preparing: "Hazırlanıyor…",
  close: "Kapat",
  waitingProgress: "İlerleme bilgisi bekleniyor…",
  // Sürüm kapısı: arka uç iletisiyle aynı kalıp ("<SEÇENEK> PostgreSQL N ve üzerini gerektirir.").
  gate: (major: number) => `PostgreSQL ${major} ve üzerini gerektirir`,
  excl: {
    full: "FULL ile birlikte kullanılamaz",
    systemObject: "Sistem nesnelerinde kullanılamaz",
    noIndex: "Tabloda seçilebilir indeks yok",
  },
  ops: {
    vacuum: "VACUUM",
    analyze: "ANALYZE",
    reindex: "REINDEX",
    cluster: "CLUSTER",
  },
  opDesc: {
    vacuum: "Ölü satırları temizler ve alanı yeniden kullanıma açar.",
    analyze: "Sorgu planlayıcı için istatistikleri günceller.",
    reindex: "İndeksleri baştan oluşturur.",
    cluster: "Tabloyu bir indeksin sırasına göre yeniden yazar.",
  },
  opts: {
    full: { label: "FULL", desc: "Tabloyu yeniden yazar ve diske alan iade eder; tabloyu kilitler." },
    freeze: { label: "FREEZE", desc: "Satırları agresifçe dondurur." },
    analyze: { label: "ANALYZE", desc: "Temizlikten sonra istatistikleri de günceller." },
    verbose: { label: "VERBOSE", desc: "Ayrıntılı ilerleme iletileri üretir." },
    disablePageSkipping: { label: "DISABLE_PAGE_SKIPPING", desc: "Görünürlük haritasını yok sayıp tüm sayfaları tarar." },
    skipLocked: { label: "SKIP_LOCKED", desc: "Kilidi alınamayan tabloları bekleme, atla." },
    indexCleanup: { label: "INDEX_CLEANUP", desc: "İndeks temizliğini zorla ya da kapat." },
    parallel: { label: "PARALLEL", desc: "İndeks temizliği için en çok bu kadar paralel işçi kullanılır (0–1024)." },
    concurrently: { label: "CONCURRENTLY", desc: "Yazmaları engellemeden yeniden kurar; daha yavaştır." },
    usingIndex: { label: "USING (indeks)", desc: "Tablonun sıralanacağı indeks." },
  },
  indexCleanup: { default: "Varsayılan", auto: "AUTO", on: "ON", off: "OFF" },
  units: { blocks: "blok", tuples: "satır", indexes: "indeks" },
  status: {
    running: "Çalışıyor",
    applied: "Tamamlandı",
    failed: "Başarısız",
    cancelled: "İptal edildi",
  },
  ccnew:
    "Başarısız ya da iptal edilen REINDEX CONCURRENTLY geçersiz bir *_ccnew indeksi bırakabilir; elle silin (DROP INDEX CONCURRENTLY) ya da işlemi yeniden deneyin.",
  // PG'nin İngilizce aşama metinleri (spec §6.5); bilinmeyen olduğu gibi gösterilir.
  phases: {
    // vacuum
    initializing: "Başlatılıyor",
    "scanning heap": "Tablo taranıyor",
    "vacuuming indexes": "İndeksler temizleniyor",
    "vacuuming heap": "Tablo temizleniyor",
    "cleaning up indexes": "İndeks artıkları temizleniyor",
    "truncating heap": "Tablo sonu kısaltılıyor",
    "performing final cleanup": "Son temizlik yapılıyor",
    // cluster
    "seq scanning heap": "Tablo sıralı taranıyor",
    "index scanning heap": "Tablo indeksle taranıyor",
    "sorting tuples": "Satırlar sıralanıyor",
    "writing new heap": "Yeni tablo yazılıyor",
    "swapping relation files": "Dosyalar değiştiriliyor",
    "rebuilding index": "İndeks yeniden kuruluyor",
    // create_index
    "waiting for writers before build": "Kuruluştan önce yazanlar bekleniyor",
    "building index: scanning table": "İndeks kuruluyor: tablo taranıyor",
    "building index: sorting live tuples": "İndeks kuruluyor: satırlar sıralanıyor",
    "building index: loading tuples in tree": "İndeks kuruluyor: ağaç dolduruluyor",
    "waiting for writers before validation": "Doğrulamadan önce yazanlar bekleniyor",
    "index validation: scanning index": "İndeks doğrulanıyor: indeks taranıyor",
    "index validation: sorting tuples": "İndeks doğrulanıyor: satırlar sıralanıyor",
    "index validation: scanning table": "İndeks doğrulanıyor: tablo taranıyor",
    "waiting for old snapshots": "Eski anlık görüntüler bekleniyor",
    "waiting for readers before marking dead": "Ölü işaretlemeden önce okuyanlar bekleniyor",
    "waiting for readers before dropping": "Silmeden önce okuyanlar bekleniyor",
    // analyze
    "acquiring sample rows": "Örnek satırlar alınıyor",
    "acquiring inherited sample rows": "Miras örnek satırlar alınıyor",
    "computing statistics": "İstatistikler hesaplanıyor",
    "computing extended statistics": "Genişletilmiş istatistikler hesaplanıyor",
    "finalizing analyze": "ANALYZE tamamlanıyor",
    // yedek
    unknown: "Bilinmiyor",
  } as Record<string, string>,
};

/** PG aşama metnini Türkçeye çevirir; bilinmeyen olduğu gibi döner. */
export function phaseLabel(phase: string): string {
  return Object.hasOwn(tm.phases, phase) ? tm.phases[phase]! : phase;
}
