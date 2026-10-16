# pg-tools — Alt Proje 5b: Fonksiyon / Procedure / Trigger / Type / Domain Diyalogları Tasarımı

- **Tarih:** 2026-10-16
- **Durum:** Taslak (Plan 18 arka ucu yazıldı; Plan 19 ön yüzü kalan)
- **Kapsam:** Object Explorer'dan **Function** (trigger fonksiyonları dahil), **Procedure** (PG 11+), **Trigger** (tablo/bölüm), **Type** (enum, composite, range) ve **Domain** için oluşturma, düzenleme, yeniden adlandırma ve silme; önizlemeli, korumalı DDL uygulaması.
- **Temel:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (Plan 10–15). Oradaki mimari (§2), risk modeli (§3), arayüz (§4), güvenlik (§6), test stratejisi (§7) ve tüm sözleşme notları **aynen geçerlidir**. Bu belge yalnızca yeni türlerin alanlarını, kurallarını ve kararlarını ekler; o spec'in §1.3 "kapsam dışı" listesinden bu beş tür çıkarılır.
- **Teslim planları:** Plan 18 (arka uç: modeller, saf üreticiler, yükleyiciler, hedefler, açılır listeler, yönetici entegrasyonu, PG 10–18 Docker matrisi) → Plan 19 (ön yüz: menü kancası + adı konmuş telafi, beş form, gövde düzenleyicisi).

---

## 1. Kararlar

| Konu | Karar |
|---|---|
| Fonksiyon gövdesi | Serbest metindir ve **arka uçta üretilen benzersiz dolar tırnağıyla** sarılır (`$pgt$ … $pgt$`; gövdede geçiyorsa `$pgt1$`, `$pgt2$` …). Gövdenin içeriği taranmaz (dil bağımsız; `;` serbest); yalnızca NUL reddedilir, ≤ 1 MiB. Nihai ifade `composed_is_clean` ile yeniden taranır (tek ifade). |
| Düzenleme stratejisi (Function/Procedure) | Gövde ve özniteliklerde değişiklik → **`CREATE OR REPLACE`** (Safe; sahiplik ve yetkiler korunur). PG'nin `OR REPLACE` ile yapamadıkları — **imza** (giriş argüman tipleri) değişimi, **dönüş tipi** değişimi, **giriş argüman adı** değişimi, **varsayılan kaldırma**, OUT argüman değişimi, Function ↔ Procedure — yalnızca açık **"Yeniden oluştur (DROP + CREATE)"** seçimiyle; seçilmeden yapılırsa alan hatası. Yeniden kurma **DataLoss**'tur (GRANT'lar, bağımlı trigger/view'lar CASCADE'siz DROP'u 2BP01 ile durdurur; Plan 14 view `recreate` kalıbı). |
| Kimlik | Fonksiyon kimliği `schema.name(giriş tipleri)`'dir; `ALTER/DROP/COMMENT` ifadeleri yüklenen (eski) imzayla yazılır. Aşırı yüklenmiş fonksiyonlar oid ile ayrı nesnelerdir. |
| Desteklenmeyen fonksiyonlar | `C`/`internal` dilli, `BEGIN ATOMIC` gövdeli (PG 14+ `prosqlbody`), aggregate, eklentiye ait, sistem şemasındaki fonksiyonlar **salt okunur** yüklenir (`unsupported` gerekçesi); create bu dilleri sunmaz. Pencere fonksiyonu (`prokind = 'w'`) salt okunur. |
| Trigger düzenleme | `RENAME`, `ENABLE/DISABLE` (`ALTER TABLE … ENABLE [REPLICA|ALWAYS] / DISABLE TRIGGER`) ve yorum doğrudan; diğer her değişiklik PG 14+'da `CREATE OR REPLACE TRIGGER` (Safe), PG 10–13'te ya da constraint trigger'da aynı transaction'da `DROP TRIGGER` + `CREATE TRIGGER` (**Destructive** + not "aynı transaction içinde yeniden kurulur"). İç (`tgisinternal`) ve bölümlenmiş üst tablodan klonlanmış (`tgparentid`, PG 13+) trigger'lar salt okunur. |
| Type | Enum: değer ekleme (`ADD VALUE [BEFORE|AFTER]`), değer yeniden adlandırma (PG 10+); değer silme/yeniden sıralama **doğrulama hatası** (PG desteklemez). PG < 12'de `ADD VALUE` **transaction dışıdır** (`stmt_nt`). Composite: öznitelik ekle (Safe), yeniden adlandır (Safe), tip değiştir ve sil (**DataLoss**: tipi kullanan tablolardaki veri). Range: yalnızca create; alter yalnız ad/şema/sahip/yorum. Base type ve multirange: salt okunur. |
| Domain | Temel tip, collation, DEFAULT, NOT NULL, CHECK kısıtları. Alter: `SET/DROP DEFAULT` (Safe), `SET NOT NULL` (Destructive: mevcut veride başarısız olabilir), `DROP NOT NULL` (Safe), `ADD CONSTRAINT … CHECK (…) [NOT VALID]` (Safe + "kullanan kolonları tarar" notu), `DROP CONSTRAINT` (Destructive), `RENAME CONSTRAINT`, `VALIDATE CONSTRAINT`; temel tip/collation değişimi doğrulama hatası. |
| Serbest SQL alanları | Fonksiyon gövdesi (dolar tırnaklı), argüman `DEFAULT` ifadeleri, trigger `WHEN` koşulu, domain `DEFAULT` ve `CHECK` ifadeleri. Gövde dışındakiler mevcut kapıdan geçer (`check_free_sql` + `wrap` + `composed_is_clean`; sıradan `'…'` içinde `\` → `E'…'`). |
| Tip ifadeleri | Argüman, dönüş, composite öznitelik, domain temel tipi ve range alt tipi mevcut katı dilbilgisiyle (`type_syntax::validate_type`) doğrulanır; `SETOF` ve `TABLE(…)` dönüşü modelde ayrı alanlardır. |
| `SET` yapılandırması | Fonksiyon/procedure `SET name = value` listesi mevcut `ConfigSetting` modeli ve `settings` yardımcılarıyla (Database/Role ile aynı kurallar); `FROM CURRENT` desteklenmez (salt okunur). |
| İfade sırası | Database/View kuralı: **`OWNER TO` en sonda**; `CREATE OR REPLACE` eski adla ilk; ardından `RENAME`, `SET SCHEMA`, diğer `ALTER`'lar, `COMMENT`. |
| Eklentiye ait nesneler | Salt okunur (`from_extension`); drop dahil designer eylemi reddedilir (kodsuz `invalid`). |

## 2. Alanlar

- **Function / Procedure:** şema, ad, sahip, yorum, dil (`plpgsql`, `sql`, kurulu güvenilir diller; `pg_language` listesi), argümanlar (mod `IN|OUT|INOUT|VARIADIC`, ad, tip, DEFAULT), dönüş (`tip` | `SETOF tip` | `TABLE (ad tip, …)` | `trigger` | `void`; Procedure'de yok), gövde, `VOLATILE|STABLE|IMMUTABLE`, `STRICT`, `SECURITY DEFINER`, `LEAKPROOF`, `PARALLEL UNSAFE|RESTRICTED|SAFE`, `COST`, `ROWS` (yalnız set dönen), `SET` listesi. Procedure: dil, argümanlar, gövde, `SECURITY DEFINER`, `SET`. Trigger fonksiyonu = dönüşü `trigger` olan Function (Explorer'daki "Trigger Functions" koleksiyonundan create dönüşü `trigger` ile önden doldurur).
- **Trigger:** tablo (bağlamdan, değiştirilemez), ad, zamanlama `BEFORE|AFTER|INSTEAD OF`, olaylar `INSERT`, `UPDATE [OF kolonlar]`, `DELETE`, `TRUNCATE`, düzey `ROW|STATEMENT`, `WHEN (…)`, fonksiyon (trigger fonksiyonu listesi), sabit argümanlar (metin dizisi), `REFERENCING OLD/NEW TABLE AS …` (PG 10+, yalnız AFTER), constraint trigger (`DEFERRABLE`, `INITIALLY DEFERRED`), etkinlik `ORIGIN|REPLICA|ALWAYS|DISABLED`, yorum.
- **Type:** şema, ad, sahip, yorum ve türe göre gövde: Enum (sıralı etiketler), Composite (öznitelik adı, tip, collation), Range (alt tip, alt tip opclass, collation, canonical/subtype_diff fonksiyonları, PG 14+ multirange adı).
- **Domain:** şema, ad, sahip, yorum, temel tip, collation, DEFAULT, NOT NULL, CHECK kısıtları (ad, ifade, NOT VALID; yüklemede "doğrulanmamış").

## 3. Risk özeti (§3 tablosuna ek)

| Risk | İfadeler |
|---|---|
| Safe | `CREATE FUNCTION/PROCEDURE/TRIGGER/TYPE/DOMAIN`, `CREATE OR REPLACE FUNCTION/PROCEDURE/TRIGGER`, `RENAME`, `SET SCHEMA`, `OWNER TO`, `COMMENT`, enum `ADD VALUE`/`RENAME VALUE`, composite `ADD/RENAME ATTRIBUTE`, domain `SET/DROP DEFAULT`, `DROP NOT NULL`, `ADD/RENAME/VALIDATE CONSTRAINT`, trigger `ENABLE/DISABLE` |
| Destructive | `DROP FUNCTION/PROCEDURE/TRIGGER/TYPE/DOMAIN` (CASCADE'siz), PG < 14 trigger yeniden kurma, domain `SET NOT NULL` ve `DROP CONSTRAINT` |
| DataLoss | Fonksiyon/procedure **yeniden oluşturma**, composite `DROP ATTRIBUTE` ve `ALTER ATTRIBUTE … TYPE`, her `DROP … CASCADE` |

## 4. Arayüz (Plan 19 özeti)

- Menü: Functions / Procedures / Trigger Functions / Types / Domains / Triggers koleksiyonlarında **Oluştur…**; düğümlerde **Düzenle… / Yeniden adlandır… / Sil…**; eklentiye ait ve sistem nesnelerinde eylem yok.
- Gövde düzenleyicisi: tembel form içinde **düzenlenebilir CodeMirror** (paylaşılan `codemirror-setup` parçasının `sqlLookExtensions()`'ı); ana paketin `__vite__mapDeps` listesi değişmemelidir (ölçülür); aksi hâlde `textarea` (Plan 15 Karar 1).
- Argüman ve composite öznitelik listeleri, enum değerleri, domain kısıtları satır düzenleyicisi (ekle / sil / geri al / sırala); `SET` listesi mevcut `SettingsEditor`.
- Ana paket bütçesi (232,343 B, pay 0): menü kancası (`ddl-kinds.ts`'e altı tür) aynı görevde adı konmuş bir telafiyle (`t.query.grid` + `t.query.cell` → tembel modül) karşılanır.

## 5. Güvenlik

- Tanımlayıcılar yalnızca `quote_ident`/`qualified`; değerler `quote_literal`; gövde yalnızca üretilmiş dolar tırnağıyla; tüm serbest SQL alanları kapıdan geçer.
- Gövde, SQL metni, argüman/değer ve tanım `tracing`'e yazılmaz; yalnızca işlem/tür/ifade sayısı/süre/kod.
- `SECURITY DEFINER` seçimi önizlemede uyarı üretir ("çağıran yerine sahibin yetkileriyle çalışır; `SET search_path` önerilir"), ama engellenmez.

## 6. Test stratejisi

- Saf üreticiler: insta anlık görüntüleri (create, alter varyantları, drop), dolar etiketi seçimi (gövdede `$pgt$` geçiyor), hostile adlar/değerler, sürüm kapıları.
- Docker PG 10–18 gidiş-dönüş: `create → load → eşit`, `alter(loaded, loaded)` boş plan (sabit nokta), `alter(old, new) → load → new`, sürüm kapıları (Procedure PG 11+, OR REPLACE TRIGGER PG 14+, enum ADD VALUE transaction davranışı PG < 12 / ≥ 12, multirange PG 14+, `prosqlbody` PG 14+ salt okunur), 2BP01 bağımlı nesne hataları.

## 7. Planlama notu

- **Plan 18 (arka uç):** modeller + hedefler + açılır listeler → Function/Procedure üreticisi → Trigger → Type → Domain → yükleyiciler ve Docker gidiş-dönüşleri → yönetici entegrasyonu → belgeler ve matris.
- **Plan 19 (ön yüz):** menü kancası + telafi → designer çekirdek eklemeleri → Function/Procedure formu (gövde düzenleyicisi) → Trigger formu → Type formu → Domain formu → smoke listesi.
