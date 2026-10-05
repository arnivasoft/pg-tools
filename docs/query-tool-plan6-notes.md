# Query Tool Plan 6 notları (arka uç devir listesi)

Plan 5 arka ucundan (`pgt-query`, `src-tauri/src/query.rs`) arayüze (Plan 6) kalan sözleşmeler ve bilinen
sınırlar. Ayrıntılar için ilgili kod yorumlarına bakın.

## Olay akışı
- Arayüz bir `qt_run`'ı **`ScriptDone` olayıyla** bitirmelidir; `Channel` iletileri `invoke` çözüldükten sonra da
  gelebilir. `invoke`'un çözülmesi "bitti" demek değildir.
- Olay sırası ifade başına: `started` -> `rows`* -> `done`, ya da herhangi bir noktada `failed`; betik sonunda
  `scriptDone { stmtsRun, stoppedOnError }`. `Failed` NOTICE taşımaz.
- `more()` ifadeyi `stmt_index` ile tanımlar (çalıştırma kimliği yok): yeni bir çalıştırma başladıktan sonra
  eski ızgaradan `more` istenmemeli.
- `truncated` imleç yolunda sonuç tam `limit` satırsa da `true` olabilir.

## Konumlar
- `charOffset` ve `Failed.position` **Unicode karakter** (scalar) ofsetidir; CodeMirror UTF-16 kod birimi kullanır.
  Astral karakterler (emoji vb.) için arayüz dönüşümü yapmalıdır. `position` ifade metnine göredir; betikteki yeri
  `charOffset + position - 1`.

## Hata kodları (`ErrorPayload.code`; metinle eşleştirmeyin)
Tam liste (`QUERY_ERROR_CODES` / `QueryErrorCode.ts`): `confirmation_required`, `locked`, `busy`, `tx_failed`,
`open_transaction` (tür şu an kullanılmıyor -> Plan 6'da tür koruyucusu ekleyin). Diğer hatalar genel kodlarla gelir (`query`, `not_found`, `not_connected`,
`invalid`, ...). `qt_complete` 5 sn'lik zaman aşımına uğrarsa `invalid` kodu ile görünür.

## Onay akışı
- Onay arka uçta zorunludur: `qt_run` `confirmed=false` ile tehlikeli/üretim-yazan betikte `confirmation_required`
  döner; arayüz onay diyaloğunu gösterip `confirmed=true` ile yeniden çağırır. `qt_analyze` hangi ifadelerin neden
  onay istediğini söyler (diyalog metni için).
- Kilitli sunucuda yazma `locked` ile reddedilir; kilit **güvenlik sınırı değildir** (bkz. `docs/testing.md`).
  Kilitli oturumlar **her** `set_config(` çağrısını reddeder (belgelenmiş yanlış pozitif; `set_config('x','y',false)`
  gibi zararsız çağrılar da engellenir).
- Sunucu kilidi veya üretim etiketi değişince oturumlar kapatılmalıdır (`close_sessions_of`); bayraklar oturum
  açılırken yakalanır.

## Transaction durumu
- `Failed` durumunda yalnızca `ROLLBACK` / `ROLLBACK TO` kabul edilir; `COMMIT` reddedilir (`tx_failed`).
- Manuel kipte `SAVEPOINT` bile otomatik `BEGIN` alır. Durum sondayla düzeltilir (`docs/testing.md` sınırları).
- Autocommit ızgara imleci açıkken transaction açık kalır (5 dk boşta kapanır).

## Oturum yaşam döngüsü
- `on_page_load` kancası sayfa yenilemede o pencerenin değil **tüm** oturumları kapatır (yetim oturum önlemi).
  İkinci pencere eklenirse pencere başına filtre gerekir; `qt_open`'ın yenilemeden hemen sonra çağrılması küçük bir
  yarış penceresi taşır.
- Meşgul oturumu `close(Rollback)` zorla kapatır; sink'e "kullanıcı iptal etti" iletisi gider (`connection_lost`
  ayrımı yok). `close_sessions_of` oturumları sırayla kapatır (en kötü N x 5 sn).
- Çıkışta `close_all` (<= 7 sn). `main.rs` boşta oturumları 30 dk sonra uyutur (`reap_idle`); sonraki ifade oturumu yeniden açar.

## Diğer
- Tamamlama oturum meşgulken ya da uyurken (idle reap) **boş liste** döner; beklemez.
- CSV dışa aktarma ayrı bir imleç kullanır (ızgara imlecine/manuel transaction'a dokunmaz); yalnızca tek
  SELECT/VALUES/TABLE/WITH ifadesi.
- İptal iletileri: kullanıcı iptalinde arka uç 57014'ü `Sorgu kullanıcı tarafından iptal edildi.` iletisine
  (`code: "query"`) çevirir; `statement_timeout` zaman aşımı kendi "zaman aşımı" iletisiyle gelir. İkisi aynı
  kodu taşır; ayrım iletidendir (ayrı kod yok) — gerekirse Plan 6'da ayrı kod eklenebilir.
- `smoke-release.ps1` `qt_*` komutlarını çağırmaz (yalnızca açılış); Plan 6 arayüzü gelince uçtan uca duman testi eklenmeli.
- NOTICE'lar ifade başına en çok 100 adet / 2.000 karakterle sınırlanır.
- **Hücre başına bayt sınırı yok**: bellek yalnızca satır sayısıyla sınırlıdır (limit/parça). 1 GB'lık bir `bytea`/`text`
  hücresi bütün olarak bellekte tutulur ve `Rows` olayıyla gönderilir. Plan 6 arayüzü hücre gösterimini kırpmalı;
  hücre/parça başına bayt sınırı olası bir izleme işidir.
