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

## Yayın exe duman testi
    bun run build
    cargo build --release -p pg-tools               # ya da bun run tauri build
    ./scripts/smoke-release.ps1                     # varsayılan: target\release\pg-tools.exe, 6 sn
    ./scripts/smoke-release.ps1 -Exe C:\yol\pg-tools.exe -Seconds 10

Betik exe'yi geçici, boş `APPDATA`/`LOCALAPPDATA` dizinleriyle başlatır (gerçek yapılandırmaya ve loglara
dokunmaz, sunucuya bağlanmaz, parola kullanmaz), ana pencerenin 10 sn içinde oluşmasını bekler, süreci
`-Seconds` boyunca izler, sonra uygulama + WebView2 süreç ağacını kapatır ve geçici
`%LOCALAPPDATA%\pg-tools\logs` altında `panicked` / `stack overflow` arar. Çıkış kodu 0 = başarılı; ≠ 0 =
exe yok, pencere oluşmadı, süre dolmadan çıktı (çıkış kodu yazdırılır) ya da logda panik var.
Yalnızca debug derlemede görünmeyen hataları yakalamak içindir: Plan 2'de release exe açılıştan ~1 sn sonra
ana iş parçacığında yığın taşmasıyla (`STATUS_STACK_OVERFLOW`) çöküyordu; varsayılan 6 sn bu tür bir
gerilemeyi yakalayacak kadar uzundur. Her yayından önce çalıştırın.

## pgAdmin içe aktarma sınırı
Dosya önizleme ile içe aktarma arasında değiştirilirse sonuç değişebilir (uygulama aynı yolu yeniden okur; indeksler kayabilir).
