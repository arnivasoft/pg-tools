# Query Tool Plan 6 notları (arka uç devir listesi)

Plan 5 arka ucundan (`pgt-query`, `src-tauri/src/query.rs`) arayüze (Plan 6) kalan sözleşmeler ve bilinen
sınırlar. Ayrıntılar için ilgili kod yorumlarına bakın.

## Plan 6 durumu (madde madde)

Aşağıda bu belgenin her maddesi için Plan 6 (Query Tool arayüzü) sonunda durum yazılıdır. **✔ Plan 6'da karşılandı**,
**✘ Karşılanmadı** (nedeniyle birlikte), **— Arka uç sınırı** (arayüzün yapacağı bir şey yok; bilinen sınır olarak kalır).
Doğrulama notları: **[test]** birim testi, **[T8]** Task 8 elle/CDP duman testi, **[T9]** Task 9 yayın exe çalıştırması.

### Olay akışı
- ✔ `qt_run` `ScriptDone` ile biter, `invoke` çözülmesi bitiş sayılmaz (`result-model.ts`/`run-model.ts`) **[test]**.
- ✔ Olay sırası ifade başına işlenir; `Failed.notices` Mesajlar'a eklenir (`result-model.ts` `applyEvent`) **[test]**.
- ✔ İfadeler arası iptal (`failed` + `scriptDone`) sıradan hata gibi işlenir; çalıştırma biter **[T8]** (iptal).
- — Atılan NOTICE sayısı olaylarda yok; arayüz "daha fazla NOTICE atıldı" demez (arka uç sınırı).
- ✔ `more()` yalnızca son `runId`'nin ızgarasından ve `canLoadMoreRows` iken istenir; yeni çalıştırma/`qt_more` sürerken ve dışa aktarırken tetikler engelli **[test] [T8]**.
- ✔ `truncated` "Satır sınırına ulaşıldı; daha fazla satır var olabilir" olarak gösterilir (kesin değil, olabilir) **[T8]**.

### Konumlar
- ✔ `charOffset`/`charLen`/`Failed.position` Unicode scalar → CodeMirror UTF-16 (`offsets.ts`); hata işareti ve onay diyaloğu metni bununla çizilir **[test]**; hata işareti elle (emojisiz) **[T8]**.
- ✔ Girdiler (`Scope.selection`, `Scope.atCursor.offset`) UTF-16 → scalar dönüştürülerek gönderilir **[test]**. Emojili betikte uçtan uca seçim çalıştırma **elle denenmedi** (smoke-test.md'de açık madde).

### Hata kodları
- ✔ `session_reset` dahil tüm kodlar için `isQueryErrorCode` tür koruyucusu (`errors.ts`) **[test]**.
- ✔ `qt_open` hataları: `invalid` ve `unknown_server` → sekme hata durumu / "sunucu yok" (`routeError`); `qt_complete`'in boş listesi sessiz kalır; `not_found` → oturum kayıp, "Yeniden bağlan" **[test]**, "sunucu yok" **[T8]**.
- ✔ `session_reset`: Mesajlar'da açıklama + "Tekrar çalıştır" düğmesi, `hintFor` **[test]**. **Gerçek PG yeniden başlatmasıyla elle denenmedi** (smoke-test.md'de açık madde).

### Onay akışı
- ✔ Onay diyaloğu `qt_analyze` ile aynı metin ve kapsamla `confirmed=true` yeniden çağırır; üretim bandı (`production = isProductionLabel`); odak "Vazgeç"te **[test] [T8]**.
- ✔ `DO`/`CALL` üretimde onay ister (`confirm-model`) **[test]**; elle yalnızca yazma onayı **[T8]**.
- ✔ Kilitli sunucuda `locked`: ifade başına neden Mesajlar'da (neden yoksa yedek metin), sekme idle'a döner **[test]**; kilitli sunucuda elle deneme **yapılmadı**. Kilidin güvenlik sınırı olmadığı uyarısı (`docs/testing.md`) geçerli — — arka uç sınırı.
- ✔ Kilit/üretim etiketi değişince oturumlar kapanır; açık transaction varsa değişiklikten önce toplu diyalog, sonra sekmeler "Yeniden bağlan" **[T8]**.

### Transaction durumu
- ✔ `Failed`'da yalnızca Rollback etkin ("Transaction hatalı — yalnızca Rollback") **[T8]**.
- ✔ 25001 → "autocommit moduna geçin" ipucu **[T8]**.
- — Durum sondayla düzeltilir, her ifadeden sonra değil; geç kalan iptal sonraki iç komutu vurabilir (arka uç sınırı).
- ✔ İptal her basışta yeniden gönderilir ve düğme `cancelling`'de de etkin kalır (tekrar basılabilir) — `canCancel` **[test]**; "tekrar basma" **elle ayrıca denenmedi** (iptal ilk basışta ~100 ms'de sonuçlandı **[T8]**).
- — Autocommit ızgara imleci açıkken transaction açık kalır (5 dk boşta kapanır): arka uç sınırı; arayüz ek bir şey yapmaz.

### Oturum yaşam döngüsü
- ✔ Sayfa yenilemede tüm oturumlar kapanır; geri yüklenen taslaklar oturumsuz (`noSession`) açılır, ilk çalıştırmada bağlanır **[T8]**. Çoklu pencere yok (filtre gerekmez) — ✘ ikinci pencere/pencere başına filtre **karşılanmadı** (kapsam dışı).
- ✔ Meşgul sekmeyi kapatma: bilgi bildirimi, sekme kapanır, PG'de yetim sorgu yok (215 ms'de kapandı) **[T8]**. En kötü ~10 sn / N × ~10 sn sınırı arka uçta kalır — ✘ kapanış sırasında ilerleme göstergesi **eklenmedi**.
- ✔ Çıkışta `close_all`: pencere kapanışı → açık/hatalı transaction'lar için tek diyalog → kapat **[T8]**. Bu sırada Plan 5'ten kalan `RunEvent::Exit` içindeki `there is no reactor running` paniği düzeltildi (`block_on`).
- ✔ Durumlu oturum: `session_reset` yolu hint'le karşılanır (bkz. Hata kodları); boşta 30 dk uyutma arayüzde görünmez.
- ✔ Kilitsiz oturumun `default_transaction_read_only=off` / `statement_timeout` ezme davranışı arayüzde zaman aşımı seçicisiyle (30 sn / 5 dk / Sınırsız) açığa çıkarıldı; ✘ rol/veritabanı düzeyindeki DBA politikasına uyma (ör. salt okunur rolü ezmeme) için ayar **eklenmedi** (politika kararı Plan 7+).

### Onay / dışa aktarma / diğer
- ✔ Tamamlama yazılırken çalıştırmadan önce `closeCompletion` çağrılır; anlık `busy` toast + sekme idle **[T8]** (kod incelemesi; yarış elle zorlanmadı).
- — `CompletionKind::Function` hiç üretilmez (arka uç).
- ✔ `qt_set_options` sonrası ızgara "daha fazla" düğmesi kapanır (`optionsChanged`) **[test]**.
- ✔ CSV: ayrı imleç; yalnızca tek SELECT/VALUES/TABLE/WITH — dışa aktar düğmesi `canExport`'a göre etkin **[test]**.
- ✔ CSV üzerine yazma: Windows kaydet diyaloğunun "zaten var" onayı (+ ad sadeleştirme) **[T8]**. ✘ Hedef Excel'de açıkken hata **ancak sonda** görünür — önceden yazılabilirlik denemesi **eklenmedi** (arka uç sınırı korunuyor).
- ✘ Kullanıcı iptali ile `statement_timeout`'u ayıran ayrı hata kodu **eklenmedi**; ayrım ileti metninden (arka uç değişmedi).
- ✘ `smoke-release.ps1` hâlâ `qt_*` çağırmaz (yalnızca açılış). Uçtan uca doğrulama `docs/smoke-test.md` "Query Tool" listesi ve Task 8/9 CDP çalıştırmalarıyla yapıldı; **betikleştirilmiş, tekrarlanabilir bir uçtan uca testi eklenmedi** (CDP betikleri depoda değil).
- — NOTICE'lar ifade başına 100 adet / 2.000 karakter (arka uç sınırı).
- ✘ **Hücre başına bayt sınırı arka uçta yok ve eklenmedi.** Arayüz hücre *gösterimini* 1000 karaktere kırpar ve ayrı hücre panelinde tam değeri gösterir (`grid-model.ts` `displayCell`) **[test]**; ancak 1 GB'lık tek hücre hâlâ bütünüyle bellekte tutulur ve `Rows` olayıyla taşınır. İzleme işi olarak kalır.

---

## Olay akışı
- Arayüz bir `qt_run`'ı **`ScriptDone` olayıyla** bitirmelidir; `Channel` iletileri `invoke` çözüldükten sonra da
  gelebilir. `invoke`'un çözülmesi "bitti" demek değildir.
- Olay sırası ifade başına: `started` -> `rows`* -> `done`, ya da herhangi bir noktada `failed`; betik sonunda
  `scriptDone { stmtsRun, stoppedOnError }`. `Failed` da hataya kadar gelen NOTICE'ları taşır (`notices` alanı;
  ör. `RAISE NOTICE` ardından `RAISE EXCEPTION`) — arayüz bunları da göstermeli.
- İfadeler arasında (ya da yeniden bağlanma / otomatik BEGIN sırasında) gelen iptal, sıradaki ifade için
  `failed` (`Sorgu kullanıcı tarafından iptal edildi.`) + `scriptDone { stoppedOnError: true }` üretir.
- NOTICE sınırı aşılınca atılan NOTICE sayısı olaylarda **yoktur** (yalnızca iz kaydında); arayüz "daha fazla
  NOTICE atıldı" diyemez.
- `more()` ifadeyi `stmt_index` ile tanımlar (çalıştırma kimliği yok): yeni bir çalıştırma başladıktan sonra
  eski ızgaradan `more` istenmemeli.
- `truncated` imleç yolunda sonuç tam `limit` satırsa da `true` olabilir.

## Konumlar
- `charOffset`, `charLen` (`StmtPlan`) ve `Failed.position` **Unicode karakter** (scalar) ofsetidir; CodeMirror UTF-16
  kod birimi kullanır. Astral karakterler (emoji vb.) için arayüz dönüşümü yapmalıdır. `position` ifade metnine
  göredir; betikteki yeri `charOffset + position - 1`. Onay diyaloğu ifade metnini
  `[charOffset, charOffset + charLen)` aralığından alabilir.
- **Girdiler de Unicode scalar ofsetidir**: `Scope.selection.start/end` ve `Scope.atCursor.offset`. Arayüz CodeMirror
  UTF-16 konumlarını çağırmadan önce dönüştürmelidir; aksi halde emojiden sonraki bir seçim kaymış metni çalıştırır.

## Hata kodları (`ErrorPayload.code`; metinle eşleştirmeyin)
Tam liste (`QUERY_ERROR_CODES` / `QueryErrorCode.ts`): `confirmation_required`, `locked`, `busy`, `tx_failed`,
`open_transaction`, `session_reset` (tür şu an kullanılmıyor -> Plan 6'da tür koruyucusu ekleyin). Diğer hatalar genel
kodlarla gelir (`query`, `not_found`, `not_connected`, `unknown_server`, `invalid`, ...).
- `qt_complete` bağlantı/sorgu sorunlarında hata vermez: 5 sn'lik zaman aşımı, sorgu hatası, meşgul ya da uyuyan oturum
  **boş liste** döner; ama bilinmeyen/kapatılmış oturum yine `not_found` döner.
- `qt_open`: bağlanırken sunucunun kilidi / üretim etiketi değiştiyse `invalid` ("… bağlanırken değişti; sekmeyi
  yeniden açın."), sunucu silindiyse `unknown_server`; bağlantı bırakılır, oturum açılmaz.
- `session_reset`: durumlu bir oturumun (bkz. Oturum yaşam döngüsü) bağlantısı koptu ve yeniden kuruldu; `SET`
  ayarları, geçici tablolar, advisory kilitler kaybolmuş olabilir. Yalnızca bir kez döner, hiçbir ifade çalışmamıştır;
  yeni bağlantı hazırdır, kullanıcı sorguyu tekrar çalıştırabilir.
  `qt_run` dışında `qt_export_csv` de (kendi oturum bağlantısını uyandırdığında) `session_reset` döndürebilir.

## Onay akışı
- Onay arka uçta zorunludur: `qt_run` `confirmed=false` ile tehlikeli/üretim-yazan betikte `confirmation_required`
  döner; arayüz onay diyaloğunu gösterip `confirmed=true` ile yeniden çağırır. `qt_analyze` hangi ifadelerin neden
  onay istediğini söyler (diyalog metni için).
- `confirmed` betiğe bağlı değildir (genel bir bayraktır): arayüz `qt_analyze` ve `qt_run`'ı **aynı metin ve aynı
  kapsamla** çağırmalıdır; aksi halde kullanıcı başka bir betiği onaylamış olur.
- Üretim etiketli sunucuda `DO` ve `CALL` de onay ister (gövde görünmez). `select yazan_fonksiyon()` onaysız çalışır
  (belgelenmiş sınır).
- Kilitli sunucuda yazma `locked` ile reddedilir; kilit **güvenlik sınırı değildir** (bkz. `docs/testing.md`).
  Kilitli oturumlar **her** `set_config(` çağrısını ve her `U&"…"` / `U&'…'` kaçışlı ad/dizeyi reddeder (belgelenmiş
  yanlış pozitif; `set_config('x','y',false)` gibi zararsız çağrılar da engellenir). CSV dışa aktarma da aynı kilit
  kapısından geçer.
- Sunucu kilidi veya üretim etiketi değişince oturumlar kapatılmalıdır (`close_sessions_of`); bayraklar oturum
  açılırken yakalanır.

## Transaction durumu
- `Failed` durumunda yalnızca `ROLLBACK` / `ROLLBACK TO` kabul edilir; `COMMIT` reddedilir (`tx_failed`).
- Manuel kipte `SAVEPOINT` bile otomatik `BEGIN` alır. Bu yüzden manuel kipte `VACUUM`, `CREATE DATABASE`,
  `CREATE INDEX CONCURRENTLY` gibi transaction bloğunda çalışamayan ifadeler otomatik `BEGIN` sonrası hata verir
  (25001); arayüz bu hatada "autocommit kipine geçin" ipucu göstermeli.
- Durum, `Tx` sınıfı ifadelerden ve hatalardan sonra (ayrıca `qt_tx`, başarısız kapanış-COMMIT'i ve dışa aktarma
  hatasından sonra) sondayla düzeltilir; her ifadeden sonra değil (`docs/testing.md` sınırları).
- Geç kalan bir iptal (ifade bittikten hemen sonra sunucuya ulaşan) bir sonraki iç komutu (imleç `CLOSE`'u ya da
  sonda) vurabilir ve kullanıcının transaction'ını `Failed`'a düşürebilir; durum bir sonraki ifadede sondayla
  düzelir.
- İptal her basışta yeniden gönderilir: PostgreSQL iki gidiş-dönüş arasında gelen iptali yok sayar (imleç yolunda
  bir ifade birden çok `FETCH`'tir); ilk iptal etkisiz kalırsa kullanıcı tekrar basabilir.
- Autocommit ızgara imleci açıkken transaction açık kalır (5 dk boşta kapanır).

## Oturum yaşam döngüsü
- `on_page_load` kancası sayfa yenilemede o pencerenin değil **tüm** oturumları kapatır (yetim oturum önlemi).
  İkinci pencere eklenirse pencere başına filtre gerekir; `qt_open`'ın yenilemeden hemen sonra çağrılması küçük bir
  yarış penceresi taşır.
- Meşgul oturumu `close(Rollback)` zorla kapatır; sink'e "kullanıcı iptal etti" iletisi gider (`connection_lost`
  ayrımı yok). Zorla kapatma oturum başına en kötü **~10 sn** sürer (kilit için 5 sn + `ROLLBACK` için 5 sn): meşgul
  bir sekmede `qt_close(Rollback)` ~10 sn bekleyebilir; `close_sessions_of` oturumları sırayla kapatır (en kötü
  N x ~10 sn). Meşgul oturumda `Commit` `busy`, `Refuse` açık transaction varsa `open_transaction` döner.
- Çıkışta `close_all` (oturumlar paralel; `main.rs` en çok 7 sn bekler, kalan bağlantılar süreçle kapanır). `main.rs` boşta oturumları 30 dk sonra uyutur (`reap_idle`);
  sonraki ifade oturumu sessizce yeniden açar.
- **Durumlu (dirty) oturum**: `Select` dışında herhangi bir ifade (SET, geçici tablo, DO/CALL,
  DDL, transaction komutu…) ve oturum durumu oluşturan `SELECT` çağrıları (oturum düzeyi `pg_advisory_lock`,
  `pg_try_advisory_lock`, `_shared` varyantları ve `set_config`; `pg_advisory_xact_lock` gibi `_xact_` varyantları
  transaction ile biter, gerekmez) başarıyla bitince oturum durumlu sayılır. Durumlu oturum boşta **uyutulmaz**; bağlantısı
  koparsa (sunucu yeniden başladı, ağ, `pg_terminate_backend`) ilk çalıştırma bir kez `session_reset` döner ve yeni
  bağlantı hazır bekler. Durumsuz oturum sessizce yeniden bağlanır. Açık transaction kaybolduysa `query` kodlu
  "bağlantı koptu; açık transaction geri alındı" hatası döner.
- Kilitsiz oturumlar her zaman `default_transaction_read_only=off` ve seçilen `statement_timeout` (varsayılan 0) ile
  açılır; bu, rol/veritabanı düzeyinde DBA'nın koyduğu `ALTER ROLE … SET default_transaction_read_only = on` ya da
  rol zaman aşımını oturum için ezer (spec gereği; politika kararı Plan 6+).

## Diğer
- Tamamlama oturum meşgulken ya da uyurken (idle reap) **boş liste** döner; beklemez. Ama tamamlama sürerken
  (en çok 5 sn) oturum kilidini tutar: bu sırada başlatılan `qt_run` `busy` alır (arayüz tamamlamayı yazarken
  iptal etmeli / kısa tutmalı).
- `CompletionKind::Function` hiçbir zaman üretilmez (fonksiyon tamamlaması yok); tür ileride kullanılmak üzere durur.
- `qt_set_options` açık ızgara imlecini kapatır: sonrasında o ızgarada `more()` hata verir (arayüz "daha fazla"
  düğmesini kapatmalı).
- CSV dışa aktarma ayrı bir imleç kullanır (ızgara imlecine/manuel transaction'a dokunmaz); yalnızca tek
  SELECT/VALUES/TABLE/WITH ifadesi.
- CSV hedefi **yeniden adlandırmayla üzerine yazılır** (`.tmp` -> hedef): arayüz var olan dosyanın üzerine yazmayı
  onaylatmalıdır (spec §169). Hedef Excel'de açık/kilitliyse hata ancak **tüm dışa aktarma bittikten sonra**
  (yeniden adlandırmada) görülür; önceden yazılabilirlik denemesi yok.
- İptal iletileri: kullanıcı iptalinde arka uç 57014'ü `Sorgu kullanıcı tarafından iptal edildi.` iletisine
  (`code: "query"`) çevirir; `statement_timeout` zaman aşımı kendi "zaman aşımı" iletisiyle gelir. İkisi aynı
  kodu taşır; ayrım iletidendir (ayrı kod yok) — gerekirse Plan 6'da ayrı kod eklenebilir.
- `smoke-release.ps1` `qt_*` komutlarını çağırmaz (yalnızca açılış); Plan 6 arayüzü gelince uçtan uca duman testi eklenmeli.
- NOTICE'lar ifade başına en çok 100 adet / 2.000 karakterle sınırlanır.
- **Hücre başına bayt sınırı yok**: bellek yalnızca satır sayısıyla sınırlıdır (limit/parça). 1 GB'lık bir `bytea`/`text`
  hücresi bütün olarak bellekte tutulur ve `Rows` olayıyla gönderilir. Plan 6 arayüzü hücre gösterimini kırpmalı;
  hücre/parça başına bayt sınırı olası bir izleme işidir.
