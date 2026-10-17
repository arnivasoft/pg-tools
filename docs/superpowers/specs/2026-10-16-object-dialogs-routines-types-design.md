# pg-tools — Alt Proje 5b: Fonksiyon / Procedure / Trigger / Type / Domain Diyalogları Tasarımı

- **Tarih:** 2026-10-16
- **Durum:** Plan 18 arka ucu uygulandı (sözleşme notları §8); Plan 19 ön yüzü yazıldı (`docs/superpowers/plans/2026-10-17-object-dialogs-p19-routines-types-on-yuz.md`), uygulanmadı
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

---

## 8. Plan 18 sözleşme notları (Plan 19 için)

Aşağıdaki maddeler **gönderilen** davranışı anlatır (kod ve Docker PG 10–18 testleriyle doğrulandı).

### 8.1 Model alanları (camelCase) ve salt okunurluk

- `ObjectDef` etiketi `kind` (`function`, `procedure`, `trigger`, `type`, `domain`); Procedure da `FunctionDef` taşır (`returns = null`). `FunctionReturn` etiketi `kind` (`type {typeName, setof}`, `table {columns[{name,typeName}]}`, `trigger`, `void`); `TypeBody` etiketi `variant` (`enum {values[{label, original}]}`, `composite {attributes[{name,typeName,collation,original}]}`, `range {subtype, subtypeOpclass, collation, canonical, subtypeDiff, multirangeTypeName}`).
- `FunctionDef`: `schema, name, owner, comment, language, args[{mode: in|out|inout|variadic, name, typeName, default}], returns, body, volatility (volatile|stable|immutable), strict, securityDefiner, leakproof, parallel (unsafe|restricted|safe), cost, rows` (sayısal **metin**), `settings[{name,value}], recreate, editable, unsupported`.
- `TriggerDef`: `schema, table, name, timing (before|after|instead_of), events{insert, update, updateColumns[], delete, truncate}, level (row|statement), when, functionSchema, functionName, args[] (metin), oldTable, newTable, constraint, deferrable, initiallyDeferred, enabled (origin|replica|always|disabled), comment, editable, unsupported`.
- `TypeDef`: `schema, name, owner, comment, body, editable, unsupported`. `DomainDef`: `schema, name, owner, comment, baseType, collation, default, notNull, constraints[{name, expression, notValid, validated, original, validate}], editable, unsupported`.
- **`editable` ve `unsupported` için serde varsayılanı YOKTUR** (dört türde de zorunlu alan): **yeni tanım `editable: true, unsupported: []` ile gönderilmelidir** (`ddl_defaults` bunu zaten doldurur; arayüz alanları düşürmemeli). `recreate` ve `DomainCheck.validate` ise `#[serde(default)]` (gönderilmezse `false`).
- **R5 kuralı (tüm türler):** yükleyici, `unsupported` boş değilse `editable = false` döndürür (ör. constraint trigger `FROM <tablo>`, `FROM CURRENT`, C/internal dil, `BEGIN ATOMIC`, aggregate/pencere, eklentiye ait, sistem şeması, iç/klonlanmış trigger). Böylece yeniden kurma, modellenemeyen bir özelliği sessizce düşüremez. Arayüz `editable = false` nesneyi salt okunur gösterir ve `unsupported` gerekçelerini listeler.
- **Base, array, pseudo, shell ve multirange tipler** boş `Composite { attributes: [] }` gövdesi + `unsupported` (+ `editable = false`) olarak yüklenir: formda **salt okunur** gösterilmeli (boş bir composite düzenleyicisi olarak değil).
- **Salt okunur kapısı:** `editable = false` bir nesnede `alter`/`drop` bağlantı ve plan üretiminden **önce** reddedilir: kodsuz `invalid`, ileti `"Bu nesne buradan düzenlenemez: <genel gerekçe>"` (ad/değer içermez; gerekçe sabit metindir). `create` etkilenmez. Arayüz `editable`'ı sahte olarak `true` yaparsa (sunucudaki güncel tanımla uyuşmaz) kapı yerine `plan_changed` / `invalid_def` döner.

### 8.2 `recreate` ve `needs_recreate`

- `recreate` yalnızca Function/Procedure `alter`'ında anlamlıdır; kalıcı değildir (yüklemede `false`). `true` ise ve `needs_recreate` doluysa plan `DROP (eski imza)` + `CREATE (yeni)` olur (**DataLoss**, GRANT'lar ve bağımlılar kaybolur); `needs_recreate` boşken sessizce yok sayılır. `recreate` seçilmeden `OR REPLACE`'in yapamadığı bir değişiklik istenirse alan hatası döner (PG'ye gidilmez; ileti: "…yeniden oluşturmayı gerektirir; 'Yeniden oluştur' seçin.").
- `needs_recreate` alan yolları: `args[i].typeName` (giriş argümanı kanonik tipi değişti; `int` = `int4` değişiklik sayılmaz), `args[i].name` (**var olan** giriş adı değişti ya da kaldırıldı; adsız argümana ad vermek serbest), `args[i].default` (var olan varsayılan **kaldırıldı**; ekleme/değiştirme serbest), `args` (giriş sayısı ya da OUT/INOUT listesi: sayı, mod, ad veya tip değişti; IN → INOUT dahil), `returns` (dönüş tipi, `SETOF`, `TABLE` kolonları, `void`). Aynı değişiklik iki kez raporlanmaz. Function ↔ Procedure dönüşümü `recreate` ile **yapılamaz** (tür uyuşmazlığı reddi); nesne silinip diğer türde yeniden oluşturulur.
- **Procedure imzaları** `ALTER/DROP/COMMENT` ifadelerinde **tüm argümanlar açık modla** yazılır (`IN tip`, `INOUT tip`, PG 14+ `OUT tip`): PG 11–13 ile 14+ arasındaki OUT kimliği farkı ve 42725 belirsizliği böyle aşılır (Docker 11–18).

### 8.3 `defaults(kind, preset)` ve `ddl_defaults`

- `defaults(server, database_oid, kind, ctx, preset)`; `preset` yalnızca `Function` için geçerlidir (`trigger_function` → dönüş `trigger`, gövde `BEGIN` / `  RETURN NEW;` / `END;`; başka türde kodsuz `invalid`). Varsayılanlar: Function dil `plpgsql`, dönüş `void`, boş `BEGIN … END;` gövdesi; Procedure `returns = null`; Type boş `enum`; Domain `baseType = "text"`; Trigger `after` + `insert` + `row`, `enabled = origin`. Function/Procedure/Type/Domain için bağlam şema (`schemaOid`), Trigger için tablo (`tableOid`; yalnız sıradan ve bölümlenmiş tablo).
- **`ddl_defaults` Tauri komutu artık isteğe bağlı `preset` alır.** Plan 19, `DdlTarget.create.preset` değerini `commands.ts` sarmalayıcısından göndermelidir (Trigger Functions koleksiyonunun Oluştur'u). `commands.ts` **ana pakettedir**: ek baytlar adı konmuş bir telafiyle karşılanmalıdır (bkz. 8.8).

### 8.4 Açılır listeler (`ddl_lookups`)

- `languages` (`{kind:"languages"}`): `pg_language` içinden prosedürel diller + `sql`; `c`/`internal` hariç; `value` = dil adı, `detail` = `güvenilir` | `güvenilmez`.
- `triggerFunctions` (`{kind:"trigger_functions", schema: string|null}`): dönüşü `trigger` ve argümansız fonksiyonlar; **`value` = fonksiyon adı, `detail` = şema** (R1). `schema = null` → kullanıcı şemaları + `pg_catalog`. Form `functionName` alanını `value`'dan, `functionSchema` alanını `detail`'den doldurmalıdır. En çok 5000 satır.

### 8.5 Alan yolları (`FieldError.path`)

- **Function/Procedure:** `schema`, `name`, `owner`, `comment`, `language`, `body`, `cost`, `rows`, `volatility`, `strict`, `leakproof`, `parallel`, `kind` (Procedure PG < 11), `object` (salt okunur), `args`, `args[i].name|typeName|default|mode` (`mode`: VARIADIC son giriş değil / Procedure OUT PG < 14), `returns`, `returns.typeName`, `returns.columns` (TABLE dönüşü kolonsuz), `returns.columns[i].name|typeName`, `settings[i].name|value`.
- **Trigger:** `schema`, `table`, `name`, `timing`, `events`, `events.updateColumns` (UPDATE olayı olmadan kolon listesi), `events.updateColumns[i]`, `events.truncate`, `level`, `when`, `functionSchema`, `functionName`, `args[i]`, `oldTable`, `newTable`, `deferrable`, `initiallyDeferred`, `comment`, `object`.
- **Type:** `schema`, `name`, `owner`, `comment`, `body`, `body.values` (liste düzeyi: yüklü enum değeri silme/yeniden sıralama), `body.values[i].label|original` (enum), `body.attributes` (liste düzeyi: yüklü öznitelik sırası / yeni öznitelik yalnız sona), `body.attributes[i].name|typeName|collation|original` (composite), `body.subtype|subtypeOpclass|collation|canonical|subtypeDiff|multirangeTypeName` (range), `object`.
- **Domain:** `schema`, `name`, `owner`, `comment`, `baseType`, `collation`, `default`, `constraints`, `constraints[i].name|expression|notValid|validate|original`, `object`.
- `old.` önekli yollar (`old.name`, `old.schema`, `old.table`, `old.args[i].typeName`, `old.body.values[i].label`, `old.body.attributes[i].name`, `old.constraints[i].name`) alter'da gönderilen **yüklü** (`old`) tanımın bayat ya da geçersiz olduğunu gösterir.
- `SECURITY DEFINER` seçimi önizlemede **uyarı** üretir (engellemez); yeniden oluşturma ayrıca GRANT uyarısı ve DROP notu ekler.

### 8.6 İpuçları (hata kodu → ipucu)

- `2BP01` + `drop` (Function/Procedure/Type/Domain): bağımlı nesneler (CASCADE seçin ya da bağımlıları kaldırın). Function/Procedure **yeniden kurma** (alter içindeki `DROP FUNCTION/PROCEDURE`) `2BP01`: PG'nin CASCADE önerisinin **yerine** "bağımlılar yeniden kurmayı engelliyor" ipucu gelir.
- `42P13` (OR REPLACE sınırı): özel ipucu yalnızca **PG kendi ipucunu verdiyse** ve ifade `CREATE OR REPLACE` ise PG ipucunun yerine geçer; PG ipucu yoksa PG iletisi olduğu gibi kalır.
- `42809`: Function ↔ Procedure dönüşümü yeniden oluşturma ister.
- `0A000` + `ALTER TYPE … ALTER ATTRIBUTE`: tablo kolonunda kullanılan composite'in öznitelik tipi değiştirilemez (kendi ipucu).
- `2BP01` + `ALTER TYPE … ATTRIBUTE` (typed table): "CASCADE gerekir (desteklenmiyor)"; PG'nin CASCADE önerisinin yerine geçer.
- `42501` + `CREATE` + `leakproof`: "LEAKPROOF yalnızca süper kullanıcı verebilir" (aynı CREATE'teki başka bir 42501 de bu ipucunu alabilir; PG iletisi asıl nedeni söyler).
- Transaction dışı ifadelerden sonra hata (PG < 12 enum `ADD VALUE`): "Önceki ifadeler uygulandı…" kısmi uygulama ipucu.
- PG'nin CASCADE öneren ipuçlarının **üçü** (typed-table `2BP01`, yeniden kurma `2BP01`, `42P13` SQL önerisi) bilinçli olarak değiştirilir: arayüz PG ipucunu değil bu metinleri gösterir.

### 8.7 Doğrulanmış davranışlar ve bilinen sınırlar

Doğrulanmış (Docker PG 10–18):
- `CREATE OR REPLACE TRIGGER` (PG 14+) `tgenabled`'ı **ORIGIN'e sıfırlar**, yorumu ve oid'i korur. PG < 14 (ve constraint trigger) `DROP` + `CREATE` yeniden kurmasında etkinlik ve yorum yeniden yazılır (Destructive + not).
- Enum `ADD VALUE` zinciri (aynı transaction'da yeni değerin ardına başka değer) PG 12–18'de çalışır. PG < 12'de `ADD VALUE` **transaction dışıdır** (`stmt_nt`): kısmi uygulama olabilir, işlenmiş (committed) sayısı raporlanır.
- Trigger `WHEN` koşulu `pg_get_triggerdef`'ten ayrıştırılır (sabit nokta testli).
- Multirange adı şema-nitelikli yazılır/yüklenir. Otomatik ad (yüklemede `None`) PG'nin `makeMultirangeTypeName`'i ile bayt bayt aynı hesaplanır: `range` içermeyen ad ham 52 bayta kesilir; çok baytlı karakter ortasına denk gelirse PG'nin ürettiği ad geçersiz UTF-8'dir (PG 16'da doğrulandı) — yükleyici adı ham bayt (`textsend`) okur ve çökmez.
- `COST`/`ROWS` PG'de `float4` olarak saklanır: yüklenen değer yazılandan farklı yuvarlanmış olabilir. Değişiklik tespiti değerleri `float4` (`f32`) olarak karşılaştırır: `float4`'e yuvarlanınca eşit olan metinler (ör. `1.23456789` ↔ `1.2345679`, `100` ↔ `100.0`) değişiklik sayılmaz; `alter(loaded, loaded)` boş plandır; arayüz yüklenen metni olduğu gibi göstermelidir.

Bilinen sınırlar (arayüz bunları açıklamalıdır):
- Yüklenen **düz `\` içeren** argüman varsayılanı, `WHEN` koşulu ya da domain `DEFAULT`/`CHECK` ifadesi (PG bunları `E'…'` öneki olmadan yazar) o ifadenin **düzenlenmesini** kapıda reddeder (ad/sahip/yorum serbest; Plan 14 view sorgusu emsali). Kaçış için `E'…'`.
- Typed table kullanan composite'te öznitelik değişimi (`CASCADE` gerektirir) desteklenmez; enum değerleri silinemez ve yeniden sıralanamaz (PG desteklemez; yalnız ekleme/yeniden adlandırma); yüklü composite öznitelikleri yeniden sıralanamaz (yeni öznitelik yalnız sona).
- Range'in multirange **şeması** modellenmez.
- PG 17+ **adlı** domain `NOT NULL` kısıtının adı korunmaz (`notNull = true` olarak yüklenir).
- View / `INSTEAD OF` trigger'lar bu planda yok (backlog; Explorer'da view altında trigger düğümü ulaşılamaz).
- `settings` sırası: yükleyici ayarları sıralı döndürür; **sıra değişikliği değişiklik sayılmaz** (ayar kümesi aynıysa `OR REPLACE` üretilmez; bir değişiklik olursa liste istemcinin gönderdiği sırayla yazılır).
- `BEGIN ATOMIC` (PG 14+ `prosqlbody`) gövdeli fonksiyonlar, C/internal dilli, aggregate/pencere, eklentiye ait ve sistem şemasındaki nesneler salt okunurdur.

### 8.8 Ön yüz bütçe notu

- Ana paket (`index-*.js`) **232,343 B**, `__vite__mapDeps` **21** girdi, preload `solid/web/Icon` — Plan 18 sonunda **değişmedi** (yeni TS yalnızca otomatik bindingler ve tembel derleme uyumu).
- Plan 19 ana pakete yalnızca `ddl-kinds.ts` (altı tür) ve `commands.ts` içindeki `ddl_defaults` `preset` gönderimini ekler; bu baytlar aynı görevde adı konmuş bir telafiyle (`t.query.grid` + `t.query.cell` → tembel modül) karşılanır; pay 0'dır ve ölçülür.
