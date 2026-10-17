# Test rehberi

## Birim testleri
    cargo test --workspace            # PGT_TEST_URLS yoksa entegrasyon testleri atlanır
    bun run test
    bun run typecheck

## Entegrasyon testleri (gerçek PostgreSQL)
`PGT_TEST_URLS` virgülle ayrılmış `postgres://kullanici:parola@host:port/postgres` adresleri içerir; hem
`pgt-catalog` hem `pgt-core` entegrasyon testleri listedeki her sunucuda ayrı ayrı çalışır.
Test kullanıcısı `CREATEDB` ve `CREATEROLE` yetkisine sahip olmalı (superuser yeterli); sunucuda
`postgresql-contrib` (pg_trgm) kurulu olmalı; kimlik doğrulama scram-sha-256.

    $env:PGT_TEST_URLS = "postgres://postgres:PAROLA@127.0.0.1:5432/postgres"
    cargo test --workspace --all-features

Testler `pgt_fixture`, `pgt_roundtrip`, `pgt_perf`, `pgt_noconn`, `pgt_private` adlı veritabanlarını
ve `pgt_reader`, `pgt_app`, `pgt_limited`, `pgt_forever` rollerini oluşturur/yeniden kurar; üretim sunucusunda çalıştırmayın.

Fixture: `tests/fixtures/schema.sql` her sürümde, `schema_pg11.sql` (procedure, partitioned index, `INCLUDE`,
hash partition, partitioned tabloda satır trigger'ı) PG 11+ ve `schema_pg12.sql` (generated kolon) PG 12+
sunucularda uygulanır. Sürüme bağlı test adımları `fx.major().await >= N` ile korunur.

## Sürüm matrisi (Docker)
    ./scripts/test-matrix.ps1                       # PG 10, 11, 12, 13, 14, 15, 16, 17, 18 (9 sürüm)
    ./scripts/test-matrix.ps1 -Versions 16,18
    ./scripts/test-matrix.ps1 -KeepContainers       # hata ayıklama için kapsayıcıları bırakır
    ./scripts/test-matrix.ps1 -SkipPerf             # 5.000 tablolu performans testini atlar

Betik her sürüm için `postgres:NN` imajından `pgt-matrix-NN` kapsayıcısını `127.0.0.1:(55400+NN)` portunda
başlatır, her çalıştırmada rastgele üretilen (hiçbir yere yazılmayan) bir parolayla `PGT_TEST_URLS`'i ayarlar,
`cargo test --workspace --all-features` çalıştırır ve kapsayıcıları siler. Docker Desktop açık olmalıdır.
Portlar 55410–55418'dir (sürüm başına bir port, çakışma yok). `powershell -File scripts/test-matrix.ps1 -Versions 13,15`
biçiminde dışarıdan çağrıldığında liste tek dize olarak gelir; betik virgüllerden böler.

Matris süresinin büyük kısmını tek başına performans testi (`five_thousand_tables_stay_within_budget`) oluşturur:
her sunucuda 5.000 tablo kurar (7 sürümlük matriste ~215 s, 9 sürümde daha uzun; geri kalan tüm testler toplamda ~45 s). Hızlı yineleme için
`-SkipPerf` bu testi `cargo test ... -- --skip five_thousand_tables_stay_within_budget` ile atlar; varsayılan
davranış (test çalışır) değişmez. Birleştirmeden önceki doğrulama `-SkipPerf` olmadan yapılmalıdır.

**PG 10–11 desteği bu matrisle gerçek oldu.** Matrisin ilk çalıştırmasında, `name` tipli katalog kolonlarına
uygulanan `ORDER BY ... COLLATE "C"` ifadelerinin PG 10–11'de hata verdiği görüldü (`collations are not supported
by type name`; `name` PG 12'den önce collatable değildir). Bu yalnızca Plan 2 sorgularını değil, Plan 1'in üretim
SQL'ini de etkiliyordu: `databases.sql`, `schemas.sql`, `tables.sql`, `acl_database.sql`, `acl_namespace.sql`,
`acl_relation.sql` — yani ağacın ilk düzeyleri (veritabanları, şemalar, tablolar) ve Database/Schema/Table
DDL'i PG 10–11 sunucularda hiç çalışmıyordu. Düzeltme (`X::text COLLATE "C"`) tüm sürümlerde aynı C sıralamasını
verir. Bu tarihten önceki "PG 10+ destekli" ifadeleri yalnızca PG 12+ için doğrulanmış sayılmalıdır.

Sürüme özel DDL snapshot'ları `tests/snapshots/*@pgNN.snap` olarak tutulur. Yeni bir snapshot gerektiğinde
`INSTA_UPDATE=new` ile üretilip PG 18 ile karşılaştırılarak gözden geçirilir. Kabul edilen farklar yalnızca
sunucunun kendi deparse çıktısından gelir: `POSITION(... IN ...)` / `"position"(...)`, `EXECUTE FUNCTION` /
`EXECUTE PROCEDURE` (PG < 12 deparse'ı), view gövdesinde tablo adıyla nitelenmiş kolonlar (PG < 16), `MAINTAIN`
yetkisi (PG 17+), pg_trgm sürümü (PG 10: 1.3, PG 11–12: 1.4, PG 13: 1.5, PG 14+: 1.6) ve PG 10'da olmayan nesneler
(partitioned index, `INCLUDE`). PG 11 snapshot'ları sürüm sınırını doğrular: generated kolon / `tgparentid`
öncesi yollar PG 10 ile, procedure / `INCLUDE` / hash partition / partitioned index yolları sonraki sürümlerle
aynı çıktıyı verir. PG 13 ve 15 snapshot'ları Plan 3'te eklendi: PG 13, PG 14'ten yalnızca `"position"(...)`
deparse'ı (2 snapshot) ve pg_trgm 1.5 ile; PG 15, PG 16'dan yalnızca matview gövdesindeki tablo adıyla nitelenmiş
kolonlarla ayrılır (PG 15 = PG 14 birebir). Sürüm kapılamasında (`.pgNN.sql` / `VersionedSql`) değişiklik gerekmedi.

Bir test her sunucuda döngüyle çalıştığı için ilk başarısız sunucuda durur; snapshot üretirken tüm sürümlerin
`.snap.new` dosyalarını tek seferde almak için `INSTA_FORCE_PASS=1` de verilebilir (yalnızca snapshot üretimi
sırasında; doğrulama çalıştırması bu değişken olmadan yapılmalıdır).

## Query Tool arka ucu (`pgt-query`)
Testler `crates/pgt-query/tests/` altındadır ve gerçek PostgreSQL ister (`PGT_TEST_URLS` yoksa atlanır; her test
listedeki her sunucuda ayrı çalışır). Birim testleri (`lex`, `split`, `classify`, `error_codes`) sunucusuz çalışır.

| Dosya | Kapsam |
|---|---|
| `exec.rs` | `Executor`: imleç/doğrudan yol, satır sınırı, `more()`, hata konumu, NOTICE, zaman aşımı |
| `session.rs` | `QueryManager`: oturum yaşam döngüsü, transaction durum makinesi, kilit/onay kapıları, iptal, boşta imleç |
| `complete.rs` | Katalog tamamlama sorguları, önbellek, 500 sonuç sınırı |
| `export.rs` | CSV dışa aktarma: akış, biçim, hata/iptal temizliği, manuel transaction korunması |
| `memory.rs` | Bellek sözleşmesi (1M satır): sayaçlar ve olay boyutları; yalnızca sayaç assert edilir, süre asla |

Docker'lı tek sunucuyla çalıştırma (kapsayıcıyı işiniz bitince silin; URL'yi/parolayı çıktıya yazmayın):

    docker run -d --name pgt-q -e POSTGRES_PASSWORD=PAROLA -e POSTGRES_INITDB_ARGS=--auth-host=scram-sha-256 `
      -e POSTGRES_HOST_AUTH_METHOD=scram-sha-256 -p 127.0.0.1:55496:5432 postgres:18
    $env:PGT_TEST_URLS = "postgres://postgres:PAROLA@127.0.0.1:55496/postgres"
    cargo test -p pgt-query                          # ya da --workspace --all-features
    docker rm -f pgt-q

Tüm sürümler için `scripts/test-matrix.ps1` (yukarıda) `pgt-query` testlerini de PG 10–18'de koşar.

`PGT_QUERY_BIG=1`: `memory.rs` içindeki ağır/bilgi amaçlı testleri açar — 1M satırlık CSV dışa aktarma
(`csv_export_streams_one_million_rows`: ilerleme monoton, tampon <= `EXPORT_CHUNK`) ve tamamlama/iptal gecikmesi
ölçümü (`measure_complete_and_cancel_latency`, `-- --nocapture` ile `MEASURE …` satırlarını yazar). Bu değişken yokken
`row_budget_holds_for_one_million_rows` (10.000 satır sınırı, 1.000'lik parçalar, `more()` x3, kapatınca
`idle in transaction` kalmaması) yine de çalışır. Süre assert edilmez; ölçümler `docs/performance.md`'dedir.

### Query Tool güvenlik ve durum sınırları
- **Salt okunur kilit bir güvenlik sınırı DEĞİLDİR.** Güçlü bir korumadır; kesin garanti için sunucuda **salt okunur
  bir veritabanı rolü kullanın.** Kilitli oturumda istemci şunları reddeder (`classify.rs` `changes_read_only`,
  testler `classify.rs` ve `session.rs`): `Write`/`Ddl` sınıfı ifadeler; `transaction_read_only` /
  `default_transaction_read_only` içeren `SET`/`RESET`/`DISCARD`/`set_config`;
  `BEGIN`/`START TRANSACTION`/`SET TRANSACTION`/`SET SESSION CHARACTERISTICS AS TRANSACTION ... READ WRITE` (yalnızca
  `READ WRITE` biçimi; `READ ONLY` serbest); `RESET ALL` ve `DISCARD ALL`; **her** `set_config` sözcüğü (hesaplanmış
  ad dahil, `DO` gövdesindeki dolar tırnaklı metin dahil); ve **her** `U&"…"` / `U&'…'` Unicode kaçışlı ad ya da dize
  (`SET U&"default_transaction_read_onl\0079" = off` metinsel taramayı atlatırdı). CSV dışa aktarma da aynı kapıdan
  geçer (`select set_config(...)` bir SELECT'tir). Yanlış pozitifler kabul edilir: dize/dolar tırnaklı metin
  içindeki `read write`, `reset all`, `discard all` ifadeleri de (ör. `select 'read write'`) reddedilir. Oturum varsayılanı `default_transaction_read_only=on`'dır, ama bu yalnızca bir **varsayılandır**
  (kullanıcı `SET`/`BEGIN READ WRITE` ile değiştirebilir; istemci bunları yakalar, sunucu yakalamaz).
  Hâlâ atlatabilecekler: `DO`/fonksiyon içinde `EXECUTE` ile ayar adını dize parçalarından kurmak
  (`'SET default_transaction_' || 'read_only = off'`: metinsel tarama parçalanmış adı görmez; bu yol
  `tests/session.rs` `lock_is_a_guard_not_a_security_boundary` ile **doğrulanmıştır**: ardından bir `DO` ile yazma
  başarılı olur); gövdesinde ayarı değiştiren veya yazan, `SELECT fonksiyon()` ile çağrılan fonksiyonlar
  (özellikle `SECURITY DEFINER`; sınıflandırıcı yalnızca çağrı ifadesini görür, `set_config` sözcüğü fonksiyonun
  içinde kalır); sınıflandırıcının tanımadığı yazan ifade biçimleri.
- **Onay kapısı arka uçta zorunludur**: tehlikeli ifadeler (`DROP`, `TRUNCATE`, `WHERE`'siz `UPDATE`/`DELETE` vb.)
  ve üretim etiketli sunucuda yazan ifadeler `confirmed = true` olmadan `qt_run` tarafından reddedilir
  (`confirmation_required`); arayüz yalnızca onay diyaloğunu gösterir, kuralı uygulamaz.
- **Transaction durum makinesi** istemci tarafında ifade türlerinden izlenir ve **`Tx` sınıfı her ifadeden ve her
  hatadan sonra** (ayrıca `qt_tx`, başarısız kapanış-COMMIT'i ve dışa aktarma hatasından sonra; başarılı `Select`/
  `Write`/`Ddl` ifadelerinden sonra değil) sunucuya `SELECT statement_timestamp() <> transaction_timestamp()`
  sondasıyla doğrulanır (sapma düzeltilir ve loglanır).
  Sınırları: fonksiyon/`DO` içinde dinamik `COMMIT`/`ROLLBACK` istemciye görünmez (sonda yine de düzeltir);
  sonda saatin ilerlemesine dayanır (transaction içinde ilk ifadede iki zaman damgası eşit olabilir, saat geri
  alınırsa yanlış sonuç mümkündür). `Failed` durumunda yalnızca `ROLLBACK`/`ROLLBACK TO` kabul edilir
  (`COMMIT` reddedilir).
- **İmleç ve `idle in transaction`**: autocommit kipinde ızgara imleci açıkken (daha fazla satır alınabilir) sunucuda
  transaction açık kalır (`idle in transaction`); imleç tükenince, yeni ifadede, `close`'ta ya da **5 dakika** boşta
  kalınca (`DEFAULT_CURSOR_IDLE`) kapatılır. Sunucuda `idle_in_transaction_session_timeout` 5 dakikadan kısaysa
  sunucu önce bağlantıyı keser. CSV dışa aktarma ayrı bir imleç kullanır (ızgara imlecini etkilemez).
- **Bellek sözleşmesi yalnızca satır sayısını sınırlar**: hücre/satır başına bayt sınırı yoktur. 1 GB'lık bir
  `bytea`/`text` hücresi bellekte tutulur ve olayla bütün olarak gönderilir; `memory.rs` yalnızca satır sayaçlarını
  doğrular. Plan 6 arayüzü gösterimi kırpmalı; hücre/parça başına bayt sınırı olası bir izleme işidir.

## pgbouncer ve `options` yedek yolu
Oturumun salt okunur/zaman aşımı/`search_path` ayarları normalde `options` başlangıç parametresiyle gönderilir.
pgbouncer gibi havuzlayıcılar bunu `unsupported startup parameter: options` (SQLSTATE 08P01) ile reddeder; bu durumda
bağlantı kurucu bir kez `options`'sız yeniden bağlanır ve aynı üç ayarı (`SESSION_SETS`) bağlantıdan hemen sonra `SET`
ile uygular (tek `batch_execute`; başarısız olursa bağlantı kapatılır). Başka hatalar (parola, ağ, TLS, eksik
veritabanı) yeniden denemeyi tetiklemez. Sınır: `SET` oturum düzeyindedir; **transaction pooling** modunda sunucu
bağlantısı işlemler arasında değişebilir ve ayarlar sonraki işlemlere taşınmayabilir. Salt okunurluk güvencesi için
session pooling (ya da doğrudan bağlantı) kullanın. `options`'ı reddetmek yerine sessizce yok sayan kurulumlar
(pgbouncer `ignore_startup_parameters = extra_float_digits,options`) için her bağlantıda sürüm sorgusu üç ayarı da
okur; biri eksikse aynı `SESSION_SETS` aynı istemcide uygulanır (ek gidiş-dönüş yalnızca o durumda). Yedek yol ve bu
denetim `connect_once(.., with_options=false)` testiyle (`session_settings_are_applied_without_options_parameter`) her
sürümde doğrulanır; `SET` hatasının bağlantı hatası olarak döndüğü `failing_session_set_is_a_connect_error`, tek yeniden
deneme ise sahte bir havuzlayıcıyla (`options_rejection_retries_once_without_options`, `other_startup_errors_do_not_retry`)
sınanır; gerçek pgbouncer gerekmez.

## TLS zincir testi (Docker)
    powershell -NoProfile -File scripts/tls-test.ps1              # postgres:16
    powershell -NoProfile -File scripts/tls-test.ps1 -Version 18
    powershell -NoProfile -File scripts/tls-test.ps1 -KeepContainer   # sunucu kapsayıcılarını bırakır

Betik bir yardımcı kapsayıcıda `openssl` ile bir test CA'sı, bu CA'dan iki sunucu sertifikası ve bağımsız ikinci bir
CA üretir; sertifikaları geçici bir dizine alır ve yalnızca `hostssl all all all scram-sha-256` kabul eden
(`ssl=on`, anahtar `postgres` sahipli ve `600`) iki PostgreSQL açar:

| kapsayıcı | port | sertifika SAN'ı |
|---|---|---|
| `pgt-tls-a` | `127.0.0.1:55480` | yalnızca `DNS:pgt-tls.test` (IP yok) |
| `pgt-tls-b` | `127.0.0.1:55481` | `DNS:pgt-tls.test`, `IP:127.0.0.1` |

Ardından `cargo test -p pgt-core --test tls` şu ortam değişkenleriyle çalışır: `PGT_TLS_URL` / `PGT_TLS_IP_URL`
(`host=127.0.0.1 port=… user=postgres`; parola içermez), `PGT_TLS_PASSWORD` (her çalıştırmada rastgele),
`PGT_TLS_CA`, `PGT_TLS_OTHER_CA`. Değişkenler yoksa testler atlanır. Kapsayıcılar ve geçici dizin (CA'lar ve özel
anahtarlar dahil) hata olsa da silinir; anahtarlar depoya hiç girmez. `-KeepContainer` ile yalnızca geçici dizin
silinir: sunucu kapsayıcıları kalır ve içlerinde sunucu özel anahtarları ile o çalıştırmanın parolası (kapsayıcı
ortamında `POSTGRES_PASSWORD`) durur; işiniz bitince `docker rm -f pgt-tls-a pgt-tls-b` ile silin. Bağlantı her
zaman `127.0.0.1` iledir:

| mod | sunucu | root_cert | beklenen |
|---|---|---|---|
| `disable` | A | – | hata, SQLSTATE 28000 (pg_hba yalnızca hostssl) |
| `prefer` | A | – | başarılı (TLS kurulur) |
| `require` | A | – | başarılı (sertifika doğrulanmaz) |
| `verify-ca` | A | doğru CA | başarılı (host adı uyuşmazlığı yok sayılır) |
| `verify-ca` | A | başka CA | hata, `kind = Tls` (`UnknownIssuer`) |
| `verify-ca` | A | yok (Windows deposu) | hata, `kind = Tls` (`UnknownIssuer`) |
| `verify-full` | A | doğru CA | hata, `kind = Tls` (127.0.0.1 SAN'da yok) |
| `verify-full` | B | doğru CA | başarılı (zincir + IP SAN) |
| `verify-full` | B | başka CA | hata, `kind = Tls` |

`verify-full` + `host=pgt-tls.test` (hosts girdisi olmadan, `hostaddr` ile) sınanmaz: `ServerConfig` yalnızca
`host` taşır ve bağlantı kurucu `hostaddr` ayarlamaz. Olumlu `verify-full` yolu bunun yerine IP SAN'lı B sunucusuyla
sınanır. İstemci sertifikası senaryosu bu testin kapsamı dışındadır.

## Performans testi
`cargo test -p pgt-catalog --test perf -- --nocapture` — 5.000 tablolu şemada liste/Properties/DDL süreleri
(spec §1.3 bütçeleri). Yavaş makinede `PGT_PERF_FACTOR=2` ile bütçeler gevşetilebilir (CI'da kullanmayın).

Bu test yalnızca loopback (aynı makine / Docker) üzerinde bir **gerileme bekçisidir**: ağ gecikmesi ~0 olduğu için
LAN/WAN gecikmesindeki davranışı ve sorguların `try_join` ile boru hattına alınmasından beklenen kazancı
doğrulayamaz. Uzak sunucu performansı ayrıca elle ölçülmelidir.

## DDL gidiş-dönüş testi

`crates/pgt-catalog/tests/roundtrip.rs` fixture veritabanındaki her nesnenin DDL'ini (`Catalog::ddl`) boş bir
`pgt_roundtrip` veritabanına uygular ve iki veritabanının katalog imzasını
(`crates/pgt-catalog/tests/fixtures/signature.sql`) karşılaştırır. Çalıştırma:

```bash
PGT_TEST_URLS="postgres://..." cargo test -p pgt-catalog --test roundtrip
```

**İmzanın kapsadıkları** (`app` ve `extra` şemaları):

- şemalar: sahip, ACL, yorum
- ilişkiler (tablo, partitioned tablo, view, matview, sequence): tür, kalıcılık (UNLOGGED), `reloptions`, RLS/FORCE RLS,
  sahip, ACL, yorum, partition anahtarı ve sınırı, INHERITS ebeveynleri, view tanımı, matview `relispopulated`
- kolonlar: tip, NOT NULL, varsayılan/üretilmiş ifade, identity, collation, kolon ACL'i, yorum
- kısıtlar (p/u/c/x/f): tanım, `convalidated`, yorum
- index'ler: tanım, yorum
- trigger'lar: tanım, etkinlik durumu (`tgenabled`), yorum
- policy'ler: komut, permissive/restrictive, roller, USING/WITH CHECK, yorum
- sequence'ler: tip, start/increment/min/max/cache/cycle ve sahiplik (`OWNED BY` / identity bağı)
- fonksiyon/prosedürler: `pg_get_functiondef`, ACL, sahip, yorum
- tipler: enum etiketleri, composite alanları, range alt tipi, domain tanımı/kısıtları, sahip, ACL, yorum
- extension'lar: ad, şema, sürüm

**Bilinçli olarak kapsam dışı:**

- extension üyeleri (extension'ın kendisi kurar)
- roller ve tablespace'ler (küme genelidir; aynı sunucuda iki veritabanı karşılaştırıldığı için zaten ortaktır)
- rule'lar, aggregate'ler, cast'ler, operatörler, istatistik nesneleri (bu türler için henüz DDL üretilmiyor)
- kolon düzeyinde storage, compression ve statistics target; tablo access method; replica identity
- partition'larda kolon düzeyi geçersiz kılmalar (varsayılan, NOT NULL vb.): partition DDL'i `PARTITION OF` biçimindedir
- base (C) tipler; range tiplerinde `SUBTYPE` dışındaki seçenekler

PG 10–18 (9 sürüm) üzerinde sürüm matrisiyle doğrulanır (aşağıya bakın). PG 10–11'de
`pg_attribute.attgenerated` olmadığı için imzanın `fixtures/signature.pg10.sql` varyantı kullanılır (generated
kolon alanı boş yazılır); geri kalanı `signature.sql` ile aynıdır.

## Bilinen DDL sınırları
- base type DDL'i desteklenmez; range type yalnızca `SUBTYPE` yazar.
- PG 18 adlandırılmış NOT NULL kısıt adları korunmaz.
- kolon `STORAGE`/`COMPRESSION` yazılmaz; GRANT veren (grantor) bilgisi kullanılmaz.
- partition DDL'i `CREATE TABLE ... PARTITION OF` biçimindedir; partition'daki kolon düzeyi geçersiz kılmalar
  (kolon varsayılanı, ek `NOT NULL` vb.) yazılmaz, yalnızca partition'a ait kısıtlar yazılır.
- partitioned tablodaki satır trigger'ları partition'lara klonlanır. PG 11–12'de klonlar `tgisinternal` olarak
  işaretlidir (partition'ın Triggers listesinde görünmez, DDL'de yazılmaz); PG 13+'da klonlar partition'ın
  Triggers listesinde görünür ama `tgparentid <> 0` olduğundan partition DDL'inde tekrar yazılmaz. (Plan
  taslağında PG 11–12'de tekrar yazıldığı varsayılmıştı; PG 11 ve 12 üzerinde canlı doğrulamada böyle olmadığı görüldü
  ve `extra.hits` fixture'ı + gidiş-dönüş testi bunu her sürümde korur.)

## Filtre performans testi
`src/features/explorer/tree-model.test.ts` içindeki filtre testi 10.000 yüklenmiş düğümlü bir ağaçta filtreyi
5 kez uygular (filtre `flatten` sırasında uygulanır) ve en kısa süreyi `filter 10k min ms: …` olarak yazdırır;
bütçe **< 50 ms** (2026-10-05 ölçümü 0,93 ms; `docs/performance.md`). `bun run test`
ile birlikte çalışır; ölçülen değeri görmek için
`npx vitest run src/features/explorer/tree-model.test.ts -t "10,000" --silent=false`.

## Query Tool ön yüzü (Plan 6)

Ön yüz mantığı DOM'dan ayrı, saf modellerde tutulur ve `bun run test` (vitest) ile sınanır; CodeMirror/DOM'a bağlı
kısım (`QueryTab.tsx`, `ResultGrid.tsx`, `editor.ts`) ince tutulur ve elle/CDP ile doğrulanır
(`docs/smoke-test.md` "Query Tool"). `src/features/query/*.test.ts`:

| Dosya | Kapsam |
|---|---|
| `offsets.test.ts` | UTF-16 (CodeMirror) ↔ Unicode scalar (arka uç) ofset dönüşümü; emoji/vekil çiftleri, sınır dışı kırpma |
| `confirm-model.test.ts` | Onay kararı (`decide`): WHERE'siz UPDATE/DELETE, DROP/TRUNCATE, üretim + yazma/`DO`/`CALL`, kilit; onay diyaloğu metni |
| `run-model.test.ts` | Çalıştırma durum makinesi (`runReduce`): açılış, çalışma, iptal, onay reddi/kilit/boş betik (`runAborted`), `scriptDone` ile bitiş, eski `runId` olayları |
| `result-model.test.ts` | Olay → sonuç (`applyEvent`): ifade başına şerit, satır parçaları, NOTICE, hata konumu, `scriptDone` bitirir, eski `runId` yok sayılır |
| `grid-model.test.ts` | Hücre gösterimi (NULL/boş ayrımı, satır sonu, 1000 karakterde kırpma), sütun genişlikleri (başlık adı, tür etiketi ve hücre kendi yazı tipleriyle ölçülür; kısa içerikte başlık kırpılmaz), seçim/kopyalama biçimi |
| `export.test.ts` | CSV dışa aktarma hedef seçimi (`exportTarget`: tek satır döndüren ifade), dosya adı sadeleştirme, ifade seçimi (`preferredIndex`) |
| `history-model.test.ts` | Geçmiş girdileri (`historyEntriesFor`): hassas ifadeyi dışarıda bırakma, başarısız/iptal işareti, emoji sonrası ofsetler, arama |
| `errors.test.ts` | `isQueryErrorCode` tür koruyucusu ve `hintFor` ipuçları (25001, `tx_failed`, `session_reset`, `locked`) |
| `completion.test.ts` | Tamamlama bağlamı (`contextAt`), `replaceRange`/`matchItems` (açık tırnak, otomatik kapanış tırnağı) |
| `tabs-store.test.ts` | Sekme deposu: Nesne sekmesi, başlık `<sunucu>/<db> #n`, kimlik çakışması, kapatma/seçim, sıra |
| `workspace-model.test.ts` | Çalışma alanı: `openContext`/`initialSql`, ortam istemi, `saveClosesSessions` (arka uçla aynı kural), `closePlan`/`bulkPolicy`, `routeError`, taslak dönüşümü, `swapEditorState`, zamanlayıcı biçimi |
| `check-model.test.ts` | Yazarken denetim (Plan 7): kapı (açık+boşta, Failed/meşgul/oturumsuz değil), 600 ms debounce, sürüm sayacıyla eski sonucun yok sayılması, sırasız sonuçlar, aynı hedefte yeniden istek yok, emojili UTF-16 eşleme, düzenlemede işaret temizleme |
| `format-edit.test.ts` | Biçimlendir komutu (Plan 7): aralık (seçim/tümü), seçim boşluklarının korunması, dize/yorum/tırnaklı ad içinde ya da belirteç ortasında başlayan/biten seçimin ifadeye genişletilmesi (`safeFormatSelection`), imleç eşleme, tek geri alma adımı (`EditorState` + `history`) |
| `settings-saver.test.ts` | Editör ayarları kaydı: değer hemen uygulanır, yazımlar sıralı (son yazım kazanır, önceki değişiklik kaybolmaz), başarısız yazım geri alınır |
| `query-controller.test.ts` | Denetleyici (sahte api + sahte zamanlayıcılar): kapanan sekmenin taslağı geri gelmez, sunucusu silinen sekme taslak yazmaz |

Ayrıca `src/features/explorer/actions.test.ts` (menüdeki `openQueryTool` ve kısayol etiketi) ve
`tree-model.test.ts` (sunucu listesi yeniden yüklenince bağlı sunucunun alt ağacı korunur) Plan 6'da genişletildi.

### GUI'nin CDP ile sürülmesi

Elle duman testi ve ölçümler otomatikleştirilmiş bir tarayıcı yerine WebView2'nin uzak hata ayıklama portuyla
(Chrome DevTools Protocol) yapıldı; ek bağımlılık gerekmez (Node ≥ 22'nin yerleşik `WebSocket`'i yeter):

1. Atılabilir PG: `docker run -d --name pgt-qt-ui … -p 127.0.0.1:55499:5432 postgres:18` (iş bitince `docker rm -f`).
2. Gerçek yapılandırmaya dokunmamak için `APPDATA` ve `LOCALAPPDATA` geçici dizinlere yönlendirilir; uygulama
   `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="<tauri.conf.json ayarları> --remote-debugging-port=9224"` ile başlatılır
   (bu değişken `additionalBrowserArgs`'ın yerine geçer: ayarları da yazın, yoksa bellek ölçümü bozulur).
3. `http://127.0.0.1:9224/json` → sayfa hedefinin `webSocketDebuggerUrl`'sine bağlanılır; `Runtime.evaluate`
   (`awaitPromise`, `returnByValue`) ile DOM okunur / `window.__TAURI_INTERNALS__.invoke(...)` ile komut çağrılır,
   `Input.dispatchMouseEvent` / `Input.dispatchKeyEvent` / `Input.insertText` ile gerçek girdi gönderilir (Enter
   tuşu yerine odaktaki düğme tıklanmalı: sentetik `keyDown` Enter bir formu göndermez), `Page.captureScreenshot` ile görüntü alınır.
4. Sunucu kaydı `invoke("save_server", …)` ile eklenir, işin sonunda `delete_server` ile silinir (`servers.json` boş doğrulanır).
   Parolalar betiğin dışına yazılmaz/yazdırılmaz. Yerel Windows kaydet diyaloğu CDP'nin dışındadır: UIAutomation + SendKeys.

Bu yöntemle Task 8'de akışlar (onay, transaction, iptal, CSV, geçmiş, 50.000 satır DOM sayımı, taslaklar, pencere
kapanışı) ve Task 9'da yayın exe'sinde sorgu sekmesi açılış süresi ile bellek (`docs/performance.md`) doğrulandı.

## Yayın exe duman testi
    bun run tauri build --no-bundle                 # dist + ön yüzü gömülü target\release\pg-tools.exe (NSIS yok)
    ./scripts/smoke-release.ps1                     # varsayılan: target\release\pg-tools.exe, 6 sn
    ./scripts/smoke-release.ps1 -Exe C:\yol\pg-tools.exe -Seconds 10

Betik exe'yi geçici, boş `APPDATA`/`LOCALAPPDATA` dizinleriyle başlatır (gerçek yapılandırmaya ve loglara
dokunmaz, sunucuya bağlanmaz, parola kullanmaz), ana pencerenin 10 sn içinde oluşmasını bekler, süreci
`-Seconds` boyunca izler, sonra uygulama + WebView2 süreç ağacını kapatır ve geçici
`%LOCALAPPDATA%\pg-tools\logs` altında `panicked` / `stack overflow` arar. Çıkış kodu 0 = başarılı; ≠ 0 =
exe yok, pencere oluşmadı, süre dolmadan çıktı (çıkış kodu yazdırılır) ya da logda panik var.
Betik önce exe'nin ön yüzü gömüp gömmediğini denetler (exe'de düz metin `assets/index-` varlık anahtarı); yoksa
"BAŞARISIZ: exe ön yüzü gömmüyor (cargo build?)" ile çıkar. Düz `cargo build --release -p pg-tools`, `tauri`
CLI'nin açtığı `custom-protocol` özelliği olmadan derlendiği için `devUrl`'ye (`http://localhost:1420`) yönelir: pencere
bir bağlantı hatası sayfası gösterir ve arayüzün açılışta çağırdığı komutlar (`list_servers`, `list_children`,
`startup_notice`, `qt_drafts_load`, `settings_get` …) hiç çalışmaz — yayın yığın taşması gibi gerilemeler kaçar.
(Doğrulama 2026-10-06: `cargo build` exe'si 14.146.560 bayt, `assets/index-` yok; `bun run tauri build --no-bundle`
exe'si 14.369.792 bayt, CDP hedef URL'si `http://tauri.localhost/`, `assets/index-` var; debug exe'de de yok.)
`scripts/measure.ps1` aynı denetimi yapar.

> **Geçmiş çalıştırmalar için not:** bu denetimden önce belgelenen `cargo build --release` + `smoke-release.ps1`
> çalıştırmaları (Plan 3, Plan 5 sonu ve Plan 6 Task 8 doğrulama/düzeltme turları) ön yüzü gömülü olmayan exe ile
> yapılmış olabilir; bunlar yalnızca açılışta çökme olmadığını gösterir, arayüz komutlarının yayın derlemesinde
> çalıştığını göstermez. Plan 2 (`bun tauri build --no-bundle`) ve Plan 6 Task 9/son düzeltme dalgası `tauri build` exe'siyle yapıldı.

Yalnızca debug derlemede görünmeyen hataları yakalamak içindir: Plan 2'de release exe açılıştan ~1 sn sonra
ana iş parçacığında yığın taşmasıyla (`STATUS_STACK_OVERFLOW`) çöküyordu; varsayılan 6 sn bu tür bir
gerilemeyi yakalayacak kadar uzundur. Her yayından önce çalıştırın.

## pgAdmin içe aktarma sınırı
Dosya önizleme ile içe aktarma arasında değiştirilirse sonuç değişebilir (uygulama aynı yolu yeniden okur; indeksler kayabilir).

Query Tool arayüzü (Plan 6) için arka uç devir notları: `docs/query-tool-plan6-notes.md`.

## Editör zekâsı testleri (Plan 7)

Arka uç testleri `crates/pgt-query` içindedir; DB'li olanlar `PGT_TEST_URLS` ister (yoksa atlanır) ve listedeki her
sunucuda ayrı çalışır, `scripts/test-matrix.ps1` hepsini PG 10–18'de koşar (2026-10-06 çalıştırması: 488 test geçti, 0
başarısız, 1 `ignored` = Windows Credential Manager testi; tek `cargo test` çalıştırması 9 sunucuya birden bağlanır,
sürüm başına ayrı sonuç satırı yoktur — bir test herhangi bir sürümde başarısız olursa çalıştırma düşer).

| Dosya | Kapsam |
|---|---|
| `src/scope.rs` (birim, sunucusuz) | Kapsam çözümleyici: `FROM`/`JOIN` öğeleri, alias'lar, `AS`, çift tırnaklı ad/küçük harfe katlama, CTE (`RECURSIVE`, `MATERIALIZED`, kolon listesi), alt sorgu/`LATERAL`, `UPDATE … FROM`, `DELETE … USING`, `INSERT … RETURNING`, fonksiyon öğeleri, iç içe/korelasyonlu bloklar, bozuk sözdiziminde kısmi sonuç, yorum/dize içindeki sözcükler |
| `tests/complete.rs` | Plan 5 tamamlama + Plan 7 `Scoped`: alias/tırnaklı ad/`search_path` çözümü (`scoped_resolves_aliases_quoted_names_and_search_path`), CTE ve alt sorgu kolonlarının Parse ile (çalıştırmadan) bulunması, transaction içinde Parse'ın transaction'a ve kilitlere dokunmaması, meşgulken boş liste |
| `tests/check.rs` + `src/check.rs` | Hata denetimi: hiçbir şey çalıştırılmaz (`check_executes_nothing`: DDL/DML/`VACUUM`/`SET` Parse'ta yalnızca ham ayrıştırılır, PG 10–18), açık transaction'da durum/kilit/yan etki yok ve `Failed`'da denetim yok, kilitli tabloda 200 ms'de vazgeçme, denetim sürerken başlayan `run` BUSY almaz, meşgulken boş liste, konumlar Unicode scalar + önceki DDL bağlamında isim hataları bilgi düzeyi, uzun betikte görünür pencere (`CheckTarget`) |
| `src/parse_guard.rs` (birim + DB) | Korumalı Parse: zaman aşımına uğrayan adımın iptal edilip sonucunun boşaltılması, bilinmeyen temizliğin yeniden denenmemesi ve kullanıcı savepoint'inin korunması, savepoint hatasında "bozuk" durum, bilinmeyen `SET lock_timeout` sonrası bekleyen geri yükleme (`restore_lock_timeout`) |
| `src/session.rs` (birim + DB) | Plan 7 ekleri: kullanıcı işlemi arka plan işini (tamamlama/denetim) yola getirir (`user_operation_makes_background_work_yield`; `busy` yalnızca başka kullanıcı işlemi kilidi tutuyorsa, en çok 3 sn bekleme), `lock_timeout`'a dokunan ifadelerin tanınması (`lock_timeout_touching_statements`), doğrulanamayan geri yüklemenin bir sonraki kullanıcı SQL'inden önce uygulanması (`unconfirmed_lock_timeout_restore_is_applied_before_the_next_user_sql`) |
| `tests/format.rs` + `tests/format/*.sql` | Biçimlendirici: 35 altın dosya çifti (`NN_ad.sql` → `NN_ad.expected.sql`), seçenek birleşimleri, korunan içerik bayt bayt aynı, `--` yorumu satırı bitirir, dize devamı, virgül/yorum, **fuzz** (belirteç eşitliği + eşkuvvetlilik, rastgele birleştirilmiş parçalar ve karakter çorbası), derin iç içelik taşma yapmaz, 5 MB üstü reddedilir, `ten_thousand_lines_are_fast` (10.000 satır: release < 200 ms, debug < 2 s) |
| `crates/pgt-core` (birim) | `settings_editor_fields_round_trip_and_old_file_loads`: `AppSettings.checkEnabled` / `format` gidiş-dönüşü, eski `settings.json`'ın açılması, geçersiz biçim seçeneğinde yalnızca `format`'ın varsayılana dönmesi |

Altın dosyaları bilerek yeniden üretmek için: `$env:PGT_BLESS = "1"; cargo test -p pgt-query --test format` (çıktıyı gözle
inceleyin; bu değişken yokken testler karşılaştırır). Yalnızca biçimlendirici (sunucusuz, hızlı):
`cargo test -p pgt-query --test format`; saf Rust süresi için `cargo test --release -p pgt-query --test format ten_thousand -- --nocapture`.
Denetim ve kapsamlı tamamlama testleri için Docker'lı tek sunucu (yukarıdaki "Query Tool arka ucu" tarifi):
`cargo test -p pgt-query --test check --test complete`  (oturum birim testleri için `cargo test -p pgt-query --lib`).

Ön yüz (`bun run test`): `check-model.test.ts`, `format-edit.test.ts` (tablo yukarıda) ve genişletilen `completion.test.ts`
(alias bağlamı, `Scoped` isteği, `statementBounds`). Lint katmanı/biçimlendir komutunun DOM kısmı elle/CDP ile
doğrulanır (`docs/smoke-test.md` "Editör zekâsı").

CDP ile Plan 7 ölçümü notları (Plan 6 yönteminin üstüne): CodeMirror'ın görünüm nesnesine `document.querySelector('.cm-content').cmTile.view`
ile erişilir (bu CodeMirror sürümünde `cmView` değil `cmTile`); büyük belgeyi `view.dispatch({changes: …})` ile tek işlemde yerleştirin —
`Input.insertText` çok satırlı metni satır satır işler ve belge boyutunun karesiyle yavaşlar (293 KB için 23 sn). `window.__TAURI_INTERNALS__.invoke`
sarmalanamaz (Tauri'ye yazılamaz görünüyor); IPC sürelerini `window.fetch` (`ipc.localhost/<komut>`) sarmalayarak ölçün. Ctrl+Shift+F için
`Input.dispatchKeyEvent` `modifiers: 10` (Ctrl 2 + Shift 8), `key: "F"`. Ölçüm betikleri depoda değildir.

## Veri düzenleyici testleri (Plan 16)

Arka uç `crates/pgt-ddl/src/data` altındadır; saf testler (insta anlık görüntüleri: `src/data/snapshots`) sunucusuzdur, Docker testleri `PGT_TEST_URLS` ister ve `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` ile PG 10–18'de koşar (sürüme bağlı adımlar atlanır: üretilmiş kolon PG 12+, bölümlü PK PG 11+).

| Dosya | Kapsam |
|---|---|
| `crates/pgt-ddl/src/data/{plan,select,source,model}.rs` (birim) | Plan üretici (INSERT/UPDATE/DELETE, DEFAULT/NULL, hostile değerler, risk), SELECT üretici, anahtar seçimi, model JSON/özel `Debug` |
| `crates/pgt-ddl/tests/data_roundtrip.rs` | Kaynak yükleme (PK/benzersiz/anahtarsız/bölümlü/üretilmiş/kimlik/yetkisiz rol) ve tür gidiş-dönüşleri |
| `crates/pgt-ddl/tests/data_manager.rs` | `DataManager`: bayat `xmin` -> `row_conflict`, kısıt/tetikleyici -> tam geri alma, kilit zaman aşımı, iptal, üretim/kilit kapıları, bölümlü INSERT sürümü |
| `crates/pgt-ddl/tests/logging.rs` | Hücre/anahtar değerlerinin günlüğe düşmediği (yakalama testi) |
| `src-tauri` (data komutları) | `data_load/select/preview/apply/cancel` uçtan uca (testkit `fixture()` yalnız ilk URL'yi kullanır) |

## Nesne diyalogları testleri: Function/Procedure/Trigger/Type/Domain (Plan 18)

Arka uç `crates/pgt-ddl` altındadır; saf testler (insta anlık görüntüleri: `src/build/snapshots`) sunucusuzdur, Docker testleri `PGT_TEST_URLS` ister ve `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` ile PG 10–18'de koşar (sürüme bağlı adımlar atlanır: Procedure PG 11+, `OR REPLACE TRIGGER` PG 14+, multirange PG 14+, `BEGIN ATOMIC` PG 14+).

| Dosya | Kapsam |
|---|---|
| `crates/pgt-ddl/src/build/{routine,trigger,pgtype,domain}.rs` (birim) | Saf üreticiler: create/alter/drop anlık görüntüleri, dolar etiketi seçimi, `needs_recreate` tablosu, hostile ad/değerler, sürüm kapıları |
| `crates/pgt-ddl/tests/routine_roundtrip.rs` | Function/Procedure/Trigger: `create → load → eşit`, sabit nokta, `alter(old, new) → load → new`, OR REPLACE sınırları, yeniden kurma, `OR REPLACE TRIGGER` / PG < 14 yeniden kurma, 2BP01, salt okunur yüklemeler |
| `crates/pgt-ddl/tests/type_roundtrip.rs` | Type (enum/composite/range/multirange) ve Domain: gidiş-dönüş, sabit nokta, enum `ADD VALUE` zinciri (PG < 12 transaction dışı + kısmi uygulama), CHECK kısıtları, base/array/shell tipin salt okunur yüklenmesi |
| `crates/pgt-ddl/tests/manager.rs` | Yönetici entegrasyonu: `find_created`, `defaults(…, preset)`, salt okunur kapısı, ipuçları (2BP01/42P13/42809/0A000/42501), sahte `editable` -> `plan_changed` |
| `crates/pgt-ddl/src/lookup.rs` (birim + DB) | `languages` ve `triggerFunctions` açılır listeleri |
| `crates/pgt-ddl/tests/logging.rs` | Gövde/ad/değerlerin günlüğe düşmediği (yakalama testi) |

## Ana paket bütçesi (Plan 21)

`bun run bundle:check` geçici dizine kaynak haritalı bir `vite build` yapar (`dist/` değişmez), `index.html`'in giriş parçasını bulur ve `scripts/bundle-budget.json` ile karşılaştırır: ana paket baytı (`//# sourceMappingURL` yorumu düşülür; `bun run build` çıktısıyla aynıdır), `__vite__mapDeps` girdi sayısı, `modulepreload` adları (yalnız solid/web/Icon), kaynak haritasından VLQ çözülerek kaynak ve paket başına bayt tablosu (ilk 25) ve ana pakete sızan yasak paketler (Kobalte, floating-ui, corvu, …). Çıkış kodu 1: tavan aşımı, `enforceForbidden` açıkken yasak paket, preload ihlali, harita yok ya da eşlenen toplam ana baytın %90'ından az. Saf işlevler `scripts/bundle-check-lib.mjs`'tedir ve `scripts/bundle-check-lib.test.mjs` ile `bun run test`'te koşar (vitest `include`'u `scripts/**/*.test.mjs`'i kapsar).
