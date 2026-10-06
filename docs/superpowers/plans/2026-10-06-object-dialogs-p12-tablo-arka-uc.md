# Nesne Diyalogları — Tablo / Kolon / Kısıt Arka Ucu (Plan 12) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** `pgt-ddl`'e **Table** türünü eklemek: yapılandırılmış tanım (kolonlar + kısıtlar + bölümleme anahtarı), saf DDL üreticisi (create, **fark → ALTER**, drop), katalog yükleyici, açılır liste verileri (tipler, collation'lar), yönetici/komut entegrasyonu (`ddl_target_of` tablo/kolon/kısıt düğümleri). Ön yüz **Plan 13**'tedir; bu planda arayüze yalnızca derleme uyumluluğu için küçük (tembel) değişiklikler girer.

**Mimari:** Plan 10–11 kalıbının aynısı: istemci SQL göndermez, `ObjectDef::Table(TableDef)` gönderir; arka uç `build::{create,alter,drop}` ile planı üretir; `DdlManager` aynı kapılarla uygular (`object` yeniden yükleme eşitliği, plan özeti, onay, üretim onayı). Tablo, Index gibi tek bir nesne türüdür; **kolonlar ve kısıtlar tablo tanımının parçasıdır** (ayrı nesne türü değil). Fark üretimi için kolon ve kısıtlar **kararlı kimlikle** eşlenir (`attnum` / kısıt `oid`, yalnızca yüklemede dolar): kimliği olan eşleşir (ad değişimi = `RENAME`), kimliği olmayan yeni eklenir, kimliği olup yeni tanımda olmayan silinir.

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta (anlık görüntü), mevcut Docker matrisi.

**Spec:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§3 risk modeli, §5 Table, §6 güvenlik, §7 test, "Sözleşme notları"). Önceki plan: `docs/superpowers/plans/2026-10-06-object-dialogs-p10-arka-uc.md` (burada tekrar edilmeyen **tüm Global Constraints** ve **Review Focus** kuralları bağlayıcıdır: tırnaklama, NUL/denetim karakteri reddi, serbest SQL kapıları, kilit/onay/üretim kapıları, günlük gizliliği, Docker testi kuralları).

## Global Constraints

- Plan 10'un Global Constraints'i aynen geçerlidir ve burada özetle yinelenir: PostgreSQL **10–18** vanilla; tüm tanımlayıcı/değer yerleştirmeleri yalnızca `quote_ident`/`quote_literal`/`qualified`; tüm ad alanlarında boş/NUL/denetim karakteri/63 bayt üstü ve sistem şeması (`pg_*` küçük harf, `information_schema`) **reddedilir**; serbest SQL alanları **olduğu gibi, parantezli ve sonuna `\n` eklenerek** yerleştirilir ve her alan + **son bileşik ifade** `scan_semicolons` kapılarından geçer (kapanmamış yapı, dış `;`, dengesiz parantez, `--` ile biten alan, rakam-bitişik `'`/`$`, NUL, boş alan → reddedilir); istemciden SQL **alınmaz**.
- **Serbest SQL alanları (Table):** `ColumnDef.default`, `GeneratedDef.expr`, `CheckDef.expr`, `PartitionKey::Expression.sql`, `ColumnDef.using` (alter-only tip dönüşümü). Başka hiçbir alan SQL olarak yorumlanmaz.
- **Veri tipi alanı** (`ColumnDef.dataType`, bir *tip ifadesi metni*, ör. `numeric(10,2)`, `character varying(50)`, `timestamp(3) with time zone`, `public.mytype`, `"My Type"[]`, `int4[][]`): serbest SQL **değil**, katı bir dilbilgisiyle doğrulanır (`type_syntax.rs`): boşlukla ayrılmış sözcük dizisi (kimlik karakterleri `[A-Za-z_][A-Za-z0-9_$]*`), noktalı şema niteleme, çift tırnaklı tanımlayıcı (`""` ikilemesiyle; denetim karakteri yok), en çok bir parantezli değiştirici listesi (yalnızca rakam, virgül, boşluk), `[]`/`[n]` dizi ekleri; **başka hiçbir karakter yok** (`;`, `'`, `--`, `/*`, `\`, `$$`, `(` içinde sözcük vb. reddedilir). Doğrulanan metin olduğu gibi yerleştirilir (tırnaklı kısımlar zaten doğrulanmış tırnaklı tanımlayıcılardır). Tipin var olup olmadığı **sunucuda** kontrol edilir (uygulamada hata; geri alınır).
- **Tablo ve kolon/kısıt adları** kısıtlar için de aynı ad doğrulayıcısından geçer; kısıt adı `None` ise sunucu adlandırır.
- Sürüm kapıları: `GENERATED … STORED` PG 12+; `GENERATED … VIRTUAL` PG 18+; `ALTER COLUMN … SET EXPRESSION` PG 17+ (daha eski sürümde üretilmiş ifade değişimi **doğrulama hatasıdır**: "kolonu silip yeniden ekleyin"); `UNIQUE NULLS NOT DISTINCT` PG 15+; PK/UNIQUE `INCLUDE` PG 11+; `PARTITION BY HASH` PG 11+; bölümlenmiş tablo PG 10+; kimlik (`GENERATED … AS IDENTITY`) PG 10+. Desteklenmeyen alan **doğrulama hatasıdır** (sessizce atlanmaz).
- **Risk sınıfları** (spec §3 ile uyumlu, değişmez): `CREATE TABLE` Safe; `ADD COLUMN` (NULL yapılabilir/varsayılanlı) Safe, `ADD COLUMN … NOT NULL` Destructive (mevcut satırlar için başarısız olabilir → not); `ALTER COLUMN TYPE` **DataLoss** (not: tabloyu yeniden yazar, `USING` gerekebilir); `DROP COLUMN` **DataLoss**; `SET NOT NULL` Destructive; `DROP NOT NULL`/`SET|DROP DEFAULT`/kimlik değişimi Safe; `RENAME` (tablo/kolon/kısıt) Safe (not: bağımlı nesneler); kısıt `DROP` Destructive; kısıt `ADD` (FK/CHECK/UNIQUE/PK) Safe + uyarı (tabloyu tarar/kilitler; `NOT VALID` seçeneği ayrı alanla); `DROP TABLE` Destructive, `CASCADE` DataLoss; `SET TABLESPACE`/`SET LOGGED|UNLOGGED` Safe + not (tabloyu yeniden yazar, ACCESS EXCLUSIVE); sahip/yorum/`fillfactor` Safe.
- **Plan sırası (alter):** ifadeler tek transaction'da, şu sabit sırayla üretilir ve sonraki ifadeler **yeni** adları kullanır: (1) tablo `RENAME`, (2) `SET SCHEMA`, (3) **kaldırılan veya değişen kısıtların `DROP CONSTRAINT`'i** (PG, kolon silince/tip değiştirince bağlı kısıtları kendiliğinden düşürebilir ya da hata verir — bu yüzden önce), (4) kolon `DROP COLUMN`, (5) kolon `TYPE` değişimleri, (6) kolon `RENAME COLUMN`, (7) `ADD COLUMN`, (8) nullability/default/kimlik/üretilmiş ifade değişimleri, (9) kısıt `RENAME CONSTRAINT`, (10) kısıt `ADD CONSTRAINT`, (11) tablo özellikleri (owner, tablespace, persistence, fillfactor, yorumlar). Kolon yeniden adlandırma bir kısıtın kolon listesini etkiliyorsa kısıt, adlar eşleme sonrası eşitse **değişmemiş** sayılır (PG kısıtı kendiliğinden izler) — gereksiz DROP+ADD üretilmez.
- **Kolon sırası** değiştirilemez: korunan kolonların göreli sırası eski ve yeni tanımda farklıysa plana uyarı eklenir ("kolon sırası değiştirilemez; yeni sıra yoksayılır") ve fark yalnızca ekleme/silme/değişim üzerinden üretilir.
- **Bölüm tabloları ve kalıtım:** `TableDef.isPartition` / `inherits` (yalnızca yüklemede dolar) true ise kolon ve kısıt değişiklikleri **doğrulama hatasıdır** ("Bölüm/kalıtımlı tablonun kolon ve kısıtları ana tablodan yönetilir"); yeniden adlandırma/şema/sahip/tablespace/yorum serbesttir. Bölümleme anahtarı alter'da değiştirilemez (hata); create-only.
- **Desteklenmeyen kısıt/özellikler yüklemede korunur:** `EXCLUDE` kısıtları, PG 18 `NOT NULL` kısıt nesneleri (`contype = 'n'`), tetikleyici kısıtları (`'t'`) ve tanımda modellenmeyen kimlik dizi seçenekleri: yükleyici bunları `TableDef.unsupported: Vec<String>` (insan okunur ad/tür listesi, yalnızca yükleme) olarak raporlar; alter hiçbir zaman bunlara dokunmaz.
- Başarı ölçütleri: Docker gidiş-dönüş testleri **PG 10–18 matrisi** (create → load → eşit; alter(old,new) → load → new; boş değişiklik → boş plan); önizleme saf ve ucuzdur (sunucu sürümü önbellekten).
- Test sunucuları, günlük, ts-rs, `Box::pin`, commit eki, git worktree yasağı: Plan 10'daki gibi. Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
- **Ön yüz derleme uyumu:** `ObjectKind`'e `table` eklenince `src/ipc/bindings/ObjectKind.ts` değişir; arayüzdeki `FORMS`/`FORM_PATHS` (`Record<ObjectKind, …>`) tip hatası vermemeli. Bu planda **yalnızca tembel designer dosyalarında** (`DesignerView.tsx`, `forms-model.ts` vb.) `Partial`'a geçilip "form henüz yok" korumasıyla uyum sağlanır; **ana paket (`index-*.js`) bayt sayısı değişmemeli** (233,733 B); menü/`ddl-kinds` kümelerine `table` **eklenmez** (Plan 13).

## Review Focus

- **Enjeksiyon/tırnaklama:** tablo/kolon/kısıt/tip/varsayılan/CHECK/üretilmiş/bölümleme alanlarında `;`, `"`, `'`, `\`, `--`, `/*`, NUL, denetim karakteri, çok uzun ad, Unicode, anahtar sözcük adlarıyla her türün create/alter/drop çıktısı **tek ifade** kalır; tip dilbilgisi ve serbest SQL kapıları atlatılamaz (saldırgan yeniden inceleme planlanır).
- **Fark doğruluğu:** kimlik eşleme (attnum/oid) ile ekleme/silme/yeniden adlandırma ayrımı; kolon yeniden adlandırma + kısıt kolon listesi eşlemesi; sabit plan sırası; boş değişiklik → boş plan; sürüm kapıları; kolon sırası uyarısı; bölüm/kalıtım koruması; desteklenmeyen kısıtların korunması.
- **Yıkıcı işlemler:** risk sınıflarının eksiksizliği (her ifade türü doğru risk; `DROP COLUMN`/`TYPE`/`SET NOT NULL`/kısıt silme); `object` yeniden yükleme eşitliği ve `plan_changed` yolu tablo için de çalışır.
- **Gidiş-dönüş doğruluğu PG 10–18** (özellikle üretilmiş kolon, kimlik, FK eylemleri, UNIQUE INCLUDE/NULLS NOT DISTINCT, bölümlenmiş tablo, collation, varsayılan ifadelerin `pg_get_expr` normalleşmesi → "değiştirmeden düzenle → boş plan").
- **Kısmi uygulama ve kilit:** tüm planlar transactional; `lock_timeout` altında uzun `ALTER TABLE`; `ADD CONSTRAINT` tarama uyarıları.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-ddl/src/model.rs` | `ObjectKind::Table`, `TableDef`, `ColumnDef`, `ConstraintDef` (PK/Unique/Check/FK), `IdentityDef`, `GeneratedDef`, `PartitionByDef`, `ObjectDef::Table` |
| `crates/pgt-ddl/src/build/type_syntax.rs` | Tip ifadesi dilbilgisi doğrulayıcısı |
| `crates/pgt-ddl/src/build/freesql.rs` | Index'ten çıkarılan ortak serbest SQL kapısı (`check_free_sql`, sarmalama, son-ifade taraması) |
| `crates/pgt-ddl/src/build/table.rs` (+ `table_alter.rs`, `table_constraint.rs`) | create/alter/drop üreticileri, fark üretimi, sıralama |
| `crates/pgt-ddl/src/load.rs`, `sql/load_table*.sql`, `lookup.rs`, `sql/lookup_types.sql`, `lookup_collations.sql` | Tablo yükleyici ve yeni lookup'lar |
| `crates/pgt-ddl/src/manager.rs`, `target.rs` | `find_created`, `defaults`, `identity_changed`, `ddl_target_of` (tablo/koleksiyon/kolon/kısıt), `focus` |
| `crates/pgt-ddl/tests/*.rs` | Docker gidiş-dönüş ve yönetici testleri |
| `src/ipc/bindings/*`, `src/features/designer/{DesignerView,forms-model}.tsx/.ts` | Yeni bindingler; yalnızca tembel derleme uyumu |

---

### Task 1: Model, tip dilbilgisi, ortak serbest SQL kapısı

**Dosyalar:**
- Değiştir: `crates/pgt-ddl/src/model.rs`, `crates/pgt-ddl/src/build/mod.rs`, `crates/pgt-ddl/src/build/index.rs` (ortak kapıyı içe aktarmak için; davranış **aynı kalmalı**, mevcut testler yeşil)
- Oluştur: `crates/pgt-ddl/src/build/type_syntax.rs`, `crates/pgt-ddl/src/build/freesql.rs` ve testleri

**Arayüzler:**
- Üretir (`model.rs`; hepsi `Serialize + Deserialize + TS`, camelCase, `#[ts(export)]`; sayılar `#[ts(type="number")]`):

```rust
#[serde(rename_all = "snake_case")] pub enum ObjectKind { Schema, Sequence, Extension, Index, Table }   // table eklenir
#[serde(rename_all = "snake_case")] pub enum Persistence { Permanent, Unlogged }
#[serde(rename_all = "snake_case")] pub enum IdentityKind { Always, ByDefault }
pub struct GeneratedDef { pub expr: String, pub stored: bool /* false = VIRTUAL (PG 18+) */ }
pub struct ColumnDef {
    /// Yalnızca yüklemede dolar (pg_attribute.attnum): fark eşlemesinin kararlı kimliği. Yeni kolonda None.
    #[ts(type = "number | null")] pub attnum: Option<i16>,
    pub name: String, pub data_type: String,                  // tip ifadesi metni (type_syntax)
    pub not_null: bool, pub default: Option<String>,          // serbest SQL
    pub identity: Option<IdentityKind>, pub generated: Option<GeneratedDef>,
    pub collation: Option<String>, pub comment: Option<String>,
    /// Alter-only, kalıcı değil: tip değişiminde `USING <ifade>` (serbest SQL).
    pub using: Option<String>,
}
#[serde(rename_all = "snake_case")] pub enum FkAction { NoAction, Restrict, Cascade, SetNull, SetDefault }
#[serde(rename_all = "snake_case")] pub enum FkMatch { Simple, Full }
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConstraintKind {
    PrimaryKey { columns: Vec<String>, include: Vec<String> },
    Unique { columns: Vec<String>, include: Vec<String>, nulls_not_distinct: bool },
    Check { expr: String /* serbest SQL */, no_inherit: bool },
    ForeignKey { columns: Vec<String>, ref_schema: String, ref_table: String, ref_columns: Vec<String>,
                 match_type: FkMatch, on_update: FkAction, on_delete: FkAction },
}
pub struct ConstraintDef {
    #[ts(type = "number | null")] pub oid: Option<u32>,       // yalnızca yükleme; yeni kısıtta None
    pub name: Option<String>, pub kind: ConstraintKind,
    pub deferrable: bool, pub initially_deferred: bool,       // PK/UNIQUE/FK
    /// Yalnızca ADD sırasında anlamlı (NOT VALID); yüklemede `validated`'ın tersi.
    pub not_valid: bool,
}
#[serde(rename_all = "snake_case")] pub enum PartitionStrategy { Range, List, Hash }
#[serde(tag = "kind", rename_all = "snake_case")] pub enum PartitionKey { Column { name: String }, Expression { sql: String } }
pub struct PartitionByDef { pub strategy: PartitionStrategy, pub keys: Vec<PartitionKey> }
pub struct TableDef {
    pub schema: String, pub name: String, pub owner: Option<String>, pub tablespace: Option<String>,
    pub persistence: Persistence, pub columns: Vec<ColumnDef>, pub constraints: Vec<ConstraintDef>,
    pub partition_by: Option<PartitionByDef>,                  // create-only
    #[ts(type = "number | null")] pub fillfactor: Option<u32>, pub comment: Option<String>,
    // yalnızca yükleme:
    pub partition_text: Option<String>,                        // pg_get_partkeydef (salt okunur gösterim)
    pub is_partition: bool, pub inherits: bool,
    pub unsupported: Vec<String>,
}
// ObjectDef'e `Table(TableDef)` eklenir (`kind: "table"`).
```

- Üretir (`build/type_syntax.rs`): `pub fn validate_type(s: &str) -> Result<(), String>` — Global Constraints'teki dilbilgisi; her red için Türkçe, alan-düzeyinde anlaşılır ileti. 128 karakter üst sınırı. Dahil: tek sözcüklü (`integer`), çok sözcüklü (`double precision`, `character varying(50)`, `timestamp(3) with time zone`, `interval year to month`), şemalı (`public.mytype`), tırnaklı (`"My Type"`), dizi (`text[]`, `int4[][]`, `numeric(10,2)[3]`).
- Üretir (`build/freesql.rs`): Index'teki serbest SQL kapılarının (boş/NUL/kapanmamış/dış `;`/dengesiz parantez/`--` ile biten/rakam-bitişik tırnak) **ortak** işlevi: `pub fn check_free_sql(path: &str, sql: &str, errs: &mut Vec<FieldError>)` ve `pub fn wrap(sql: &str) -> String` (= `({sql}\n)`); `build/index.rs` bunları kullanacak şekilde yeniden düzenlenir (davranış değişmez; mevcut index testleri ve anlık görüntüleri **aynen** geçmeli).
- `build/mod.rs`: `ObjectDef::Table` için `validate/create/alter/drop` dispatch'i **bu görevde** "Table türü henüz desteklenmiyor" yer tutucu hatasıyla bağlanır (Task 2–4 değiştirir).

- [ ] **Adım 1: Başarısız testler.** `type_syntax`: kabul tablosu (yukarıdaki örnekler + çok parçalı) ve ret tablosu (`int; drop table t`, `int'x'`, `text --`, `a/*b*/`, `numeric(1;2)`, `numeric(a)`, `int[`, `"unterminated`, `"a\nb"`, boş, 129 karakter, NUL, `$$`, `(` içinde sözcük, `[` içinde sözcük); `freesql`: Index'in mevcut saldırı matrisinin işlev düzeyinde kopyası (sonuçlar aynı); `model`: JSON gidiş-dönüş (camelCase, `type`/`kind` etiketleri, `attnum: null`, enum snake_case), eksik alan varsayılanları (`not_valid`/`deferrable` false, `using` null), geçersiz enum reddi.
- [ ] **Adım 2:** `cargo test -p pgt-ddl` — başarısızlığı gör; **Adım 3: Uygula.** Index yeniden düzenlemesinin ardından **tüm mevcut pgt-ddl testleri** (anlık görüntüler dahil) değişmeden geçmeli.
- [ ] **Adım 4:** `cargo test -p pgt-ddl`, `cargo clippy -p pgt-ddl --all-targets -- -D warnings`; yeni binding dosyalarını (`TableDef.ts`, `ColumnDef.ts`, `ConstraintDef.ts`, `ConstraintKind.ts`, …) ve değişen `ObjectKind.ts`/`ObjectDef.ts`'yi ekle (`git checkout -- src/ipc/bindings` yalnızca satır sonu gürültüsü için).
- [ ] **Adım 5: Commit** (`feat(ddl): Table modeli, tip dilbilgisi doğrulayıcısı, ortak serbest SQL kapısı`).

---

### Task 2: Tablo `create` ve `drop` üreticileri

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/table.rs` (+ anlık görüntüler); Değiştir: `build/mod.rs`

**Arayüzler:**
- Tüketir: Task 1 modeli/dilbilgisi/kapı; `quote.rs`; `Plan/Statement/Risk`.
- Üretir: `build::create(&ObjectDef::Table(..))` / `build::drop(..)` tablo kolları ve tabloya özgü `validate_table(def, ctx) -> Vec<FieldError>`.

**Doğrulama (FieldError yolları):** `name`, `schema`, `owner`, `tablespace`, `comment`; `columns` (en az bir kolon — bölümlenmiş tablo dahil), `columns[i].name|dataType|default|generated|collation|comment|using`; yinelenen kolon adı (`columns[i].name`); `dataType` → `validate_type`; `default`/`generated.expr` → `check_free_sql`; `identity` ve `generated` birlikte olamaz; `identity` + `default` birlikte olamaz; `generated` + `default` birlikte olamaz; `generated.stored == false` → PG 18+ kapısı; `generated` → PG 12+ kapısı; `constraints[i].name` (ad doğrulayıcı; yinelenen kısıt adı), `constraints[i].columns[j]` her kolon adı tablo kolonları içinde olmalı (FK için `columns` yerel kolonlar, `refColumns` ref tablo kolonlarıdır — ref tablonun kolonları sunucuda doğrulanır, burada yalnızca boş olmama ve `columns.len() == refColumns.len()`), PK en çok bir tane; `check.expr` → `check_free_sql`; `include` PG 11+; `nullsNotDistinct` PG 15+; `deferrable=false` iken `initiallyDeferred=true` hata; `fillfactor` 10–100; `partitionBy` → `keys` boş değil, `Expression.sql` → `check_free_sql`, `Hash` PG 11+; `isPartition`/`inherits` true olan tanımlarda **create** reddedilir ("yüklenmiş tanım yeniden oluşturulamaz" — create yalnızca yeni tanım içindir).

**Üretim:**
- `CREATE [UNLOGGED] TABLE <schema.name> (\n  <kolon>,\n  …,\n  <kısıt>,…\n) [PARTITION BY <STRATEGY> (<anahtarlar>)] [WITH (fillfactor = n)] [TABLESPACE <ts>]` (Statement.sql tek ifade; kolon/kısıtlar `,\n  ` ile ayrılır — yeni satırlar **yapısal** olup serbest SQL alanları kendi `(…\n)` sarmalarıyla ayrıdır).
- Kolon: `<quote_ident(ad)> <dataType> [COLLATE <q>] [GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY] [GENERATED ALWAYS AS (<expr>\n) { STORED | VIRTUAL }] [DEFAULT (<expr>\n)] [NOT NULL]` (sıra deterministik; tip metni doğrulanmış olduğundan olduğu gibi).
- Kısıt: `[CONSTRAINT <q>] PRIMARY KEY (<kolonlar>) [INCLUDE (…)]`, `UNIQUE [NULLS NOT DISTINCT] (…) [INCLUDE (…)]`, `CHECK (<expr>\n) [NO INHERIT]`, `FOREIGN KEY (<yerel>) REFERENCES <ref_schema.ref_table> (<ref kolonlar>) [MATCH FULL] ON UPDATE <eylem> ON DELETE <eylem>`; `DEFERRABLE [INITIALLY DEFERRED]`; create'te `not_valid` **yok sayılmaz**: `NOT VALID` yalnızca FK/CHECK için ve **create'te hata** ("NOT VALID yalnızca mevcut tabloya kısıt eklerken").
- Sonrasında ayrı ifadeler: `ALTER TABLE … OWNER TO <o>` (owner varsa), `COMMENT ON TABLE`, kolon yorumları `COMMENT ON COLUMN <schema.table.col> IS …` (yalnızca dolu olanlar) — hepsi Safe; create planı Safe.
- `drop`: `DROP TABLE <schema.name>` (Destructive) / `CASCADE` (DataLoss, uyarı "Bağımlı görünümler, yabancı anahtarlar da silinir"); bölüm tablosu için not.

- [ ] **Adım 1: Başarısız testler.** Tablo odaklı + `insta`: temel tablo (1 kolon), çok kolonlu (tüm özellik kombinasyonları: identity/generated stored/virtual, default, collation, not null), PK/UNIQUE(+INCLUDE, NULLS NOT DISTINCT)/CHECK/FK (tüm eylem/match kombinasyonları, deferrable), bölümlenmiş (range/list/hash, ifade anahtarı), UNLOGGED, fillfactor, tablespace, owner/yorumlar, sürüm kapıları (örn. `110000`/`120000`/`150000`/`170000`/`180000` varyantları), her doğrulama hatası yolu, **enjeksiyon matrisi** (her ad/tip/serbest SQL alanında hostile girdiler → tek ifade; `scan_semicolons` yardımcısıyla `assert_single_statements`), anahtar sözcük adları, 63/64 bayt, denetim karakterleri, `drop` matrisi.
- [ ] **Adım 2–4:** Başarısızlığı gör, uygula, `cargo test -p pgt-ddl` + clippy temiz; anlık görüntüler commit edilir (`.snap.new` kalmaz).
- [ ] **Adım 5: Commit** (`feat(ddl): Table create/drop üreticileri (kolonlar, kısıtlar, bölümleme, sürüm kapıları)`).

---

### Task 3: Tablo `alter` — tablo düzeyi ve kolon farkı

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/table_alter.rs` (+ anlık görüntüler); Değiştir: `build/table.rs`, `build/mod.rs`

**Arayüzler:**
- Üretir: `build::alter(&Table(old), &Table(new))` — kimlik eşlemeli kolon farkı ve tablo düzeyi değişiklikler (kısıt farkı Task 4'te; bu görevde `constraints` farkı **dokunulmaz**: iki tanımın kısıtları farklıysa şimdilik `Err` "henüz desteklenmiyor" döner, Task 4 kaldırır).

**Kurallar (Global Constraints'teki sıra 1, 2, 4–8, 11):**
- Doğrulama: `old` yalnızca **kimlik alanları** (ad/şema/kolon adları/attnum'lar) için doğrulanır (Plan 10 dersi: yüklenmiş tanımın create-geçerliliği gerekmez); `new` tam doğrulanır (kısıtlar dahil Task 1–2 kuralları, `isPartition`/`inherits` koruması hariç — aşağıya bak). `isPartition || inherits` ise kolon/kısıt farkı → `columns`/`constraints` hata yolu; `partitionBy` değişimi → hata; ad/şema/sahip/tablespace/yorum serbest.
- **Kolon eşleme:** `old.columns` ve `new.columns` `attnum` ile eşlenir. `new`'de `attnum: Some(n)` olup `old`'da bulunmayan → doğrulama hatası (bayat/oynanmış tanım); `attnum: None` → yeni kolon; `old`'da olup `new`'de olmayan → silinen. Tekrarlanan `attnum` → hata.
- **Silinen kolon:** `ALTER TABLE <q> DROP COLUMN <q>` — **DataLoss**, not "Kolondaki veri kalıcı silinir".
- **Tip değişimi** (`dataType` metni farklı): `ALTER TABLE … ALTER COLUMN <q> TYPE <tip> [COLLATE <q>] [USING (<expr>\n)]` — **DataLoss**, not "Tabloyu yeniden yazar; bağlı indeks/kısıtlar yeniden oluşturulur"; `using` verilmişse serbest SQL kapısı; `using` yalnızca tip/collation değişimiyle birlikte anlamlıdır (aksi halde `using` alanı yoksayılır + uyarı).
- **Yeniden adlandırma:** `ALTER TABLE … RENAME COLUMN <eski> TO <yeni>` Safe + not.
- **Ekleme:** `ALTER TABLE … ADD COLUMN <kolon tanımı>` (create'teki kolon biçimi); `NOT NULL` + varsayılansız ise Destructive + not "Mevcut satırlar varsa başarısız olur"; `generated`/`identity` kuralları create ile aynı.
- **Nullability/varsayılan/kimlik/üretilmiş:** `SET NOT NULL` (Destructive, not) / `DROP NOT NULL` (Safe); `SET DEFAULT (<expr>\n)` / `DROP DEFAULT` (Safe); kimlik: yok→var `ADD GENERATED … AS IDENTITY` (NOT NULL varsayar; not), var→yok `DROP IDENTITY`, tür değişimi `SET GENERATED ALWAYS|BY DEFAULT`; üretilmiş ifade: yok→var **kolonu yeniden eklemek gerekir** → doğrulama hatası ("Mevcut kolonu üretilmiş kolona çevirmek desteklenmez; yeni kolon ekleyin"), var→yok `DROP EXPRESSION` (PG 13+, Safe+not) , ifade değişimi PG 17+ `SET EXPRESSION AS (<expr>\n)` (DataLoss? yeniden hesaplar → Destructive + not), PG<17 hata; `collation` yalnızca `TYPE` ifadesinin parçası olarak değişir.
- **Kolon yorumu:** `COMMENT ON COLUMN` (değişen/eklenen; `None` → `IS NULL`).
- **Tablo düzeyi:** `RENAME TO`, `SET SCHEMA <q>` (Safe + not "bağımlılar etkilenir"), `OWNER TO`, `SET TABLESPACE` (Safe + not; **temizleme yok**: `new.tablespace == None` ve `old` dolu → doğrulama hatası "pg_default'u açıkça seçin", Index kuralıyla aynı), `SET LOGGED|UNLOGGED` (Safe + not), `SET (fillfactor = n)`/`RESET (fillfactor)`, `COMMENT ON TABLE`.
- **Boş değişiklik** → boş plan; birleşik değişikliklerde ifade sırası Global Constraints'teki sabit sıradır ve sonraki ifadeler **yeni** tablo adını/şemasını kullanır (`schema.name` ilk iki adımdan sonra güncellenir).
- **Kolon sırası uyarısı:** korunan kolonların göreli sırası farklıysa plan uyarısı.
- `Plan.warnings`: tablo yeniden yazımı (TYPE, SET TABLESPACE, SET LOGGED/UNLOGGED), `lock_timeout` önerisi (uzun ALTER TABLE).

- [ ] **Adım 1: Başarısız testler.** Matris + `insta`: tek tek her değişiklik türü (silme, ekleme (nullable/NOT NULL/default/identity/generated), tip (USING'li/USING'siz), yeniden adlandırma, nullability, default, kimlik üç geçiş, üretilmiş ifade geçişleri × sürüm kapıları (`120000`/`130000`/`170000`/`180000`), yorum), tablo düzeyi (ad, şema, sahip, tablespace (temizleme hatası dahil), persistence, fillfactor set/reset, yorum), **birleşik değişiklik sırası** (tablo rename + kolon rename + tip + ekleme + silme aynı planda; sonraki ifadelerin yeni adı kullandığı), boş plan (`alter(x,x)`), kimlik eşleme hataları (bayat attnum, yinelenen attnum), kolon sırası uyarısı, bölüm/kalıtım koruması, `old` yalnızca kimlik doğrulaması (create-geçersiz bir yüklenmiş tanım düzenlenebilir), **enjeksiyon matrisi** yeni adlar/tip/using/default alanlarında, risk sınıfları tablosu (her ifade türü için beklenen risk — tek testte).
- [ ] **Adım 2–4:** Uygula; `cargo test -p pgt-ddl` + clippy temiz.
- [ ] **Adım 5: Commit** (`feat(ddl): Table alter — kimlik eşlemeli kolon farkı ve tablo düzeyi değişiklikler`).

---

### Task 4: Kısıt farkı ve sıralama (alter tamamlanır)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/table_constraint.rs` (+ anlık görüntüler); Değiştir: `build/table_alter.rs`

**Arayüzler:**
- Üretir: `alter`'ın kısıt kolu ve Global Constraints'teki **tam sıra** (3: kısıt DROP, …, 9: RENAME CONSTRAINT, 10: ADD CONSTRAINT).

**Kurallar:**
- Eşleme: `oid` ile (yüklemeden gelen). `new`'de `oid: Some(n)` olup `old`'da bulunmayan → doğrulama hatası; `oid: None` → yeni kısıt; `old`'da olup `new`'de olmayan → silinen.
- **Değişmemiş sayma:** tür ve alanlar eşitse (kolon adları kolon yeniden adlandırma eşlemesinden sonra karşılaştırılır; FK `ref_*` doğrudan karşılaştırılır; `deferrable/initiallyDeferred`; `Check.expr` metin eşitliği; `nulls_not_distinct`; `include`; `not_valid` **yüklemede** `!validated`, karşılaştırmada **yoksayılır** — doğrulama durumu değişimi `VALIDATE CONSTRAINT` ayrı eylemidir: `old` doğrulanmamış (`notValid=true`) ve `new` `notValid=false` ise `ALTER TABLE … VALIDATE CONSTRAINT <q>` (Safe, not "Tabloyu tarar")).
- Yalnızca **ad** değişmişse `ALTER TABLE … RENAME CONSTRAINT <eski> TO <yeni>` (Safe); PK/UNIQUE'in indeksi de yeniden adlanır (not).
- **Yapısal değişiklik** (herhangi başka alan) → `DROP CONSTRAINT` (**Destructive**, not: bağımlı yabancı anahtarlar varsa 2BP01 hatası verir; CASCADE **sunulmaz**) + `ADD CONSTRAINT` (Safe + uyarı: tabloyu tarar/kilitler; `not_valid` ile eklenebilir) — ADD, adı **yeni** ad olarak verir.
- **Silinen:** `ALTER TABLE … DROP CONSTRAINT <q>` Destructive.
- **Yeni:** `ALTER TABLE … ADD [CONSTRAINT <q>] <tanım> [NOT VALID]` (`NOT VALID` yalnızca FK/CHECK; diğerlerinde `notValid=true` hata). Kısıt adı yoksa sunucu adlandırır.
- Kısıtın bağlı olduğu kolon `DROP`/`TYPE` ile birlikteyse: sabit sıra (3 → 4/5) sayesinde kısıt önce düşürülür; kısıt `new`'de duruyorsa (kolon silinmediyse) 10. adımda yeniden eklenir; kısıt kolonu `new`'de **yok** ise doğrulama hatası (`constraints[i].columns[j]`).
- Birincil anahtar değişimi: eski PK `DROP` + yeni PK `ADD` (DataLoss değil Destructive; uyarı: bağımlı FK'ler).
- **Bağımlılık uyarıları:** FK hedefi (`ref_*`) için sunucuda varlık kontrolü yapılmaz (uygulamada hata).
- Boş/aynı → boş plan.

- [ ] **Adım 1: Başarısız testler.** Matris + `insta`: her kısıt türü için ekleme/silme/ad değişimi/yapısal değişim (kolon listesi, FK eylemleri, CHECK ifadesi, deferrable, INCLUDE, NULLS NOT DISTINCT), kolon yeniden adlandırma + kısıt kolon eşlemesi (**gereksiz DROP+ADD yok**), `VALIDATE CONSTRAINT`, NOT VALID kuralları, birleşik plan sırası (kısıt DROP → kolon DROP/TYPE → … → kısıt ADD; tam sıra anlık görüntüsü), kısıt kolonu silinen kolon → hata, bayat oid hataları, bölüm/kalıtım koruması, boş plan, enjeksiyon matrisi (kısıt adı, CHECK ifadesi, FK ref adları).
- [ ] **Adım 2–4:** `cargo test -p pgt-ddl` + clippy; Task 3'teki geçici "henüz desteklenmiyor" `Err` kaldırılır.
- [ ] **Adım 5: Commit** (`feat(ddl): Table kısıt farkı (eşleme, yeniden adlandırma, DROP+ADD, VALIDATE) ve tam plan sırası`).

---

### Task 5: Tablo yükleyici, lookup'lar ve gidiş-dönüş testleri (Docker)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/sql/load_table.sql`, `load_table_columns.sql` (+ `.pg10`/`.pg11`/`.pg12`/`.pg15`/`.pg17` varyantları gerektiğinde), `load_table_constraints.sql`, `lookup_types.sql`, `lookup_collations.sql`, `crates/pgt-ddl/tests/table_roundtrip.rs`
- Değiştir: `crates/pgt-ddl/src/load.rs`, `crates/pgt-ddl/src/lookup.rs`

**Arayüzler:**
- Üretir: `load(.., ObjectRef{kind: Table, oid})` → `ObjectDef::Table`; `LookupRequest::Types { query: Option<String> }` (örn. `int` yazınca `integer`, `int4`… öneri; `value` = `format_type` biçiminde **kullanılabilir tip ifadesi** (şema niteleme yalnızca `search_path` dışındaysa), `detail` = tür kategorisi; en çok 200 sonuç; kullanıcı tipleri ve extension tipleri dahil, `pg_catalog` sistem iç tipleri (`pg_*`, `_arrayturleri`, `internal`, `record`…) hariç), `LookupRequest::Collations` (`value` kullanılabilir ad, `detail` sağlayıcı/locale).
- Yükleyici ayrıntıları: **tablo** (`pg_class` relkind `r`/`p`; sahip, tablespace (`pg_default` → None), persistence (`relpersistence`), `fillfactor` (reloptions), yorum; `is_partition = relispartition`, `inherits` = `pg_inherits`'te ebeveyn varlığı; `partition_text` = `pg_get_partkeydef`); **kolonlar** (`pg_attribute`: `attnum > 0 AND NOT attisdropped`; `data_type = format_type(atttypid, atttypmod)`; `not_null`; varsayılan `pg_get_expr(adbin, adrelid)` yalnızca `attgenerated = ''` ve kimlik değilse; `identity` (`attidentity`: `a`/`d`); `generated` (`attgenerated` `s` stored / `v` virtual (PG 18) + ifade); collation yalnızca kolon tipinin varsayılan collation'ından **farklıysa**; yorum); **kısıtlar** (`pg_constraint` `contype` `p`/`u`/`c`/`f`, `conrelid = tablo`; kolon adları `conkey`→`attname` sıralı; PK/UNIQUE `include` ve `nulls_not_distinct` `pg_index` üzerinden (PG 11+/15+ varyantları); `check.expr` `pg_get_expr(conbin, conrelid)` (dış parantezleri çıplak tek-ifade biçiminde tutarlı normalleştir: önceki Index dersi — **dengeli iç parantezleri bozmayan** soyma); FK: `confrelid` şema/ad, `confkey`→ref kolon adları, `confmatchtype`, `confupdtype`, `confdeltype`; `condeferrable`, `condeferred`; `not_valid = !convalidated`); **desteklenmeyen**: `contype IN ('x','t','n')` kısıtlar ve tanımsız kimlik dizisi seçenekleri `unsupported` listesine (tür + ad); bölüm/kalıtım ebeveynli tabloda kolonlar yüklenir (salt okunur değerlendirme builder'dadır).
- **Normalleşme sözleşmesi:** `load → (düzenleme yapmadan) → alter(loaded, loaded)` **boş plan** olmalıdır (her tür için test); `data_type` metni `format_type` çıktısıdır ve `validate_type`'tan geçmelidir (her yüklenen tip, kullanıcı tipleri ve extension tipleri dahil, dilbilgisini sağlamalı — sağlamayan tip varsa bu görevde dilbilgisi genişletilir ve test eklenir).

- [ ] **Adım 1: Başarısız Docker testleri** (`tests/table_roundtrip.rs`; `servers()`/`guarded`/per-test veritabanı kalıbı `tests/roundtrip.rs` ile aynı; `PGT_TEST_URLS` yoksa atlanır):

```rust
async fn create_then_load_equals_def()        // çok kolonlu tablo: identity, generated stored (PG12+), virtual (PG18), default, collation, NOT NULL; PK/UNIQUE(+INCLUDE PG11+, NULLS NOT DISTINCT PG15+)/CHECK/FK(tüm eylemler); UNLOGGED; fillfactor; tablespace pg_default → None; yorumlar; bölümlenmiş (range/list/hash PG11+) → partition_text dolu
async fn alter_then_load_equals_new()         // her değişiklik türü old→new: ekle/sil/yeniden adlandır/tip(USING)/null/default/identity/generated; kısıt ekle/sil/yeniden adlandır/yapısal; tablo rename/schema/owner/persistence/fillfactor; birleşik plan
async fn unchanged_edit_is_an_empty_plan()    // load → alter(loaded, loaded) boş — her kolon/kısıt türü ve PG sürümü için
async fn column_rename_keeps_constraints()    // kolon yeniden adlandır + kısıt kolon listesi güncelle → DROP+ADD YOK, kısıt oid aynı kalır
async fn partitions_and_inheritance_are_flagged() // partition child ve INHERITS child: is_partition/inherits; kolon/kısıt farkı doğrulama hatası; ad/yorum serbest
async fn unsupported_constraints_are_preserved()  // EXCLUDE kısıtlı tablo: unsupported dolu; alter başka bir değişiklikte EXCLUDE'a dokunmaz (apply sonrası hâlâ var); PG18 NOT NULL kısıtları görünmez/etkilenmez
async fn every_loaded_type_passes_the_grammar()   // çeşitli tipler (builtin, domain, enum, composite, extension tipi (citext varsa), array, range, bit/varbit, numeric(p,s), timestamptz(3), interval fields) için load→validate_type
async fn type_lookup_and_collation_lookup()       // Types{query:"int"} makul sonuçlar, sınır 200, sistem iç tipleri yok; Collations
async fn drop_table_variants()                    // DROP, CASCADE bağımlı FK/view ile; bölüm tablosu
```

  Gerçek sunucuda **üreticinin SQL'inin geçerliliğini** kanıtlamak için create/alter yolları `apply` ile değil `admin_client` ile üretilen ifadeleri çalıştırarak sınanır (Task 6'dan önce manager yok). Sürüm-bağımlı testler `server_version`'a göre dallanır ve atladığını açıkça yazdırır.
- [ ] **Adım 2: Uygula** yükleyici ve lookup'lar. Üreticide gidiş-dönüş testinin ortaya çıkardığı **hatalar bu görevde düzeltilir** (regresyon birim testiyle; raporda belirtilir).
- [ ] **Adım 3:** Docker'lı: tek PG 18 ile `cargo test -p pgt-ddl`; sonra `scripts/test-matrix.ps1 -Versions 10,11,12,13,15,17,18 -SkipPerf` (PG 12 üretilmiş kolon kapısı; PG 15 NULLS NOT DISTINCT; PG 17 `SET EXPRESSION`; PG 18 sanal kolon) — kapsayıcılar silinir.
- [ ] **Adım 4:** clippy temiz; **Adım 5: Commit** (`feat(ddl): Table yükleyici, tip/collation lookup'ları ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 6: Yönetici ve komut entegrasyonu

**Dosyalar:**
- Değiştir: `crates/pgt-ddl/src/manager.rs`, `crates/pgt-ddl/src/target.rs`, `crates/pgt-ddl/src/lib.rs`, gerekirse `src-tauri/src/ddl.rs`; Test: `crates/pgt-ddl/tests/manager.rs`, `crates/pgt-ddl/tests/json.rs`, `target.rs` birim testleri

**Arayüzler:**
- `DdlManager`: `defaults(kind=Table, ctx)` (`schema_oid` zorunlu → şema adı; varsayılan tanım: `columns: []`, `constraints: []`, `persistence: permanent`), `find_created` Table için (şema + ad → `pg_class` oid), `identity_changed(old,new)` Table için (ad veya şema değişti → true), apply yolunda tablo `alter/drop` için aynı kapılar (`object` yeniden yükleme eşitliği: yüklenen `TableDef` ile `old` **tam eşit** — yükleyici deterministik olmalı), `PlanRequest` JSON'u Table için çalışır; sistem tabloları (`pg_catalog`/`information_schema`/`pg_toast` şeması) `load` aşamasında `Invalid`.
- `DdlTarget` genişler: `focus: Option<DdlFocus>` (`DdlFocus { kind: "column" | "constraint", id: number }` — kolon için `attnum`, kısıt için `oid`) — `ts(export)`. `target_of`: `Seg::Table(oid)` düğümü → `object: Table`; `Coll::Tables` (şema altında) → `create: Table` (`schema_oid`); `Seg::Column(attnum)` düğümü → `object: Table(üst tablo oid)` + `focus: column(attnum)`; `Seg::Constraint(oid)` düğümü (tablo altında) → `object: Table` + `focus: constraint(oid)`; **Seg::Partition/ViewColumn vb. diğer her düğüm → boş**; `refreshNodeId`: create → koleksiyon, tablo → üst `Tables` koleksiyonu, kolon/kısıt → **tablonun kendisi** (alt koleksiyonlar yenilensin) — gerçek `NodeId` API'siyle (arayüz kimlik ayrıştırmaz).
- `LookupRequest` bağlama doğrulaması (Types/Collations JSON'u).

- [ ] **Adım 1: Başarısız testler.** `target.rs`: tablo/koleksiyon/kolon/kısıt düğümleri (+ görünüm kolonu, partition koleksiyonu, rol vb. için boş), `refreshNodeId` kuralları, bozuk kimlik → `invalid`; JSON testleri (`TableDef`, `ConstraintDef` etiketleri, `PlanRequest` create/alter/drop × table); Docker (`tests/manager.rs`): tablo create → apply → `Applied.created` dolu → `load` eşit; alter apply (kolon ekle + kısıt ekle) → yeniden yükle eşit; **bayat `old`** (başka oturumda kolon eklendi) → `plan_changed`; yıkıcı plan onaysız → `confirmation_required`; `DROP COLUMN` planı DataLoss + üretim etiketli sunucuda `production_confirmation_required`; kilitli sunucuda `locked`; kısıt kolonu silme yolunda plan sırası gerçek sunucuda **geçerli** (kısıtlı kolon sil + kısıtı kaldır aynı apply); `lock_timeout` altında kilitli tabloya ALTER → `55P03`, değişiklik yok; sistem tablosu `load` → `Invalid`; kolon yeniden adlandırma + kısıt eşlemesinin `apply` sonucu `load` eşit; ayrıca bölüm tablosu kolon değişikliği isteği → `invalid_def`.
- [ ] **Adım 2: Uygula.** **Adım 3:** Docker'lı `cargo test -p pgt-ddl` + `scripts/test-matrix.ps1 -Versions 10,13,18 -SkipPerf`; `cargo test --workspace` (Docker'sız) ve `cargo test -p pg-tools`; clippy.
- [ ] **Adım 4: Commit** (`feat(ddl): Table yöneticisi/komut entegrasyonu (defaults, find_created, target_of kolon/kısıt odağı)`).

---

### Task 7: Ön yüz derleme uyumu (tembel), belgeler ve bütünleşik doğrulama

**Dosyalar:**
- Değiştir: `src/features/designer/DesignerView.tsx`, `src/features/designer/forms-model.ts` ve gerekirse `designer-model.ts` (yalnızca tip uyumu), `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (durum satırı + Sözleşme notları: `ColumnDef.attnum`/`ConstraintDef.oid` yalnızca yükleme kimliğidir, tip metni, `DdlTarget.focus`, alter `created` semantiği tablo için), `docs/smoke-test.md` (Plan 12 arka uç Docker doğrulama notu — GUI maddeleri Plan 13'te)

- [ ] **Adım 1: Derleme uyumu.** `bun run typecheck` yeni `ObjectKind` ile kırılıyorsa `FORMS`/`FORM_PATHS` ve `kindLabel`/`defName` gibi `Record<ObjectKind, …>` ya da kapsamlı `switch` kullanan **tembel** designer kodunu `Partial`/varsayılan koluna uyarla: `table` için `kindLabel("table") = "Table"`, `defName` (`schema.name`), form yuvası boşken `DesignerView`'da **tek satırlık geçici koruma** ("Bu tür için form henüz yok" — `tdes.view.formMissing`; Plan 13 kaldırır); ana pakette (`src/app`, `QueryTabs`, `tabs-store`, `explorer/*`, `strings.ts`) **hiçbir değişiklik yok**; `ddl-kinds.ts` kümelerine `table` eklenmez (menüde tablo eylemi çıkmaz).
- [ ] **Adım 2: Doğrulama.** `bun run typecheck`, `bun run test` (834+ yeşil; varsa yeni testler), `bun run build` (**ana `index-*.js` tam 233,733 B**), `cargo test --workspace` (Docker'sız yeşil), Docker'lı matris: `scripts/test-matrix.ps1 -Versions 10,11,12,13,15,17,18 -SkipPerf` yeşil; `cargo clippy --workspace --all-targets -- -D warnings`; `rg "tracing::" crates/pgt-ddl/src` çağrıları SQL/ad içermez; `rg "format!" crates/pgt-ddl/src/build` her kullanım `quote_*` çıktısı, doğrulanmış tip metni ya da sabit anahtar sözcük birleştirir (incelemede tek tek). Spec durum satırı: "Plan 10–12 (arka uç Table dahil) uygulandı; Plan 13 (Table arayüzü), Plan 14 (View/MatView, Database, Role) kalan".
- [ ] **Adım 3: Commit** (`docs/chore(ddl): Table için ön yüz derleme uyumu, spec ve smoke notları`).

---

## Kapanış kontrolü (Plan 12 bitişi)

- Docker matrisi PG 10–18 yeşil (Table gidiş-dönüş + yönetici testleri); kapsayıcılar silinir.
- Saldırgan yeniden inceleme (Table tip dilbilgisi + serbest SQL kapıları) Plan 10'daki Index incelemesi gibi yapılır (final incelemeden ÖNCE ayrı görev olarak planlanır; ledger'a not).
- Final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.
