# Test notları

## DDL gidiş-dönüş testi ve bilinen sınırlar

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

Şimdiye kadar yalnızca PostgreSQL 18 üzerinde doğrulandı. PG 10–17 kod yolları (ör. `*.pg10.sql`) canlı
sunucuda gidiş-dönüş testinden geçirilmedi.
