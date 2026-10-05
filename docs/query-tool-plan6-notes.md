# Query Tool Plan 6 notları (arka uç devir listesi)

Plan 5 arka ucundan (`pgt-query`, `src-tauri/src/query.rs`) arayüze (Plan 6) kalan sözleşmeler ve bilinen
sınırlar. Ayrıntılar için ilgili kod yorumlarına bakın.

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
- `qt_complete` hata vermez: 5 sn'lik zaman aşımı, sorgu hatası, meşgul ya da uyuyan oturum **boş liste** döner.
- `qt_open`: bağlanırken sunucunun kilidi / üretim etiketi değiştiyse `invalid` ("… bağlanırken değişti; sekmeyi
  yeniden açın."), sunucu silindiyse `unknown_server`; bağlantı bırakılır, oturum açılmaz.
- `session_reset`: durumlu bir oturumun (bkz. Oturum yaşam döngüsü) bağlantısı koptu ve yeniden kuruldu; `SET`
  ayarları, geçici tablolar, advisory kilitler kaybolmuş olabilir. Yalnızca bir kez döner, hiçbir ifade çalışmamıştır;
  yeni bağlantı hazırdır, kullanıcı sorguyu tekrar çalıştırabilir.

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
- **Durumlu (dirty) oturum**: `Select` dışında herhangi bir ifade (SET, geçici tablo, advisory kilit, DO/CALL,
  DDL, transaction komutu…) başarıyla bitince oturum durumlu sayılır. Durumlu oturum boşta **uyutulmaz**; bağlantısı
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
