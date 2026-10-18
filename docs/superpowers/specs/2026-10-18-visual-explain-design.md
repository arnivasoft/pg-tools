# pg-tools — Görsel EXPLAIN Tasarımı

- **Tarih:** 2026-10-18
- **Durum:** Taslak (Plan 28 yazıldı)
- **Kapsam:** Query Tool'da imleçteki ya da seçili **tek** ifadenin planını `EXPLAIN (FORMAT JSON, …)` ile alıp ağaç olarak göstermek. Gösterilenler: düğüm türü, nesne, maliyet, tahmin/gerçek satır sapması, gerçek süre (özel/dahil), döngü, buffer, en pahalı düğümler, düğüm ayrıntı paneli. Ayrıca kullanıcının kendi çalıştırdığı `EXPLAIN (FORMAT JSON)` sonucunu da plan olarak açabilmek.
- **Teslim:** Plan 28 (arka uç komutu + ön yüz "Plan" paneli; tek plan). Yedekleme ön yüzü Plan 27 olarak ayrılmıştır.

---

## 1. Kararlar

| Konu | Karar |
|---|---|
| Hedef ifade | Kapsam mevcut `Scope` (seçim ya da imleç); kapsam **tam bir ifade** içermeli (birden çok → hata "Plan için tek bir ifade seçin."). İfade zaten `EXPLAIN` ile başlıyorsa reddedilir ("İfade zaten EXPLAIN; çalıştırıp 'Planı göster'i kullanın."). EXPLAIN'e uygun olmayan ifadeler (DDL, yardımcı komutlar, `BEGIN`/`SET`…; sınıflandırıcı `StmtKind` ve ilk anahtar sözcük) reddedilir; uygun olanlar: `SELECT`, `WITH`, `VALUES`, `TABLE`, `INSERT`, `UPDATE`, `DELETE`, `MERGE` (PG 15+), `EXECUTE`, `DECLARE`, `CREATE TABLE … AS`, `CREATE MATERIALIZED VIEW … AS`, `SELECT … INTO`. |
| Seçenekler | `analyze` (varsayılan kapalı), `buffers` (ANALYZE ile; PG 13+'ta ANALYZE'sız da), `verbose`, `settings` (PG 12+), `wal` (PG 13+, ANALYZE ile), `timing` (ANALYZE ile; kapatılabilir), `costs` (varsayılan açık), `summary` (ANALYZE ile varsayılan). Sürüm altı seçenek doğrulama hatası (sessiz atlama yok). Üretilen ifade: `EXPLAIN (FORMAT JSON[, ANALYZE][, BUFFERS]…) <ifade>`; ifade metni kullanıcınınkiyle aynen (sondaki `;` atılır). |
| Güvenlik (ANALYZE) | `EXPLAIN ANALYZE` ifadeyi **çalıştırır**. Sınıflandırma sarmalanmış metinle yapılır (mevcut `classify`: `EXPLAIN ANALYZE <DML>` → Write + tehlike). Kilitli sunucuda yazma reddedilir; tehlike/üretim yazımı onay ister (mevcut `confirmation_required` akışı ve `ConfirmRunDialog`). **Otomatik geri alma:** yazan ifadelerde (Write) seçenek `rollback` varsayılan **açık**: oturum transaction dışındaysa (`TxState::Idle`, autocommit ya da manuel modda açık transaction yok) `BEGIN; EXPLAIN …; ROLLBACK` ile çalıştırılır ve değişiklik kalıcılaşmaz (not: sıra/`nextval` gibi transaction dışı etkiler geri alınmaz — notta belirtilir). Oturum zaten bir transaction içindeyse otomatik geri alma yapılmaz (kullanıcının transaction'ı bozulmaz); uyarı: "Etkiler açık transaction'ınızda kalır." `Failed` transaction'da plan alınamaz (mevcut kural). |
| Yürütme | Mevcut sekme oturumu (aynı bağlantı, ayarlar, `statement_timeout`, iptal `qt_cancel`); oturum meşgulse `busy`. Sonuç satırları akış olarak değil, tek JSON metni olarak döner; boyut sınırı 32 MiB (aşılırsa hata "Plan çok büyük"). NOTICE'lar da döner. Geçmişe (history) "EXPLAIN" olarak yazılır (mevcut geçmiş kuralıyla; metin olduğu gibi). |
| Ayrıştırma | JSON ön yüzde (tembel parça) saf modelle ayrıştırılır: `[{ "Plan": {...}, "Planning Time", "Execution Time", "Triggers", "JIT", "Settings" }]`. Düğüm alanları sürümler arası değişebildiği için genel ayrıştırma: bilinen anahtarlar tiplenir, kalanlar "diğer" listesinde aynen gösterilir. |
| Hesaplar | Gerçek süreler (`Actual Total Time` × `Actual Loops`) **dahil** süre; **özel** süre = dahil − çocukların dahili (CTE/InitPlan/SubPlan alt planları da çocuk sayılır; negatif → 0). Toplam = kök dahil + `Execution Time` farkı (tetikleyiciler ayrı satır). Satır sapması = max(gerçek×döngü, 1) / max(tahmin×döngü, 1) ya da tersi; ≥10× **uyarı**, ≥100× **kritik**. Paralel düğümlerde döngü işçi sayısını içerir (PG kuralı); not. ANALYZE yoksa süre/gerçek alanlar yok; maliyet tabanlı vurgu (özel maliyet = toplam − çocuk toplamları). |
| Görselleştirme | Girintili ağaç (genişlet/daralt; varsayılan tümü açık, 200+ düğümde 3 düzey), her satırda: düğüm türü (+ ilişki/indeks/join türü/Strategy), süre çubuğu (özel süre / toplam %), maliyet, tahmin→gerçek satır ve sapma rozeti, döngü, buffer (shared hit/read) kısaltması. "En pahalı 5 düğüm" listesi (özel süreye göre, ANALYZE yoksa özel maliyete göre) → tıklayınca ağaçta seçer. Seçili düğüm ayrıntı paneli: tüm anahtar/değerler (Filter, Index Cond, Hash Cond, Output, Sort Key, Rows Removed by Filter…). Üstte özet: planlama/çalışma süresi, tetikleyici süreleri, JIT. "Ham JSON" sekmesi (kopyala). Grafik/akış diyagramı kapsam dışı. |
| Giriş noktaları | ContextBar'da **"Planı göster"** düğmesi + seçenek menüsü; kısayol **F7** (EXPLAIN), **Shift+F7** (EXPLAIN ANALYZE; onay akışı geçerli). Sonuç panelinde yeni **"Plan"** sekmesi. Izgarada tek hücreli `QUERY PLAN` (json) sonucu görülünce "Planı göster" bağlantısı (kullanıcının kendi EXPLAIN'i). |
| Paket | Tümü tembel QueryTab parçasında (plan modeli, görünüm, metinler `strings-plan.ts`); ana pakete yalnız `commands.ts` sarıcısı (telafiyle). |
| Kapsam dışı | Grafik düğüm diyagramı, plan karşılaştırma, `auto_explain` günlüklerini içe aktarma, metin (TEXT) formatlı plan ayrıştırma, plan paylaşma/dışa aktarma (ham JSON kopyalama hariç). |

## 2. Model (özet)

- Arka uç: `ExplainOptions { analyze, buffers, verbose, settings, wal, timing, costs, summary, rollback }`; `qt_explain(session, sql, scope, options, confirmed) -> ExplainResult { json: string, notices, elapsedMs, rolledBack: bool, sql: string (çalıştırılan EXPLAIN metni) }`; hata kodları mevcut `query_payload` + `explain_*` doğrulama iletileri.
- Ön yüz: `PlanNode { id, nodeType, label, relation?, index?, joinType?, startupCost, totalCost, planRows, planWidth, actual?: { startup, total, rows, loops }, inclusiveMs?, exclusiveMs?, exclusiveCost, misestimate?: { factor, direction, level }, buffers?, details: [key, value][], children: PlanNode[] }`, `PlanSummary { planningMs?, executionMs?, triggers[], jit?, settings? }`.

## 3. Test stratejisi

- Rust birim: ifade kapsamı (tek/çok/boş), EXPLAIN'e uygunluk tablosu, seçenek sürüm kapıları, üretilen metin (insta), rollback koşulu (Idle/InTransaction/Failed × Write/Select), sınıflandırma ile kapılar.
- Docker PG 10–18: basit SELECT planı JSON döner ve ayrıştırılır (ön yüz modeline uygun örnek kayıt olarak saklanır), ANALYZE + BUFFERS, `EXPLAIN ANALYZE DELETE` rollback ile satırlar silinmez, manuel modda açık transaction'da rollback yapılmaz, kilitli sunucuda `EXPLAIN ANALYZE UPDATE` reddedilir ama düz `EXPLAIN UPDATE` serbest, `statement_timeout`/iptal.
- vitest: ayrıştırıcı (sürümlerin örnek JSON'ları — Docker testinin ürettiği örnekler fixture olarak), özel/dahil süre, sapma, en pahalı düğümler, büyük plan daraltma kuralı, ham JSON'dan açma.
