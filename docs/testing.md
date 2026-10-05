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
ve `pgt_reader`, `pgt_app`, `pgt_limited` rollerini oluşturur/yeniden kurar; üretim sunucusunda çalıştırmayın.

Fixture: `tests/fixtures/schema.sql` her sürümde, `schema_pg11.sql` (procedure, partitioned index, `INCLUDE`,
hash partition, partitioned tabloda satır trigger'ı) PG 11+ ve `schema_pg12.sql` (generated kolon) PG 12+
sunucularda uygulanır. Sürüme bağlı test adımları `fx.major().await >= N` ile korunur.

## Sürüm matrisi (Docker)
    ./scripts/test-matrix.ps1                       # PG 10, 12, 14, 16, 17, 18
    ./scripts/test-matrix.ps1 -Versions 16,18
    ./scripts/test-matrix.ps1 -KeepContainers       # hata ayıklama için kapsayıcıları bırakır

Betik her sürüm için `postgres:NN` imajından `pgt-matrix-NN` kapsayıcısını `127.0.0.1:(55400+NN)` portunda
başlatır, her çalıştırmada rastgele üretilen (hiçbir yere yazılmayan) bir parolayla `PGT_TEST_URLS`'i ayarlar,
`cargo test --workspace --all-features` çalıştırır ve kapsayıcıları siler. Docker Desktop açık olmalıdır.

Sürüme özel DDL snapshot'ları `tests/snapshots/*@pgNN.snap` olarak tutulur. Yeni bir snapshot gerektiğinde
`INSTA_UPDATE=new` ile üretilip PG 18 ile karşılaştırılarak gözden geçirilir. Kabul edilen farklar yalnızca
sunucunun kendi deparse çıktısından gelir: `POSITION(... IN ...)` / `"position"(...)`, `EXECUTE FUNCTION` /
`EXECUTE PROCEDURE` (PG < 11), view gövdesinde tablo adıyla nitelenmiş kolonlar (PG < 16), `MAINTAIN` yetkisi
(PG 17+), pg_trgm sürümü (PG 10: 1.3, PG 12: 1.4, PG 14+: 1.6) ve PG 10'da olmayan nesneler (partitioned index,
`INCLUDE`).

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

PG 10, 12, 14, 16, 17 ve 18 üzerinde sürüm matrisiyle doğrulanır (aşağıya bakın). PG 10–11'de
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
  taslağında PG 11–12'de tekrar yazıldığı varsayılmıştı; PG 12 üzerinde canlı doğrulamada böyle olmadığı görüldü
  ve `extra.hits` fixture'ı + gidiş-dönüş testi bunu her sürümde korur.)
