# Nesne Diyalogları — Policy / Tablespace Arka Ucu (Plan 34) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** `pgt-ddl`'e iki yeni nesne türünün **arka ucunu** eklemek: **Policy** (RLS politikası; tablo kapsamlı) ve **Tablespace** (sunucu kapsamlı) — modeller, saf üreticiler (create / alter farkı / drop), katalog yükleyicileri, Explorer hedef eşlemesi, yönetici entegrasyonu; ayrıca **Table** tanımına satır güvenliği bayrakları (`rowSecurity`) ve yetki matrisine **tablespace** (`GRANT CREATE ON TABLESPACE`). **Ön yüz bu planda değil** (Plan 35); bu plan ön yüze yalnızca **tembel derleme uyumu** için dokunur (Task 1).

**Mimari:** Plan 10–18 kalıbı aynen: istemci SQL göndermez, `ObjectDef` gönderir; arka uç `build::{create, alter, drop}` ile planı üretir; `DdlManager` aynı kapılarla uygular (yeniden yükleme eşitliği, plan özeti, onay, üretim onayı, kilit). Yenilikler: (1) **politika yeniden kurma** — `ALTER POLICY`'nin yapamadığı değişiklikler (komut, PERMISSIVE/RESTRICTIVE, ifade kaldırma) aynı transaction'da `DROP` + `CREATE` (Plan 18 trigger kalıbı); (2) **transaction dışı create/drop** — `CREATE/DROP TABLESPACE` `stmt_nt` (Plan 14 Database kalıbı; kısmi uygulamada `failed.created`); (3) **sunucu kapsamlı yeni tür** — Tablespace bakım veritabanında yüklenir/uygulanır (`server_scoped() = true`).

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta, mevcut Docker matrisi (`scripts/test-matrix.ps1`).

**Spec:** `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md` (§1 kararlar, §2 Grup A, §5–§7). Temel spec'ler: `docs/superpowers/specs/2026-10-06-object-dialogs-design.md`, `…/2026-10-16-object-dialogs-routines-types-design.md` (R5 kuralı, §8 sözleşme notları), `…/2026-10-17-privileges-design.md` (yetki matrisi). Önceki planlar: `2026-10-16-object-dialogs-p18-routines-types-arka-uc.md`, `2026-10-17-privileges-p20-arka-uc.md` — burada tekrarlanmayan **tüm Global Constraints ve Review Focus** maddeleri bağlayıcıdır.

## Kararlar

1. **`PolicyDef`:** `schema`, `table` (bağlamdan; alter'da değişimi alan hatası `table`), `name`, `command` (`all|select|insert|update|delete`), `permissive: bool`, `roles: Vec<String>` (boş = `PUBLIC`; yükleyici `polroles = {0}` → boş liste; ad listesi **ada göre sıralı** yüklenir, sıra farkı değişiklik sayılmaz), `using: Option<String>`, `with_check: Option<String>`, `comment`, `editable`, `unsupported`, ve **salt okunur** yükleme alanı `table_rls: Option<RowSecurity>` (`#[serde(default)]`; üretici yok sayar; create'te `None`).
2. **Politika kuralları (alan hatası):** `insert` → `using` yasak (`using`); `select`/`delete` → `with_check` yasak (`withCheck`); `roles` öğeleri `check_ident`, yinelenen yok; `public` (küçük/büyük harf) öğe olarak reddedilir ("PUBLIC için listeyi boş bırakın"). İfadesiz politika geçerli; plan uyarısı ("İfadesiz politika: PERMISSIVE tüm satırlara izin verir, RESTRICTIVE etkisizdir."). `table_rls = Some({enabled: false, ..})` iken create/alter planına uyarı: "Tabloda satır güvenliği kapalı; politika etkili olmaz."
3. **Politika alter:** (1) ad değiştiyse `ALTER POLICY <eski> ON <tablo> RENAME TO <yeni>`; (2) `command`/`permissive` değişti **ya da** `using`/`with_check` `Some → None` oldu → aynı transaction'da `DROP POLICY <ad> ON <tablo>` + `CREATE POLICY …` (**Destructive** + not "Politika aynı transaction içinde yeniden kurulur."), aksi hâlde değişen kısımlarla tek `ALTER POLICY <ad> ON <tablo> [TO …] [USING (…)] [WITH CHECK (…)]` (Safe); (3) `COMMENT ON POLICY <ad> ON <tablo>`. Yeniden kurmada yorum yeniden yazılır. Drop: `DROP POLICY <ad> ON <tablo>` (**Destructive**; CASCADE yok — `cascade = true` alan hatası değil, yok sayılır ve drop modeli Plan 35'te kutuyu göstermez; `drop-model` tablosu `cascade: false`).
4. **`RowSecurity { enabled: bool, forced: bool }`** (`Default` = false/false). `TableDef.row_security: RowSecurity` (`#[serde(default)]`). Create: `enabled` → `ALTER TABLE … ENABLE ROW LEVEL SECURITY`, `forced` → `ALTER TABLE … FORCE ROW LEVEL SECURITY` (Safe; boş yeni tablo), `CREATE TABLE`'dan sonra, `COMMENT`/`OWNER TO`'dan önce. Alter: `ENABLE` ve `FORCE` **Destructive** + not ("Politikası olmayan tabloda sahibi dışındakiler hiçbir satır görmez."), `DISABLE`/`NO FORCE` Safe + uyarı ("Tablonun politikaları devre dışı kalır."). Sıra: kolon/kısıt ifadelerinden sonra, `COMMENT`'ten önce. Bölüm tablosunda ve INHERITS çocuğunda da düzenlenebilir (Plan 12 `isPartition` izin listesine eklenir). Yükleyici `relrowsecurity`/`relforcerowsecurity` okur.
5. **`TablespaceDef`:** `name`, `owner: Option<String>`, `location: String` (create-only; alter'da değişim alan hatası `location`), `options: TablespaceOptions { seq_page_cost, random_page_cost, effective_io_concurrency, maintenance_io_concurrency }` (her biri `Option<String>`, sayısal **metin**), `comment`, `editable`, `unsupported`.
6. **Tablespace doğrulama:** ad `check_ident` + `pg_` öneki reddi (`name`); `location` boş değil, NUL yok, mutlak (`/` ile başlar ya da `^[A-Za-z]:[\\/]`), ≤ 1024 bayt (`location`); `seq_page_cost`/`random_page_cost` sonlu ondalık ≥ 0; `effective_io_concurrency`/`maintenance_io_concurrency` 0–1000 tamsayı; `maintenance_io_concurrency` PG < 13 alan hatası (sürüm kapısı). Değer metni ifadeye **kanonik biçimde** yazılır (ayrıştırılan sayının `to_string()`'i değil, doğrulanan metnin kendisi; tek tırnaklı literal: `seq_page_cost = '1.5'`).
7. **Tablespace ifadeleri:** create: `CREATE TABLESPACE <q> [OWNER <q>] LOCATION <literal> [WITH (a = 'x', …)]` **`stmt_nt`**, ardından `COMMENT ON TABLESPACE` (`stmt_nt`; kısmi uygulama kuralı, `failed.created`). Alter (hepsi `stmt_nt`, Database kalıbı): (1) `ALTER TABLESPACE <eski> RENAME TO <yeni>`; (2) `SET (…)` (yeni/değişen) ve `RESET (…)` (kaldırılan) — iki ayrı ifade, ad sırası sabit; (3) `COMMENT`; (4) `OWNER TO` (**en sonda**). Drop: `DROP TABLESPACE <q>` **`stmt_nt`**, **Destructive** (CASCADE yok). Risk: create/rename/set/reset/owner/comment Safe.
8. **Salt okunur tablespace:** `pg_default`, `pg_global` → `unsupported = ["Sistem tablespace'i"]`, `editable = false` (R5); salt okunur kapısı alter/drop'u reddeder. `pg_tablespace_location()` boş dizge döner (sistem) → yükleyici `location = ""` yazar.
9. **Sunucu kapsamı:** `ObjectKind::Tablespace.server_scoped() = true` (`Database|Role|Tablespace`). Hedef: yol `[Coll::Tablespaces]` → `create(Tablespace)`, `[Seg::Tablespace(oid)]` → `object(Tablespace, oid)`; `databaseOid = None`; `refreshNodeId`: create'te koleksiyon (`srv:U/tablespaces`), nesnede `srv:U/tablespaces`. Yetki matrisi de bakım veritabanında çalışır (Database ACL kalıbı).
10. **Policy hedefi:** `…/tbl:T/pol:P` (ebeveyn `table_at` ile geçerli tablo; bölüm tablosu dahil) → `object(Policy, P)`, `refreshNodeId = …/tbl:T/policies`; `…/tbl:T/policies` → `create(Policy, schemaOid = en yakın şema, tableOid = T)`. `defaults(Policy)`: bağlam tablosunun `schema`/`table`'ı, `command = all`, `permissive = true`, `roles = []`, ifadeler `None`, `table_rls` = tablonun güncel durumu. `defaults(Tablespace)`: `owner = None`, `location = ""`, seçenekler boş.
11. **ACL:** `AclDef`/yetki matrisi `ObjectKind::Tablespace`'i kabul eder: ayrıcalık kümesi `{Create}`, nesne ifadesi `TABLESPACE <q>`, yükleyici `pg_tablespace.spcacl` (+ `acldefault('t', spcowner)`), sunucu kapsamlı (bakım veritabanı). Varsayılan yetkiler (`ALTER DEFAULT PRIVILEGES`) tablespace'i desteklemez — istek alan hatası.
12. **Şema karşılaştırma etkisi:** `pgt-diff` Table'ı `pgt-ddl` yükleyici + `alter` ile karşılaştırır; `row_security` farkı artık diff'e girer (bilinçli). `pgt-diff` testleri ve `TableDef { … }` literal'leri derlenecek biçimde güncellenir; politikalar şema karşılaştırmasına **eklenmez** (backlog; smoke notu).
13. **Kapsam dışı (backlog):** yerinde tablespace (`allow_in_place_tablespaces`), tablespace'teki nesneleri taşıma, politika şema karşılaştırması, `ALTER POLICY` ile tablo değişimi (PG desteklemez).

## Global Constraints

- Plan 10/12/14/18 Global Constraints aynen: PG **10–18** vanilla; tüm tanımlayıcı/değer yerleştirmeleri yalnızca `quote_ident`/`quote_literal`/`qualified`; ad alanlarında boş/NUL/denetim/görünmez biçim karakteri/63 bayt üstü reddedilir (`check_ident`), yüklü (eski) taraf yalnız kimlik alanlarıyla gevşek doğrulanır (`check_old_*`); alter yalnızca **değişen ya da ifadeye yazılan** alanları doğrular; `Statement.sql` sonda `;` taşımaz ve tek ifadedir.
- **Serbest SQL alanları:** Policy `using`, `with_check` → `check_free_sql(path, …)` + `wrap()` + nihai ifadede `composed_is_clean`; sıradan `'…'` içinde `\` reddedilir (`E'…'`). Yüklenen (`pg_get_expr`) metin düz `\` içerirse yalnız o ifadenin düzenlenmesi reddedilir (Plan 18 §8.7 emsali).
- **Risk sınıfları:** spec §2.3 tablosu aynen (Karar 3, 4, 7).
- **Transaction:** Policy ve Table RLS ifadeleri transaction içinde; tüm Tablespace ifadeleri `stmt_nt`.
- **Kapsam ve hedefler:** Karar 9–10. `other_nodes_are_empty` benzeri testlerden `pol`/`ts` düğümleri çıkarılır.
- **Günlük/gizlilik:** ifade, ad, değer, konum (`location`) ve SQL asla `tracing`'e, hata iletisine ya da diske yazılmaz (yalnız işlem/tür/ifade sayısı/süre/kod).
- **Docker:** yalnızca `scripts/test-matrix.ps1` (`pgt-matrix-<v>` geçici kapsayıcılar; bitince silinir); başka kapsayıcılara dokunulmaz; `PGT_TEST_URLS` yalnızca test komutunun ortamında; sır adlı dosya yazılmaz. Test nesneleri benzersiz önekli şema/ad (`pgt_ddl_rt_<etiket>`) ve `guarded` temizliğiyle. **Tablespace dizinleri:** Task 4, betiğe kapsayıcı başına `docker exec <ad> sh -c 'mkdir -p /var/lib/postgresql/pgt_ts/ts1 … ts8 && chown -R postgres:postgres /var/lib/postgresql/pgt_ts'` ekler ve `PGT_TEST_TS_ROOT=/var/lib/postgresql/pgt_ts` ortam değişkenini yalnız test komutu süresince ayarlar (`finally` içinde silinir). Değişken yoksa tablespace Docker testleri **atlanır** (başarısız sayılmaz; çıktıda not). Her test kendi alt dizinini (`ts1`…`ts8`) kullanır; aynı dizini iki test paylaşmaz.
- **Ön yüz derleme uyumu (yalnızca Task 1):** yeni `ObjectKind`/`ObjectDef` değerleri TS tiplerini değiştirir; uyum **yalnızca tembel designer/schema-diff dosyalarında** (`FORMS` `formMissing` koruması, `KIND_LABELS`, `defName`, `FORM_PATHS`, `drop-model` tabloları, `SCHEMA_MOVE_COLLECTION` gerekmez); `ddl-kinds.ts` ve `ACL_KINDS` **değişmez** (menü Plan 35'te). Ana paket **tam 126,582 B**, başlangıç **154,255 B** kalmalı (`bun run bundle:check`; Bun PATH'te yoksa `npm run`/`npx`). Ana pakete giren bir dosya (ör. `data-opener.ts`) yalnız tip düzeyinde değişebilir; bayt artarsa durup raporla.
- `ts-rs`, `Box::pin`, git worktree yasağı; `cargo test` `src/ipc/bindings/*.ts`'i LF ile yeniden yazar → `git checkout -- src/ipc/bindings/` ile gürültü geri alınır, **yeni** binding dosyaları commit edilir (CRLF'e çevrilerek, depodaki diğer bindinglerle aynı satır sonu). Dosyalar CRLF'dir; düzenlemeler satır sonunu korur. Crate geneli `cargo fmt` yok. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Yorumlar Türkçe, çevredeki yoğunlukta. `graphify-out/` ve `.superpowers/` commit edilmez.

## Review Focus

- **Politika yeniden kurma doğruluğu:** komut/permissive değişimi ve ifade kaldırma `ALTER POLICY` ile yazılmaz (PG reddeder ya da sessizce ifadeyi korur); yeniden kurma aynı transaction'da, yeni adla, yorumla; yalnız ad değişimi tek `RENAME`; `roles` sıra farkı boş plan.
- **Sabit nokta:** `alter(loaded, loaded)` her türde **boş plan** — `pg_get_expr(polqual/polwithcheck)` metni, rol listesi sırası, `PUBLIC`, tablespace seçenekleri (`spcoptions` `name=value` metni), `location`, RLS bayrakları; normalleştirme kuralları belgeli.
- **Tablespace transaction dışı davranışı:** create ardından comment başarısızsa `failed.created` dolu ve `committed = 1`; `DROP TABLESPACE` boş değilken 55000 → ipucu, hiçbir şey değişmez; `pg_default`/`pg_global` alter/drop reddi.
- **RLS bayrakları:** ENABLE/FORCE alter'da Destructive (üretimde onay), DISABLE/NO FORCE Safe + uyarı; create'te Safe; bölüm/INHERITS çocuğunda izinli; şema karşılaştırması RLS farkını üretir ve boş tabloda farksızdır.
- **Hedefler ve kapsam:** `pol:P` yalnız geçerli tablo altında; `tablespaces`/`ts:N` yalnız sunucunun hemen altında; Tablespace yükleme/uygulama/ACL bakım veritabanında (`databaseOid = None`); günlükte ifade/konum/ad yok.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-ddl/src/model.rs` | `ObjectKind::{Policy, Tablespace}`, `PolicyCommand`, `PolicyDef`, `RowSecurity`, `TableDef.row_security`, `TablespaceOptions`, `TablespaceDef` |
| `crates/pgt-ddl/src/build/policy.rs` (+ snapshots) | Policy create/alter/drop, yeniden kurma |
| `crates/pgt-ddl/src/build/tablespace.rs` (+ snapshots) | Tablespace create/alter/drop, seçenek doğrulama |
| `crates/pgt-ddl/src/build/table.rs`, `table_alter.rs` | `row_security` create/alter ifadeleri |
| `crates/pgt-ddl/src/build/acl.rs`, `load_acl.rs`, `src/sql/load_acl_tablespace.sql` | Tablespace yetkileri |
| `crates/pgt-ddl/src/build/mod.rs` | Dispatch, `validate`, `validate_identity` |
| `crates/pgt-ddl/src/load.rs`, `src/sql/load_policy.sql`, `load_tablespace.sql`, `load_table.sql` | Yükleyiciler |
| `crates/pgt-ddl/src/target.rs` | `pol`/`policies`, `ts`/`tablespaces` eşlemeleri |
| `crates/pgt-ddl/src/manager.rs`, `error_codes.rs` | `find_created`, `defaults`, `identity_changed`, salt okunur kapısı, `hint_for` |
| `crates/pgt-ddl/tests/policy_roundtrip.rs` (yeni; policy + tablespace + RLS), `acl_roundtrip.rs`, `manager.rs`, `json.rs`, `logging.rs`, `common/mod.rs` | Docker gidiş-dönüş ve kapılar |
| `crates/pgt-diff/src/*` (+ testler) | `row_security` alanı derleme uyumu ve diff testi |
| `scripts/test-matrix.ps1` | Tablespace dizinleri + `PGT_TEST_TS_ROOT` |
| `src/features/designer/*`, `src/features/schema-diff/*` (yalnız tembel, Task 1) | Derleme uyumu |
| `docs/smoke-test.md`, `docs/testing.md`, spec'ler | Notlar, durum |

---

### Task 1: Model, hedefler ve tembel derleme uyumu

**Dosyalar:**
- Değiştir: `crates/pgt-ddl/src/model.rs`, `build/mod.rs` (geçici `Invalid("henüz desteklenmiyor")` kolları), `load.rs`, `manager.rs` (geçici kollar), `target.rs`; `TableDef` literal'leri olan tüm Rust dosyaları (`row_security: RowSecurity::default()`; `pgt-ddl` testleri, `pgt-diff`); `src/features/designer/{DesignerView.tsx, forms-model.ts, designer-model.ts, drop-model.ts}` ve TS hatası veren diğer **tembel** dosyalar; ilgili testler
- Test: `model.rs` serde testleri, `target.rs` birim testleri, `tests/json.rs`

**Arayüzler:**
- Üretir (`model.rs`; serde camelCase, enum'lar snake_case):

```rust
pub enum ObjectKind { /* mevcutlar */, Policy, Tablespace }
// server_scoped(): Database | Role | Tablespace
pub enum PolicyCommand { All, Select, Insert, Update, Delete }
#[derive(Default)] pub struct RowSecurity { pub enabled: bool, pub forced: bool }
pub struct PolicyDef {
    pub schema: String, pub table: String, pub name: String,
    pub command: PolicyCommand, pub permissive: bool, pub roles: Vec<String>,
    pub using: Option<String>, pub with_check: Option<String>, pub comment: Option<String>,
    pub editable: bool, pub unsupported: Vec<String>,
    #[serde(default)] pub table_rls: Option<RowSecurity>,      // yalnız yükleme/varsayılan; üretici yalnız uyarı için okur
}
pub struct TablespaceOptions {                                  // #[derive(Default)]; her alan sayısal METİN
    pub seq_page_cost: Option<String>, pub random_page_cost: Option<String>,
    pub effective_io_concurrency: Option<String>, pub maintenance_io_concurrency: Option<String>,
}
pub struct TablespaceDef {
    pub name: String, pub owner: Option<String>, pub location: String,
    pub options: TablespaceOptions, pub comment: Option<String>,
    pub editable: bool, pub unsupported: Vec<String>,
}
// TableDef'e: #[serde(default)] pub row_security: RowSecurity
// ObjectDef'e: Policy(PolicyDef), Tablespace(TablespaceDef); kind()/name() kolları
```
  (`with_check` → `withCheck`, `table_rls` → `tableRls`, `row_security` → `rowSecurity`.)
- Üretir (`target.rs`): Karar 9–10 eşlemeleri ve `refreshNodeId`'ler.
- Ön yüz (yalnız tembel): `FORMS` yeni türler için `formMissing` koruması (Plan 14/18 kalıbı), `KIND_LABELS` (`policy: "Politika"`, `tablespace: "Tablespace"`), `defName` (Policy: `table` bağlamlı ad; Tablespace: `name`), `FORM_PATHS` boş kümeler, `drop-model` tablosu (`policy`: cascade yok, force yok; `tablespace`: cascade yok, force yok). `ddl-kinds.ts` **değişmez**.

- [ ] **Adım 1: Başarısız testler.** Serde: `PolicyDef`/`TablespaceDef`/`RowSecurity` camelCase şekli, `ObjectKind` yeni değerler (`"policy"`, `"tablespace"`), `TableDef` `rowSecurity` eksikken `false/false` (geri uyum); `tests/json.rs` tür listesine iki tür; `server_scoped(Tablespace) == true`; `target.rs`: `srv:U/db:D/sch:S/tbl:T/pol:P` → `ObjectRef{Policy, P}` + `refreshNodeId = …/tbl:T/policies`, bölüm tablosu altındaki `pol` de eşlenir, `…/tbl:T/policies` → `DdlCreate{Policy, schemaOid S, tableOid T}`, `srv:U/tablespaces` → `create(Tablespace)` + `databaseOid None`, `srv:U/ts:N` → `object(Tablespace, N)`; `db:D/ts:N` gibi geçersiz konum boş.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula (üretici/yükleyici/yönetici kollarında geçici `Invalid`; derleme her yerde geçer).
- [ ] **Adım 4:** `cargo test -p pgt-ddl -p pgt-diff` (Docker'sız), `cargo clippy --workspace --all-targets -- -D warnings`; bindingler üretilir; `npm run typecheck`, `npm run test`, `npm run build`, `npm run bundle:check` (ana **126,582 B**, başlangıç **154,255 B**); `git checkout -- src/ipc/bindings/` gürültüsü; yeni binding dosyaları (CRLF) commit.
- [ ] **Adım 5: Commit** (`feat(ddl): Policy/Tablespace modelleri, tablo RLS alanı ve hedef eşlemeleri`).

---

### Task 2: Policy üreticisi

**Dosyalar:** Oluştur `crates/pgt-ddl/src/build/policy.rs` (+ snapshots); değiştir `build/mod.rs`.

**Arayüzler:**

```rust
pub fn create(ctx: &BuildCtx, d: &PolicyDef) -> Result<Plan, Vec<FieldError>>;
pub fn alter(ctx: &BuildCtx, old: &PolicyDef, new: &PolicyDef) -> Result<Plan, Vec<FieldError>>;
pub fn drop(d: &PolicyDef) -> Result<Plan, Vec<FieldError>>;
/// ALTER POLICY ile yapılamayan değişiklik var mı (Karar 3).
fn needs_rebuild(old: &PolicyDef, new: &PolicyDef) -> bool;
```
- `CREATE POLICY <q ad> ON <qualified tablo> AS {PERMISSIVE|RESTRICTIVE} FOR {ALL|SELECT|INSERT|UPDATE|DELETE} TO {PUBLIC | <q rol>, …} [USING (<ifade>\n)] [WITH CHECK (<ifade>\n)]` (`AS PERMISSIVE` her zaman yazılır; PG 10+ tabanda), ardından `COMMENT ON POLICY <q> ON <tablo> IS …`.
- Doğrulama: Karar 2; `editable = false` eski tanım → `object` hatası; `table`/`schema` değişimi alan hatası.
- Alter: Karar 3 sırası; `roles` karşılaştırması küme (sıra bağımsız); `ALTER POLICY` yalnız değişen parçaları yazar.
- [ ] **Adım 1: Başarısız testler (insta + birim):** create varyantları (ALL PERMISSIVE PUBLIC ifadesiz + uyarı; SELECT RESTRICTIVE iki rol USING; INSERT WITH CHECK; UPDATE her ikisi; tablo RLS kapalı uyarısı), kural ihlalleri (INSERT+USING, SELECT/DELETE+WITH CHECK, `public` öğesi, yinelenen rol), alter (yalnız ad → tek RENAME; roller → `ALTER POLICY … TO`; USING değişimi; ad + USING birlikte → RENAME sonra ALTER yeni adla; komut değişimi → DROP+CREATE Destructive + not; permissive değişimi; USING kaldırma → yeniden kurma; WITH CHECK ekleme → ALTER; roller yalnız sıra farkı → boş plan; yorum), drop (Destructive), hostile ad/rol/ifade (`;`, `)`, `--`, `'`, `\` reddi).
- [ ] **Adım 2–5:** uygula, `cargo test -p pgt-ddl`, clippy, commit (`feat(ddl): Policy üreticisi — PERMISSIVE/RESTRICTIVE, roller, aynı transaction'da yeniden kurma`).

---

### Task 3: Tablespace üreticisi ve tablo RLS ifadeleri

**Dosyalar:** Oluştur `crates/pgt-ddl/src/build/tablespace.rs` (+ snapshots); değiştir `build/mod.rs`, `build/table.rs`, `build/table_alter.rs` (+ snapshot'lar).

**Arayüzler:** `tablespace::{create(ctx, &TablespaceDef), alter(ctx, old, new), drop(&TablespaceDef)}` (aynı desen).
- Tablespace: Karar 5–8; tüm ifadeler `stmt_nt`; seçenek ifadeleri `SET (random_page_cost = '1.1', seq_page_cost = '1')` (ad sırası alfabetik), `RESET (effective_io_concurrency)`.
- Table: Karar 4 (create ve alter; `isPartition`/`inherits` izin listesi).
- [ ] **Adım 1: Başarısız testler (insta + birim):** tablespace create (sahipli/sahipsiz, seçenekli, yorumlu → iki `stmt_nt`), Windows ve POSIX konumu, göreli konum/boş/NUL reddi, tek tırnaklı konum doğru kaçışlı, `pg_` öneki reddi, seçenek doğrulama tablosu (negatif, `NaN`, `1e400`, 1001, `abc`), PG 12'de `maintenance_io_concurrency` reddi / PG 13'te serbest, alter (rename; set+reset ayrı; owner en sonda; location değişimi hatası), drop Destructive `stmt_nt`, salt okunur eski tanım reddi; table create `rowSecurity` (enabled, enabled+forced) Safe ve sıra; table alter ENABLE/FORCE Destructive + not, DISABLE/NO FORCE Safe + uyarı, bölüm tablosunda izinli, değişmeyen bayrak → ifade yok.
- [ ] **Adım 2–5:** uygula, `cargo test -p pgt-ddl`, clippy, commit (`feat(ddl): Tablespace üreticisi ve tablo satır güvenliği ifadeleri`).

---

### Task 4: Yükleyiciler, test matrisi dizinleri ve Docker gidiş-dönüşü

**Dosyalar:** Değiştir `load.rs`, `src/sql/load_table.sql`; oluştur `src/sql/load_policy.sql`, `load_tablespace.sql`; oluştur `crates/pgt-ddl/tests/policy_roundtrip.rs`; değiştir `tests/common/mod.rs` (`ts_root() -> Option<String>` yardımcısı: `PGT_TEST_TS_ROOT`), `tests/table_roundtrip.rs` (RLS), `scripts/test-matrix.ps1` (Global Constraints "Tablespace dizinleri"), `docs/testing.md` (değişkenin anlamı).

- Policy yükleme: `pg_policy` + tablo/şema; `polcmd` (`*`→all, `r`,`a`,`w`,`d`), `polpermissive`, `polroles` → `pg_authid`/`pg_roles` adları (0 → boş liste; ada göre `COLLATE "C"` sıralı), `pg_get_expr(polqual, polrelid)`, `pg_get_expr(polwithcheck, polrelid)`, yorum (`pg_description` `pg_policy` sınıfı), `table_rls` tablonun bayraklarından; tablo eklentiye aitse `unsupported` + salt okunur.
- Tablespace yükleme (bakım veritabanı): `pg_tablespace`, `pg_get_userbyid(spcowner)`, `pg_tablespace_location(oid)`, `spcoptions` (`name=value` → `TablespaceOptions`; bilinmeyen anahtar → `unsupported` + salt okunur), `pg_shdescription` yorumu, Karar 8.
- Table: `relrowsecurity`, `relforcerowsecurity`.
- [ ] **Adım 1: Başarısız testler (Docker, her sunucuda):** policy create → load → eşit ve `alter(loaded, loaded)` boş (PUBLIC; iki rol; RESTRICTIVE; USING/WITH CHECK karmaşık ifade (`current_user`, alt sorgu, `::` dönüşümü)); komut değişimi yeniden kurma uygulanır ve yorum korunur; USING kaldırma; yalnız ad değişimi; bölüm tablosunda politika; table RLS create/alter gidiş-dönüşü ve ENABLE sonrası RLS'nin etkili olduğu (sahip olmayan bir test rolüyle `SELECT count(*)` = 0; rol `guarded` ile silinir); **tablespace** (yalnız `ts_root()` varsa): create (seçenekli, yorumlu) → load → eşit, `alter(loaded, loaded)` boş, rename + set/reset + owner, içinde tablo varken drop → 55000 ve tablespace durur, boşken drop; `pg_default` ve `pg_global` salt okunur yüklenir.
- [ ] **Adım 2–4:** uygula; `cargo test --workspace` (Docker'sız), clippy, matris (`powershell -NoProfile -File scripts/test-matrix.ps1 -Versions 10,11,12,13,14,15,16,17,18 -SkipPerf`; kapsayıcılar silinir); tablespace testlerinin **atlanmadığını** çıktıdan doğrula.
- [ ] **Adım 5: Commit** (`feat(ddl): Policy/Tablespace yükleyicileri, tablo RLS bayrakları ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 5: Tablespace yetkileri (ACL)

**Dosyalar:** Değiştir `build/acl.rs`, `load_acl.rs`, `manager.rs` (ACL yolu sunucu kapsamı); oluştur `src/sql/load_acl_tablespace.sql`; değiştir `tests/acl_roundtrip.rs`.

- Karar 11: ayrıcalık kümesi `{Create}` dışındaki ayrıcalık alan hatası; nesne metni `TABLESPACE <q>`; `acldefault('t', spcowner)` ile varsayılan; sistem tablespace'leri (`pg_default`, `pg_global`) salt okunur değildir **yetki için** (PG `GRANT CREATE ON TABLESPACE pg_default` geçerlidir) — ancak `pg_global` için `GRANT` PG tarafından anlamsızdır: `pg_global` ACL'i salt okunur (`unsupported`).
- Varsayılan yetkilerde tablespace isteği alan hatası (`kind`).
- [ ] **Adım 1: Başarısız testler:** birim (nesne metni tablosuna `(Tablespace, "TABLESPACE t")`, ayrıcalık doğrulaması, varsayılan yetki reddi); Docker (yalnız `ts_root()` varsa): GRANT CREATE → load → eşit; REVOKE; `WITH GRANT OPTION`; bakım veritabanında uygulandığı.
- [ ] **Adım 2–5:** uygula, test, clippy, matris (ACL testleri), commit (`feat(ddl): tablespace yetkileri (GRANT CREATE ON TABLESPACE)`).

---

### Task 6: Yönetici entegrasyonu ve kapılar

**Dosyalar:** Değiştir `manager.rs`, `error_codes.rs` (+ `tests/manager.rs`, `tests/logging.rs`).

- `find_created`: Policy `tablo oid + ad`; Tablespace `spcname`. `defaults` (Karar 10; Policy için bağlam tablosundan `schema`/`table`/`table_rls` yüklenir). `identity_changed`: Policy ad, Tablespace ad. Nesne denetimi (yeniden yükleme eşitliği) iki tür için. Salt okunur kapısı (R5) yeni türleri kapsar. `hint_for`: `55000` + `DROP TABLESPACE` → "Tablespace boş değil; önce içindeki nesneleri başka tablespace'e taşıyın (başka veritabanlarındaki nesneler de sayılır)."; `42501` + `CREATE TABLESPACE` → "Tablespace oluşturmak süper kullanıcı yetkisi ister."; `58P01`/`42P17`/`22023` + `CREATE TABLESPACE` (dizin yok/göreli/boş değil) → PG iletisi olduğu gibi + ipucu "Konum sunucuda var olan, PostgreSQL hizmet kullanıcısına ait boş bir dizin olmalı."; `42710` + `CREATE POLICY` → PG iletisi (özel ipucu yok).
- [ ] **Adım 1: Başarısız testler (Docker):** `load_and_apply_targets_agree_for_every_kind` yeni türlerle (tablespace yalnız `ts_root()` varsa); create → `find_created` doğru oid; politika yeniden adlandırmada `created`; tablespace create + comment başarısız senaryosu (ör. yorum ifadesini bozmak mümkün değilse: `COMMENT` öncesi tablespace'i başka oturumdan silerek değil — **birim düzeyde** `stmt_nt` sırası ve `failed.created` eşlemesi test edilir; Docker'da yalnız mutlu yol); kilitli sunucuda iki tür `locked`; Destructive (politika drop/yeniden kurma, RLS ENABLE, tablespace drop) `confirmation_required` ve üretimde `production_confirmation_required`; bayat `old` → `plan_changed`; `pg_default` alter/drop reddi; 55000 ipucu; günlükte ifade/konum/ad yok (`logging.rs` kalıbı: ayırt edici sır dizgeleri USING ifadesinde ve konum yolunda).
- [ ] **Adım 2–4:** uygula; `cargo test --workspace`, clippy, matris. **Adım 5: Commit** (`feat(ddl): Policy/Tablespace yönetici entegrasyonu — find_created, varsayılanlar, kapılar, ipuçları`).

---

### Task 7: Belgeler, matris ve bütünleşik doğrulama

**Dosyalar:** Değiştir `docs/smoke-test.md`, `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md`, `docs/testing.md`.

- [ ] **Adım 1:** Tam Docker matrisi `powershell -NoProfile -File scripts/test-matrix.ps1 -Versions 10,11,12,13,14,15,16,17,18 -SkipPerf` — yeşil; atlanan test yok (tablespace dahil).
- [ ] **Adım 2: `docs/smoke-test.md`** — "Nesne diyalogları: Policy / Tablespace arka ucu (Plan 34)" bölümü (GUI maddeleri Plan 35'te `[—]`); bilinen sınırlar (politika yeniden kurma, tablespace transaction dışı, yerinde tablespace yok, şema karşılaştırması RLS bayraklarını karşılaştırır ama politikaları karşılaştırmaz).
- [ ] **Adım 3: Spec** — durum "Plan 34 arka ucu uygulandı; Plan 35 ön yüzü kalan"; **§9 Plan 34 sözleşme notları (Plan 35 için)**: model alanları (camelCase), `editable`/`unsupported` zorunluluğu (serde varsayılanı yok), `tableRls` ve `rowSecurity`, alan yolları (`name`, `table`, `command`, `roles`, `roles[i]`, `using`, `withCheck`, `comment`, `object`, `location`, `options.seqPageCost|randomPageCost|effectiveIoConcurrency|maintenanceIoConcurrency`, `owner`, `rowSecurity.enabled|forced`, `old.*`), uyarı metinleri, ipucu kodları, yeniden kurma koşulları, ACL tablespace (`{Create}`), `defaults` davranışı, ön yüz bütçe notu (ana 126,582 B; Plan 35 menü kancası telafi ister).
- [ ] **Adım 4: Bütünleşik doğrulama.** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `npm run test`, `npm run typecheck`, `npm run build`, `npm run bundle:check` (ana **126,582 B**, başlangıç **154,255 B**).
- [ ] **Adım 5: Kapanış greps:** `rg "tracing::" crates/pgt-ddl/src/build/policy.rs crates/pgt-ddl/src/build/tablespace.rs` boş; `rg "location" crates/pgt-ddl/src/manager.rs` yeni günlük/hata çağrısı yok.
- [ ] **Adım 6: Commit** (`docs(ddl): Plan 34 sözleşme notları, smoke ve bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 34 bitişi)

- İki tür + tablo RLS + tablespace ACL için create/alter/drop/load PG 10–18'de yeşil; sabit nokta her türde; tablespace testleri atlanmadı.
- Politika yeniden kurma ve tablespace transaction dışı davranışı testli; salt okunur sistem tablespace'lerinde değişiklik yok; günlükte ifade/konum/ad yok.
- Ana paket **126,582 B**, başlangıç **154,255 B** (değişmedi).
- Elle kontrol maddeleri Plan 35'tedir; `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §2.1 Policy (alanlar → T1; kurallar/alter/drop → T2; RLS durumu uyarısı → T2/T4; tablo bayrakları → T3/T4; hedef → T1; lookup roller mevcut), §2.2 Tablespace (alanlar → T1; doğrulama/ifadeler → T3; salt okunur → T4/T6; izin ipucu → T6; ACL → T5), §2.3 risk (T2/T3), §1 gizlilik/transaction dışı (Global Constraints, T3/T6), §6 güvenlik (T6 günlük), §7 test (T2–T6; tablespace dizinleri T4), §5 ön yüz Plan 35.
- **Bilinçli kararlar:** `cascade` politikada yok sayılır; şema karşılaştırması RLS bayraklarını kazanır, politikaları değil; tablespace testleri ortam değişkeni yoksa atlanır (matris betiği her zaman sağlar).
- **Yer tutucu taraması:** Task 1'deki geçici `Invalid` kolları Task 2–6'da kalkar (açıkça belirtilmiş ara durum).
- **Tip tutarlılığı:** `PolicyDef.table_rls`/`RowSecurity` (T1) → uyarı (T2) → yükleyici (T4) → `defaults` (T6); `TableDef.row_security` (T1) → `table.rs`/`table_alter.rs` (T3) → `load_table.sql` (T4) → `pgt-diff` (T1); `TablespaceOptions` (T1) → `tablespace.rs` (T3) → yükleyici (T4); `server_scoped` (T1) → hedef (T1) → ACL (T5) → yönetici (T6).
