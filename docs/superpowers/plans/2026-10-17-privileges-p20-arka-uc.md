# Yetkiler (GRANT/REVOKE) ve Varsayılan Yetkiler — Arka Uç (Plan 20) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** `pgt-ddl`'e nesne yetkilerinin (GRANT/REVOKE, `WITH GRANT OPTION`, `CASCADE`) ve varsayılan yetkilerin (`ALTER DEFAULT PRIVILEGES`) **arka ucunu** eklemek: model, yetki kataloğu, saf üreticiler, katalog yükleyicileri, yönetici entegrasyonu (mevcut kapılar), iki yeni Tauri komutu ve PG 10–18 Docker matrisi. **Ön yüz bu planda değil** (Plan 21); bu plan ön yüze yalnızca **tembel derleme uyumu** için dokunur (Task 1).

**Mimari:** Plan 10–19 kalıbı: istemci SQL göndermez; yüklenen `AclDef`/`DefaultAclDef`'in düzenlenmiş kopyasını gönderir; arka uç **fark** üretir (`build::acl`, `build::default_acl`); `DdlManager` aynı kapılarla uygular (kilit → salt okunur → hedef → plan → **yeniden yükleme eşitliği** → hash → onay → üretim onayı → lock_timeout → çalıştır). Yenilikler: (1) `PlanRequest`'e iki varyant (`Privileges`, `DefaultPrivileges`) ve nesne denetiminin bu varyantlar için genelleştirilmesi; (2) **etkin ACL** — NULL ACL `acldefault(...)` ile açık gösterilir; (3) **veren (grantor) ayrımı** — yalnız sahibin verdiği girdiler düzenlenir; (4) uygulama sonrası ACL denetimi ve uyarı.

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta, mevcut Docker matrisi (`scripts/test-matrix.ps1`).

**Spec:** `docs/superpowers/specs/2026-10-17-privileges-design.md` (§1 kararlar, §2 model, §4 güvenlik, §5 test). Temel spec: `docs/superpowers/specs/2026-10-06-object-dialogs-design.md`. Önceki planlar: `2026-10-06-object-dialogs-p10-arka-uc.md`, `2026-10-07-object-dialogs-p14-view-database-role.md`, `2026-10-16-object-dialogs-p18-routines-types-arka-uc.md` — burada tekrarlanmayan **tüm Global Constraints ve Review Focus** maddeleri bağlayıcıdır.

## Kararlar

1. **Tür kapsamı:** `ObjectKind::{Table, View, MaterializedView, Sequence, Function, Procedure, Schema, Database, Type, Domain}` (`acl_supported(kind) -> bool`). Diğer türler (Index, Extension, Role, Trigger) → `Invalid("Bu nesne türünün yetkileri yoktur.")`. Bölümlenmiş tablo ve bölümler `Table`'dır. Yabancı tablo, tablespace, dil, FDW, large object, parametre ve `GRANTED BY` backlog.
2. **Yetki kümeleri (`privileges_for(kind, version) -> &'static [Privilege]`):** Table/View/MV: `Select, Insert, Update, Delete, Truncate, References, Trigger` + PG 17+ `Maintain`; Sequence: `Usage, Select, Update`; Function/Procedure: `Execute`; Schema: `Usage, Create`; Database: `Create, Temporary, Connect`; Type/Domain: `Usage`. Kümede olmayan yetki → alan hatası `grants[i].privileges[j].privilege` ("Bu nesne türünde geçersiz yetki."). Sürüm kapısı: PG < 17 `Maintain` → alan hatası (sessiz atlama yok). Kanonik sıra bu dizilerdeki sıradır (yükleyici ve üretici aynı sırayı kullanır).
3. **ACL harfleri (`aclexplode` `privilege_type` metni ↔ `Privilege`):** `SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER, MAINTAIN, USAGE, EXECUTE, CREATE, CONNECT, TEMPORARY`. Bilinmeyen metin (gelecek sürüm) → `unsupported` girdisi `"yetki <metin>"` ve `editable = false`.
4. **Etkin ACL ve `explicit`:** yükleyici `aclexplode(coalesce(<acl>, acldefault('<harf>', <sahip>)))` kullanır (`r` ilişki — dizi için `s`; `f` fonksiyon; `n` şema; `d` veritabanı; `T` tip). `explicit = <acl> IS NOT NULL`. Üretici `explicit`'i karşılaştırmaz (bilgi alanı); fark yalnız `grants` üzerindendir.
5. **Veren ayrımı:** `grantor = sahip` olan girdiler `grants`'a (alıcı başına tek `AclGrant`, yetkiler kanonik sırada); `grantor ≠ sahip` olanlar `foreign`'a (salt okunur; üretici `old.foreign != new.foreign` ise alan hatası `foreign`). Kolon ACL'leri (`pg_attribute.attacl`, yalnız Table/View/MV) `columns`'a (salt okunur; değişirse alan hatası `columns`). `grants` alıcıya göre sıralıdır: `PUBLIC` (None) önce, sonra ad (`COLLATE "C"`); yükleyici ve üretici aynı sıralamayı kullanır.
6. **Fark ve ifade sırası (alıcı sırasıyla, her alıcı için):**
   1. `REVOKE p, … ON <hedef> FROM <alıcı>[ CASCADE]` — eski tanımda olup yenide olmayan yetkiler.
   2. `REVOKE GRANT OPTION FOR p, … ON <hedef> FROM <alıcı>[ CASCADE]` — yenide kalan ama `grantOption` true→false.
   3. `GRANT p, … ON <hedef> TO <alıcı>` — yeni eklenen ve `grantOption = false`.
   4. `GRANT p, … ON <hedef> TO <alıcı> WITH GRANT OPTION` — yeni eklenen `grantOption = true` **ve** var olup false→true.
   Önce tüm alıcıların REVOKE'ları (1–2), sonra tüm GRANT'lar (3–4). Boş liste ifade üretmez. `ALL PRIVILEGES` yazılmaz. Alıcı `PUBLIC` ya da `quote_ident(ad)`; hedef `TABLE <qualified>` (View/MV da `TABLE`), `SEQUENCE <qualified>`, `FUNCTION|PROCEDURE <qualified>(<signature>)`, `SCHEMA <q>`, `DATABASE <q>`, `TYPE <qualified>`, `DOMAIN <qualified>`.
7. **Risk:** GRANT → Safe; REVOKE ve REVOKE GRANT OPTION FOR → **Destructive**; `cascade = true` iken REVOKE'lar → **DataLoss** (not: "Alıcının bu yetkiyle başkalarına verdiği yetkiler de geri alınır."). `cascade` hiç REVOKE yokken yok sayılır (uyarı yok). Varsayılan yetkiler: tüm ifadeler Safe + plan notu "Yalnızca bundan sonra oluşturulan nesneleri etkiler; mevcut nesnelerin yetkileri değişmez."
8. **Sahip satırı:** sahip alıcı olarak `grants`'ta yer alır (etkin ACL'de varsayılan olarak tüm yetkiler). Sahip satırında `grantOption` değişimi alan hatası (`grants[i].privileges[j].grantOption`: "Sahip, yetkileri zaten devredebilir; GRANT OPTION ayarlanamaz." — PG 0LP01'i önden yakalar). Sahibin kendi yetkisini kaldırması → plan uyarısı ("Sahip kendi yetkisini kaldırıyor; sahip bu yetkiyi istediği an geri verebilir.").
9. **Kendini kilitleme uyarıları (engellemez):** `viewer`'dan (bağlı rol) yetki kaldırma → "Bağlı rol bu nesne üzerindeki yetkisini kaybedebilir."; Database'de `PUBLIC` ya da `viewer`'dan `CONNECT` kaldırma → "Bu veritabanına bağlanma yetkisi kaldırılıyor; bağlı rol (ve PUBLIC üzerinden bağlananlar) yeniden bağlanamayabilir."; Schema'da `PUBLIC`/`viewer`'dan `USAGE` kaldırma → "Şemadaki nesnelere erişim kesilebilir.". Uyarılar ad/değer içermez.
10. **Alıcı doğrulaması:** `grantee` `Some(ad)` → `check_ident`; aynı alıcı iki kez → alan hatası `grants[i].grantee` ("Bu alıcı zaten listede."); boş `privileges` olan satır = alıcının tüm yetkileri kaldırılır (geçerli); `PUBLIC` küçük/büyük harf `Some("public")` → alan hatası ("PUBLIC için alıcıyı boş bırakın.") — PG `public` adlı rol oluşturmaya izin vermez, ad ile yazmak anlam değiştirir.
11. **Salt okunur (`editable = false`):** sistem şemasındaki nesneler ve sistem şemaları (`pg_catalog`, `information_schema`, `pg_toast*`), dizi tipleri, multirange tipleri, tablo satır tipleri (composite `relkind <> 'c'`), bilinmeyen yetki harfi. Alter isteği yönetici kapısında reddedilir (Plan 18 `read_only_refusal` kalıbı; gerekçe sabit metinler). Eklentiye ait nesnelerde yetki düzenlemeye **izin verilir** (PG `pg_init_privs` izler).
12. **Varsayılan yetkiler:**
    - Nesne türleri `DefaultAclObjectType::{Tables, Sequences, Functions, Types, Schemas}` (PG harfleri `r, S, f, T, n`); `Schemas` yalnız genel kapsamda (şema kapsamında bölüm yok; gelirse alan hatası). Yetki kümeleri Karar 2'deki Table / Sequence / Function / Type / Schema kümeleri (Tables'ta PG 17+ `Maintain`).
    - Genel kapsam: her tür için etkin değer `aclexplode(coalesce(defaclacl, acldefault('<harf>', <rol oid>)))` (`defaclnamespace = 0`); `GRANT` ve `REVOKE` serbest (ör. fonksiyonlar için `PUBLIC EXECUTE` kaldırma).
    - Şema kapsamı: `grants` yalnız o şemanın `pg_default_acl` girdileri (yoksa boş); `baseline` genel kapsamın etkin değeri (salt okunur; değişirse alan hatası `sections[i].baseline`). REVOKE yalnız şema girdisinde olanı kaldırır.
    - Bölümler her zaman sabit sırayla ve uygun türlerin tamamıyla gelir (Tables, Sequences, Functions, Types[, Schemas]); eksik/fazla/yinelenen bölüm → alan hatası `sections`.
    - İfadeler: `ALTER DEFAULT PRIVILEGES FOR ROLE <q>[ IN SCHEMA <q>] {GRANT p, … ON <TÜR> TO <alıcı>[ WITH GRANT OPTION] | REVOKE [GRANT OPTION FOR ]p, … ON <TÜR> FROM <alıcı>}` — Karar 6 sırası, bölüm sırasıyla. `CASCADE` yok (PG kabul eder ama anlamsız).
    - Sahip satırı burada sahip = `role`; Karar 8'in GRANT OPTION kuralı geçerli.
13. **İstek ve kapılar:** `PlanRequest::Privileges { object: ObjectRef, old: AclDef, new: AclDef, cascade: bool }` (`op = "privileges"`), `PlanRequest::DefaultPrivileges { scope: DefaultAclScope, old: DefaultAclDef, new: DefaultAclDef }` (`op = "default_privileges"`). Hedef: `Privileges` → `target_for(object.kind, …)` (Database → bakım bağlantısı); `DefaultPrivileges` → veritabanı kapsamlı (`target_for(ObjectKind::Schema, …)` ile aynı kural). Nesne denetimi: `old` yeniden yüklenen tanıma **birebir** eşit olmalı (`viewer` dahil), değilse `PLAN_CHANGED`. `old.kind != object.kind` ya da yeni/eski kimlik alanları (`kind, schema, name, signature, owner, viewer, explicit`; varsayılanda `role, schema, viewer`) farklı → `INVALID_DEF`.
14. **Uygulama sonrası denetim:** başarılı uygulamadan sonra ACL (ya da varsayılan ACL) yeniden yüklenir; `grants` (varsayılanda `sections[*].grants`) istenenden farklıysa `ApplyOutcome::Applied.warnings`'e `ACL_WARNING` ("Yetkiler uygulama sonrasında istenenden farklı: bağlı rol nesnenin sahibi değilse yetkiler onun adıyla kaydedilir ya da bazı GRANT/REVOKE işlemleri etkisiz kalır (PG uyarısı). Yetkileri yeniden yükleyip kontrol edin."). Yeniden yükleme hatası uyarı üretmez (en iyi çaba).
15. **İpuçları (`hint_for`, yalnız PG ipucu yoksa; `op` ile):** `2BP01` + `privileges` → `ACL_DEPENDENTS_HINT` ("Alıcı bu yetkiyi başkalarına devretmiş; 'Bağımlı yetkilerle birlikte geri al (CASCADE)' seçin."); `42501` + `privileges` → `ACL_PRIVILEGE_HINT` ("Yetki vermek/geri almak için nesnenin sahibi, sahip rolün üyesi ya da ilgili yetkide GRANT OPTION sahibi olmak gerekir."); `42501` + `default_privileges` → `DEFAULT_ACL_PRIVILEGE_HINT` ("FOR ROLE için o rolün üyesi (ya da süper kullanıcı) olmak gerekir."); `42704` (rol yok) + ikisi → `ACL_ROLE_MISSING_HINT` ("Alıcı rol bulunamadı; rol listesini yenileyin."). Genel `("42501", _)` kolundan **önce**.

## Global Constraints

- Plan 10/12/14/18 Global Constraints aynen: PG **10–18** vanilla; tüm tanımlayıcı yerleştirmeleri yalnızca `quote_ident`/`qualified`; ad alanlarında `check_ident`; yüklü (eski) taraf yalnız kimlik alanlarıyla gevşek doğrulanır; `Statement.sql` sonda `;` taşımaz ve tek ifadedir; GRANT/REVOKE/ALTER DEFAULT PRIVILEGES **transactional** (tek blok).
- **İmza metni:** `signature` yalnız Function/Procedure'de dolu: `pg_get_function_identity_arguments(oid)`; üretici bunu yalnızca yüklenen tanımdan (nesne denetimiyle doğrulanmış) yazar, eski ≠ yeni ise `INVALID_DEF`; nihai ifade `sqlscan::composed_is_clean` ile taranır (temiz değilse `Invalid`). NUL/denetim karakteri içeren imza → yükleyici `editable = false` (`unsupported: "imza"`).
- **Günlük/gizlilik:** rol adları, yetki listeleri, SQL, imza ve URL asla `tracing`'e, hata iletisine ya da diske yazılmaz (yalnız `op`, `kind`, sayılar, sonuç kodu).
- **Docker:** yalnızca `scripts/test-matrix.ps1` (`pgt-matrix-<v>` geçici kapsayıcılar; bitince silinir); başka kapsayıcılara (hub-broker, arniva-report vb.) dokunulmaz; `PGT_TEST_URLS` yalnızca test komutunun ortamında; URL/parola yazdırılmaz/günlüğe alınmaz. Test nesneleri benzersiz önekli şemada (`pgt_ddl_rt_<etiket>`), test rolleri `pgt_acl_rt_<etiket>_<n>` (küme genelidir: `guarded` temizlikte her veritabanında `DROP OWNED BY` + `DROP ROLE IF EXISTS`).
- **Ön yüz derleme uyumu (yalnızca Task 1):** yeni `PlanRequest` varyantları TS birleşimini genişletir; uyum **yalnızca tembel designer dosyalarında** sağlanır (kapsamlı `switch`/`never` kontrolleri varsa). `src/ipc/commands.ts`'e **komut eklenmez** (Plan 21, telafiyle). Ana `index-*.js` **tam 231,724 B** kalmalı; `index.html` yalnızca solid/web/Icon; ana `__vite__mapDeps` 21 girdi.
- `ts-rs` (yeni tipler `#[ts(export)]`, camelCase), `Box::pin` gerekmedikçe yok, git worktree yasağı yok (SDD dalında çalışılır), `src/ipc/bindings` satır sonu gürültüsü `git checkout -- src/ipc/bindings`; yeni binding dosyaları commit edilir. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Yorumlar Türkçe, çevredeki yoğunlukta.

## Review Focus

- **NULL ACL ve ilk değişiklik:** varsayılan ACL'li fonksiyonda `PUBLIC EXECUTE` kaldırma → tek `REVOKE EXECUTE … FROM PUBLIC`; uygulama sonrası `explicit = true`, sahip satırı korunur, `alter(loaded, loaded)` boş plan; veritabanında `PUBLIC CONNECT, TEMPORARY` varsayılanı doğru gösterilir.
- **Veren karışıklığı:** `WITH GRANT OPTION` almış rolün verdiği yetki `foreign`'da görünür ve hiçbir planla değişmez; aynı alıcının hem sahipten hem başka verenden yetkisi olduğunda sahip girdisini kaldırmak yalnız sahip girdisini kaldırır (PG davranışı) ve uygulama sonrası uyarı üretmez; bağlı rol sahip değilken GRANT → yetki `foreign`'a düşer → `ACL_WARNING`.
- **CASCADE / RESTRICT:** bağımlı yetki varken `cascade = false` → PG 2BP01 + `ACL_DEPENDENTS_HINT`, hiçbir şey değişmez (tek transaction); `cascade = true` → DataLoss, üretimde ad yazma onayı; `REVOKE GRANT OPTION FOR` da aynı kurallar.
- **Hostile adlar ve imzalar:** alıcı/şema/nesne adlarında `"`, boşluk, Unicode, büyük harf; fonksiyon imzasında tırnaklı tip adı ve `VARIADIC`/`OUT` (PG 14+ procedure) — tek ifade, doğru nesne (aşırı yüklenmiş iki fonksiyondan yalnız biri değişir).
- **Varsayılan yetkiler:** şema kapsamında genelden gelen `baseline` değiştirilemez; `FOR ROLE` başka rol için yetkisiz bağlantıda 42501 + ipucu; `Schemas` bölümü şema kapsamında yok; yeni oluşturulan tabloya varsayılan yetki gerçekten uygulanır.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-ddl/src/acl.rs` (yeni) | Model: `Privilege`, `PrivilegeGrant`, `AclGrant`, `ForeignGrant`, `ColumnGrant`, `AclDef`, `DefaultAclObjectType`, `DefaultAclSection`, `DefaultAclDef`, `DefaultAclScope`; `acl_supported`, `privileges_for`, `default_privileges_for`, harf eşlemeleri, kanonik sıralama |
| `crates/pgt-ddl/src/build/acl.rs` (+ `snapshots/`) | Nesne ACL farkı üreticisi |
| `crates/pgt-ddl/src/build/default_acl.rs` (+ `snapshots/`) | Varsayılan yetki farkı üreticisi |
| `crates/pgt-ddl/src/load_acl.rs` (yeni), `src/sql/load_acl_*.sql`, `load_default_acl*.sql` | Yükleyiciler |
| `crates/pgt-ddl/src/manager.rs` | `PlanRequest` varyantları, nesne denetimi, salt okunur kapısı, ipuçları, uygulama sonrası denetim, `load_acl`/`load_default_acl` |
| `src-tauri/src/ddl.rs`, `src-tauri/src/lib.rs` | `ddl_load_acl`, `ddl_load_default_acl` komutları |
| `crates/pgt-ddl/tests/acl_roundtrip.rs` (+ `common/mod.rs` rol yardımcıları), `tests/manager.rs`, `tests/logging.rs`, `tests/json.rs` | Docker gidiş-dönüş ve kapılar |
| `src/features/designer/*` (yalnız tembel, Task 1) | Derleme uyumu |
| `docs/smoke-test.md`, spec'ler, `docs/testing.md` | Notlar, durum |

---

### Task 1: Model, yetki kataloğu, istek varyantları ve tembel derleme uyumu

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/acl.rs`
- Değiştir: `crates/pgt-ddl/src/lib.rs` (`pub mod acl;`), `manager.rs` (`PlanRequest` varyantları; `plan_view`/kapı kollarında geçici `Invalid("henüz desteklenmiyor")`), `tests/json.rs`; gerekirse `src/features/designer/*` (yalnız tembel)
- Test: `acl.rs` birim testleri, `tests/json.rs`

**Arayüzler — Üretir (`acl.rs`):**

```rust
#[serde(rename_all = "snake_case")]
pub enum Privilege { Select, Insert, Update, Delete, Truncate, References, Trigger, Maintain, Usage, Execute, Create, Connect, Temporary }
impl Privilege {
    pub fn sql(self) -> &'static str;                         // "SELECT", …, "TEMPORARY"
    pub fn from_acl(text: &str) -> Option<Privilege>;         // aclexplode privilege_type
}
#[serde(rename_all = "camelCase")]
pub struct PrivilegeGrant { pub privilege: Privilege, pub grant_option: bool }
pub struct AclGrant { pub grantee: Option<String> /* None = PUBLIC */, pub privileges: Vec<PrivilegeGrant> }
pub struct ForeignGrant { pub grantor: String, pub grantee: Option<String>, pub privilege: Privilege, pub grant_option: bool }
pub struct ColumnGrant { pub column: String, pub grantor: String, pub grantee: Option<String>, pub privilege: Privilege, pub grant_option: bool }
pub struct AclDef {
    pub kind: ObjectKind, pub schema: Option<String> /* Schema/Database: None */, pub name: String,
    pub signature: Option<String> /* yalnız Function/Procedure */, pub owner: String, pub viewer: String,
    pub explicit: bool, pub grants: Vec<AclGrant>, pub foreign: Vec<ForeignGrant>, pub columns: Vec<ColumnGrant>,
    pub editable: bool, pub unsupported: Vec<String>,
}
#[serde(rename_all = "snake_case")]
pub enum DefaultAclObjectType { Tables, Sequences, Functions, Types, Schemas }
pub struct DefaultAclSection { pub object_type: DefaultAclObjectType, pub grants: Vec<AclGrant>, pub baseline: Vec<AclGrant> }
pub struct DefaultAclDef { pub role: String, pub schema: Option<String>, pub viewer: String, pub sections: Vec<DefaultAclSection>, pub editable: bool, pub unsupported: Vec<String> }
pub struct DefaultAclScope { pub role: Option<String> /* None → bağlı rol */, #[ts(type = "number | null")] pub schema_oid: Option<u32> }

pub fn acl_supported(kind: ObjectKind) -> bool;                                      // Karar 1
pub fn privileges_for(kind: ObjectKind, version_num: u32) -> &'static [Privilege];    // Karar 2 (sürüme göre MAINTAIN)
pub fn default_privileges_for(t: DefaultAclObjectType, version_num: u32) -> &'static [Privilege];
pub fn default_sections(schema_scoped: bool) -> &'static [DefaultAclObjectType];      // Karar 12 sırası
pub fn acldefault_letter(kind: ObjectKind) -> char;                                   // Karar 4
pub fn sort_grants(grants: &mut [AclGrant], order: &[Privilege]);                     // Karar 5: PUBLIC önce, ad (bayt) sırası; yetkiler kanonik
```
  (Tüm yapılar `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]`, `#[serde(rename_all = "camelCase")]`, `#[ts(export)]`.)
- **Üretir (`manager.rs`):** `PlanRequest::Privileges { object: ObjectRef, old: AclDef, new: AclDef, cascade: bool }`, `PlanRequest::DefaultPrivileges { scope: DefaultAclScope, old: DefaultAclDef, new: DefaultAclDef }`; `op()` → `"privileges"`, `"default_privileges"`; `kind()` → `object.kind` / `ObjectKind::Schema` (veritabanı kapsamlı vekil; belgelenir).

- [ ] **Adım 1: Başarısız testler.** Serde: `Privilege` snake_case (`"temporary"`), `AclDef` camelCase (`grantOption`, `schemaOid`), `PlanRequest` `{"op":"privileges",…}` ve `{"op":"default_privileges",…}` gidiş-dönüşü (`tests/json.rs`); `privileges_for` her tür ve PG 16/17 `Maintain` farkı; `acl_supported` (Index/Extension/Role/Trigger false); `Privilege::from_acl` tüm 13 metin + bilinmeyen `None`; `sort_grants` (PUBLIC önce, `"B" < "a"` bayt sırası, yetkiler kanonik); `default_sections(true)` Schemas'sız.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula (yeni varyantlar için `plan_view`/`apply` kollarında geçici `Invalid`; derleme her yerde geçer; `read_only_refusal`, `object_check`, `HintCtx::of` kalıpları yeni varyantlarda derlenir).
- [ ] **Adım 4:** `cargo test -p pgt-ddl` (Docker'sız), `cargo clippy --workspace --all-targets -- -D warnings`; bindingler üretilir; `bun run typecheck`, `bun run test`, `bun run build` (**ana 231,724 B**, mapDeps 21, preload solid/web/Icon); `git checkout -- src/ipc/bindings` gürültüsü; yeni binding dosyaları commit.
- [ ] **Adım 5: Commit** (`feat(ddl): yetki modeli, yetki kataloğu ve PlanRequest Privileges/DefaultPrivileges varyantları`).

---

### Task 2: Nesne ACL farkı üreticisi

**Dosyalar:** Oluştur `crates/pgt-ddl/src/build/acl.rs` (+ `snapshots/`); değiştir `build/mod.rs` (`pub mod acl;`).

**Arayüzler — Üretir:**

```rust
/// GRANT/REVOKE hedefi (`TABLE s.t`, `FUNCTION s.f(integer)`, `DATABASE d` …).
pub fn object_target(d: &AclDef) -> Result<String, Vec<FieldError>>;
pub fn privileges(ctx: &BuildCtx, old: &AclDef, new: &AclDef, cascade: bool) -> Result<Plan, Vec<FieldError>>;
```
- Kurallar: Karar 2, 5–10. Doğrulama: `acl_supported(old.kind)`; kimlik alanları (`kind, schema, name, signature, owner, viewer, explicit`) eşit değil → `object` alan hatası; `old.editable == false` → `object` hatası; `foreign`/`columns` değişimi → hata; alıcı kuralları (Karar 10); yetki kümesi ve sürüm (Karar 2); bir alıcının aynı yetkisi iki kez → `grants[i].privileges[j]` hatası; sahip `grantOption` (Karar 8).
- Fark eşleşmesi alıcıya göre (`Option<String>` eşitliği); eski tanımda olup yenide hiç olmayan alıcı = tüm yetkilerini kaldır.
- Risk/uyarı/not: Karar 7–9; `PlanView` notları mevcut `Plan.warnings`/`notes` alanlarıyla (Plan 18 kalıbı).
- Nihai her ifade `composed_is_clean` ile taranır (yalnız Function/Procedure imzası dış kaynaklı; diğerlerinde de ucuz savunma).

- [ ] **Adım 1: Başarısız testler (insta + birim):** her tür için hedef metni (Table, View `TABLE`, MV `TABLE`, Sequence, Function, Procedure, Schema `SCHEMA "s"`, Database, Type, Domain); fark anlık görüntüleri: yalnız GRANT; yalnız REVOKE (Destructive); GRANT OPTION ekleme (var olan yetkiye → yalnız `GRANT … WITH GRANT OPTION`); GRANT OPTION kaldırma (`REVOKE GRANT OPTION FOR`); karışık çok alıcı (sıra: tüm REVOKE'lar sonra GRANT'lar, PUBLIC önce); alıcının tamamen silinmesi; yeni alıcı; `cascade` (DataLoss + not; REVOKE'suz planda yok sayılır); boş fark → boş plan; `alter(x, x)` boş; hatalar tablosu (geçersiz yetki, PG 16 `Maintain`, yinelenen alıcı, `Some("PUBLIC")`/`Some("public")`, sahip `grantOption`, `foreign` değişimi, `columns` değişimi, kimlik değişimi, `editable = false`, desteklenmeyen tür); uyarılar (sahip kendini, viewer, Database CONNECT PUBLIC, Schema USAGE viewer); hostile adlar (`"`, Unicode, boşluk) tek ifade; imzada `;`/NUL → `Invalid`.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula. **Adım 4:** `cargo test -p pgt-ddl`, clippy. **Adım 5: Commit** (`feat(ddl): nesne yetkileri fark üreticisi — GRANT/REVOKE, GRANT OPTION, CASCADE`).

---

### Task 3: Varsayılan yetki farkı üreticisi

**Dosyalar:** Oluştur `crates/pgt-ddl/src/build/default_acl.rs` (+ `snapshots/`); değiştir `build/mod.rs`.

**Arayüzler — Üretir:**

```rust
pub fn default_privileges(ctx: &BuildCtx, old: &DefaultAclDef, new: &DefaultAclDef) -> Result<Plan, Vec<FieldError>>;
```
- Kurallar: Karar 12 (+ Karar 6 sırası, Karar 8 sahip = `role`, Karar 10 alıcı kuralları). Kimlik (`role, schema, viewer`) değişimi → `object` hatası; `editable = false` → `object`; bölüm sırası/kümesi (`default_sections(schema.is_some())`) → `sections`; `baseline` değişimi → `sections[i].baseline`; tür kümesi dışı yetki → `sections[i].grants[j].privileges[k].privilege`.
- Tüm ifadeler Safe + plan notu (Karar 7).

- [ ] **Adım 1: Başarısız testler (insta + birim):** genel kapsam Tables GRANT SELECT TO r; Functions `REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC`; Schemas `GRANT USAGE ON SCHEMAS`; şema kapsamı `IN SCHEMA "s"` GRANT ve REVOKE; GRANT OPTION ekleme/kaldırma; çok bölümlü plan sırası; hatalar (şema kapsamında Schemas bölümü, eksik bölüm, `baseline` değişimi, PG 16 `Maintain`, kimlik değişimi); hostile rol/şema adı.
- [ ] **Adım 2–5:** uygula, `cargo test -p pgt-ddl`, clippy, commit (`feat(ddl): varsayılan yetkiler fark üreticisi — ALTER DEFAULT PRIVILEGES`).

---

### Task 4: Nesne ACL yükleyicisi ve Docker gidiş-dönüşü

**Dosyalar:** Oluştur `crates/pgt-ddl/src/load_acl.rs`, `src/sql/load_acl_relation.sql`, `load_acl_proc.sql`, `load_acl_namespace.sql`, `load_acl_database.sql`, `load_acl_type.sql`, `load_acl_columns.sql`; oluştur `crates/pgt-ddl/tests/acl_roundtrip.rs`; değiştir `tests/common/mod.rs` (test rolü oluşturma/temizleme yardımcıları, `set role` ile ikinci bağlantı yardımcısı gerekiyorsa).

**Arayüzler — Üretir:**

```rust
pub async fn load_acl(mgr: &ConnectionManager, server: Uuid, database_oid: impl Into<Option<u32>> + Send, obj: ObjectRef) -> Result<AclDef>;
```
- SQL deseni (her katalog için; `pgt-catalog/src/sql/acl_relation.sql` örneğindeki `aclexplode` kalıbı, ancak **grantor ayrımı ve etkin ACL** ile):
  ```sql
  SELECT n.nspname::text AS schema, c.relname::text AS name, pg_get_userbyid(c.relowner)::text AS owner,
         current_user::text AS viewer, c.relacl IS NOT NULL AS explicit, c.relkind::text AS relkind,
         CASE WHEN a.grantee = 0 THEN NULL ELSE pg_get_userbyid(a.grantee)::text END AS grantee,
         pg_get_userbyid(a.grantor)::text AS grantor, a.grantor = c.relowner AS by_owner,
         a.privilege_type::text AS privilege, a.is_grantable AS grantable
  FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
  LEFT JOIN LATERAL aclexplode(coalesce(c.relacl, acldefault(CASE WHEN c.relkind = 'S' THEN 's' ELSE 'r' END::"char", c.relowner))) a ON true
  WHERE c.oid = $1
  ```
  (adlar ham metin döner; tırnaklama üreticide). Tür/relkind uyuşmazlığı (ör. `ObjectRef{Table}` ama `relkind = 'S'`) → `NotFound`. Function/Procedure: `pg_proc` + `pg_get_function_identity_arguments`; Schema: `pg_namespace` (`acldefault('n', …)`); Database: `pg_database` (bakım bağlantısı; `acldefault('d', …)`); Type/Domain: `pg_type` (`acldefault('T', …)`; `typtype = 'd'` ⇔ Domain; dizi `typelem <> 0 AND typlen = -1` / `typcategory = 'A'`, multirange `typtype = 'm'`, tablo satır tipi → Karar 11). Kolonlar: `pg_attribute` `attacl IS NOT NULL AND attnum > 0 AND NOT attisdropped`, `aclexplode(attacl)`.
- Sistem şeması: `nspname IN ('pg_catalog','information_schema') OR nspname LIKE 'pg\_toast%'` → `editable = false`, `unsupported: ["sistem şeması"]`.
- `grants` kurulumu Karar 5 (`by_owner`); `sort_grants` ile kanonik. Bilinmeyen yetki → Karar 3.

- [ ] **Adım 1: Başarısız testler (Docker, her sunucuda):** her tür için: varsayılan ACL'nin etkin gösterimi (Table: sahip tüm yetkiler, `explicit = false`; Function: sahip + `PUBLIC EXECUTE`; Database: `PUBLIC CONNECT, TEMPORARY`; Type/Domain: `PUBLIC USAGE`; Schema: yalnız sahip), `privileges(loaded, düzenlenmiş)` uygula → yeniden yükle → beklenen `grants` ve `alter(loaded2, loaded2)` boş plan; GRANT OPTION ekleme/kaldırma; `PUBLIC EXECUTE` kaldırma sonrası `explicit = true`; ikinci rol `WITH GRANT OPTION` aldıktan sonra (`SET ROLE` ile) üçüncü role verdiği yetki `foreign`'da; bağımlı yetki varken RESTRICT REVOKE → 2BP01 ve ACL değişmez; CASCADE → bağımlı yetki de gider; kolon yetkisi (`GRANT SELECT (c) …`) `columns`'ta; aşırı yüklenmiş iki fonksiyondan yalnız biri değişir; hostile adlı rol ve şema; PG 17+ `MAINTAIN` gidiş-dönüşü (PG < 17 test atlanır ve rapora yazılır); procedure PG 11+; sistem şeması nesnesi (`pg_catalog.pg_class`) `editable = false`; dizi tipi salt okunur.
- [ ] **Adım 2–4:** uygula; `cargo test --workspace` (Docker'sız), clippy, matris (`scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` ya da yalnız bu test dosyasına süzülmüş kopya; kapsayıcılar silinir).
- [ ] **Adım 5: Commit** (`feat(ddl): nesne yetkileri yükleyicisi ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 5: Varsayılan yetki yükleyicisi ve Docker gidiş-dönüşü

**Dosyalar:** Değiştir `load_acl.rs`; oluştur `src/sql/load_default_acl.sql` (+ rol/şema çözümü); değiştir `tests/acl_roundtrip.rs`.

**Arayüzler — Üretir:**

```rust
pub async fn load_default_acl(mgr: &ConnectionManager, server: Uuid, database_oid: u32, scope: DefaultAclScope) -> Result<DefaultAclDef>;
```
- Rol: `scope.role` `None` → `current_user`; `pg_roles WHERE rolname = $1` (yoksa `NotFound("rol")`). Şema: `schema_oid` → `pg_namespace` (yoksa `NotFound("şema")`; sistem şeması → `editable = false`).
- Genel bölümler: her tür için `aclexplode(coalesce((SELECT defaclacl FROM pg_default_acl WHERE defaclrole = $rol AND defaclnamespace = 0 AND defaclobjtype = $harf), acldefault($harf, $rol)))`. Şema bölümleri: yalnız `defaclnamespace = $şema` girdileri (`grants`), `baseline` = genel etkin değer. `grantor` her zaman `role`'dür (PG); `by_owner` ayrımı gerekmez — farklıysa `unsupported`.
- `pg_default_acl` `defaclobjtype = 'n'` PG 10+ (taban); PG 18 `'L'` (large objects) satırı → yok sayılmaz: `unsupported: ["LARGE OBJECTS varsayılan yetkileri"]` ve `editable` **true** kalır (o bölüm modelde yok, üretici dokunmaz) — not raporda.

- [ ] **Adım 1: Başarısız testler (Docker, her sunucuda):** boş durumda genel bölümler `acldefault` değerleri (Functions: rol + `PUBLIC EXECUTE`; Types: rol + `PUBLIC USAGE`), şema kapsamı boş `grants` + doğru `baseline`; GRANT SELECT ON TABLES → yeni tablo oluştur → `has_table_privilege` true; `REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC` → yeni fonksiyonda PUBLIC yok; şema kapsamı GRANT → yeniden yükle → `grants` dolu, `alter(x, x)` boş; başka rol için `FOR ROLE` yetkisiz bağlantıda 42501 (ikinci bağlantı ya da `SET ROLE`; mümkün değilse raporla); hostile rol/şema adları.
- [ ] **Adım 2–5:** uygula, test, clippy, matris, commit (`feat(ddl): varsayılan yetkiler yükleyicisi ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 6: Yönetici entegrasyonu, kapılar ve Tauri komutları

**Dosyalar:** Değiştir `crates/pgt-ddl/src/manager.rs` (+ `tests/manager.rs`, `tests/logging.rs`), `src-tauri/src/ddl.rs`, `src-tauri/src/lib.rs` (komut kaydı).

**Arayüzler — Üretir:**

```rust
impl DdlManager {
    pub async fn load_acl(&self, server: Uuid, database_oid: impl Into<Option<u32>> + Send, obj: ObjectRef) -> Result<AclDef>;
    pub async fn load_default_acl(&self, server: Uuid, database_oid: u32, scope: DefaultAclScope) -> Result<DefaultAclDef>;
}
pub const ACL_WARNING: &str; pub const ACL_DEPENDENTS_HINT: &str; pub const ACL_PRIVILEGE_HINT: &str;
pub const DEFAULT_ACL_PRIVILEGE_HINT: &str; pub const ACL_ROLE_MISSING_HINT: &str;   // Karar 14–15 metinleri
```
```rust
// src-tauri/src/ddl.rs
#[tauri::command] async fn ddl_load_acl(server_id: Uuid, database_oid: Option<u32>, object: ObjectRef) -> Result<AclDef, ErrorPayload>;
#[tauri::command] async fn ddl_load_default_acl(server_id: Uuid, database_oid: u32, scope: DefaultAclScope) -> Result<DefaultAclDef, ErrorPayload>;
```
- `plan_view`: `Privileges` → `build::acl::privileges(&ctx, old, new, *cascade)`; `DefaultPrivileges` → `build::default_acl::default_privileges(&ctx, old, new)`.
- Nesne denetimi genelleştirmesi: `object_check` yerine iç `enum Expected<'a> { Def(ObjectRef, &'a ObjectDef), Acl(ObjectRef, &'a AclDef), DefaultAcl(DefaultAclScope, &'a DefaultAclDef) }`; `apply` (4)'te ilgili yükleyici çağrılır, eşitsizlik `PLAN_CHANGED`; `Privileges`'ta `object.kind != old.kind` → `INVALID_DEF`; `!acl_supported(object.kind)` → `Invalid` (Karar 1 iletisi). `DefaultPrivileges`'ta `scope.role` `Some` ise `old.role` ile eşit olmalı (değilse `INVALID_DEF`).
- Salt okunur kapısı: `old.editable == false` → `READ_ONLY_PREFIX` + gerekçe (`read_only_reason`'a `"sistem şeması"`, `"dizi tipi"`, `"MULTIRANGE"`, `"tablo satır tipi"`, `"yetki "` → "bilinmeyen yetki", `"imza"` → "imza çözülemedi" girdileri; önek tablosu dizi boyutu güncellenir).
- Bakım veritabanı reddi (4a) bu isteklerde uygulanmaz (yetki değişimi veritabanını kapatmaz; Karar 9 yalnız uyarı).
- Uygulama sonrası denetim (Karar 14): `Job`'a `verify_acl: Option<VerifyAcl>` (`Object(ObjectRef, Vec<AclGrant>)` / `Default(DefaultAclScope, Vec<Vec<AclGrant>>)`); `Applied` sonrası yeniden yükle ve karşılaştır → `ACL_WARNING`.
- İpuçları (Karar 15) `hint_for`'a, genel `("42501", _)` kolundan önce; `kind` + `op` ile.
- `find_created`/`identity_changed`: yeni varyantlarda `created = None`.
- Tauri komutları mevcut `ddl_load` desenini izler (aynı hata eşlemesi, aynı `DdlManager` durum nesnesi); günlükte yalnız `op`, `kind`, sonuç.

- [ ] **Adım 1: Başarısız testler.** Birim (Docker'sız): `plan_view` yeni varyantlar; `read_only_refusal` ACL için; `hint_for` dört yeni ipucu ve genel 42501'in `op = "alter"`'da değişmediği; `Expected` kimlik uyuşmazlıkları `INVALID_DEF`. Docker (`tests/manager.rs`): kilitli sunucuda iki istek `locked`; bayat `old` (arada başka bağlantı GRANT etmiş) → `plan_changed`; REVOKE `confirmation_required`; CASCADE üretimde `production_confirmation_required`; tam akış `preview → apply` hash ile; Database ACL bakım bağlantısında uygulanır; sahip olmayan bağlı rol (GRANT OPTION'lı) GRANT → `Applied` + `ACL_WARNING`; 2BP01 ipucu; `logging.rs`: ayırt edici rol adı ve yetki dizgeleri günlükte yok.
- [ ] **Adım 2–4:** uygula; `cargo test --workspace`, clippy, matris; `cargo build -p pg-tools` (src-tauri) derlenir.
- [ ] **Adım 5: Commit** (`feat(ddl): yetki istekleri için yönetici kapıları, uygulama sonrası denetim ve Tauri komutları`).

---

### Task 7: Belgeler, matris ve bütünleşik doğrulama

**Dosyalar:** Değiştir `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-17-privileges-design.md`, `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§1.3'ten GRANT/REVOKE maddesini çıkar, yeni spec'e işaret), `docs/testing.md` (yeni test dosyası ve test rolleri temizliği).

- [ ] **Adım 1:** Tam Docker matrisi `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` — yeşil; sürüm kapılı atlamalar (MAINTAIN < 17, procedure PG 10) rapora.
- [ ] **Adım 2: `docs/smoke-test.md`** — "Yetkiler arka ucu (Plan 20)" bölümü (GUI maddeleri Plan 21'de); bilinen sınırlar (kolon yetkileri salt okunur, başka verenlerin yetkileri salt okunur, `GRANTED BY` yok, tablespace/dil/FDW/large object/parametre yok, PG 18 LARGE OBJECTS varsayılan yetkileri salt okunur).
- [ ] **Adım 3: Spec** — durum "Plan 20 arka ucu uygulandı; Plan 21 ön yüzü kalan"; **Plan 20 sözleşme notları (Plan 21 için)**: model alanları, `viewer`/`explicit` anlamı, alan yolları (`grants[i].grantee`, `grants[i].privileges[j].privilege|grantOption`, `foreign`, `columns`, `sections[i].grants[j]…`, `sections[i].baseline`, `object`), uyarı metinleri, ipucu sabitleri, `ACL_WARNING`, iki Tauri komutunun imzaları, `DefaultAclScope.role = null` davranışı, ön yüz bütçe notu (ana 231,724 B; `commands.ts` eklemesi + Explorer menü kancası telafi gerektirir).
- [ ] **Adım 4: Bütünleşik doğrulama.** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `bun run test`, `bun run typecheck`, `bun run build` (ana **231,724 B**, mapDeps 21, preload solid/web/Icon).
- [ ] **Adım 5: Kapanış greps:** `rg "tracing::" crates/pgt-ddl/src/build/acl.rs crates/pgt-ddl/src/build/default_acl.rs crates/pgt-ddl/src/load_acl.rs` boş; `rg "ALL PRIVILEGES" crates/pgt-ddl/src` yalnız yorumlarda; `rg "format!\(" crates/pgt-ddl/src/build/acl.rs` her yerleştirme `quote_ident`/`qualified`/sabit yetki metniyle.
- [ ] **Adım 6: Commit** (`docs(ddl): Plan 20 sözleşme notları, smoke ve bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 20 bitişi)

- Nesne yetkileri (10 tür) ve varsayılan yetkiler (genel + şema) için load/plan/apply PG 10–18'de yeşil; sabit nokta her türde; NULL ACL, veren ayrımı, RESTRICT/CASCADE testli.
- Salt okunur nesnelerde değişiklik yok; günlükte rol adı/yetki/SQL yok; uygulama sonrası fark uyarısı çalışır.
- Ana paket **231,724 B** (değişmedi); yeni TS yalnızca bindingler (+ gerekirse tembel uyum).
- Elle kontrol maddeleri Plan 21'dedir; final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §1 nesne türleri → K1/T1; etkin ACL → K4/T4; düzenlenebilir satırlar/veren → K5/T2/T4; sahip satırı → K8/T2; ifadeler/sıra → K6/T2; CASCADE → K7/T2/T6; risk → K7; adresleme → K6/Global Constraints; yetki kümeleri → K2/T1; varsayılan yetkiler → K12/T3/T5; kendini kilitleme → K9/T2; kapsam (Database bakım) → K13/T6; §2 model → T1; uygulama sonrası uyarı → K14/T6; §4 güvenlik → Global Constraints + T6 günlük; §5 test → T2–T6; §3 arayüz → Plan 21.
- **Bilinçli kararlar:** imza metni `pg_get_function_identity_arguments` (spec güncellendi); `DefaultPrivileges` hedef türü `Schema` vekili (yeni `ObjectKind` ön yüzdeki tam `Record`'ları bozmasın diye); bakım veritabanı reddi yetkilerde uygulanmaz (yalnız uyarı); eklentiye ait nesnelerde düzenlemeye izin.
- **Yer tutucu taraması:** Task 1'deki geçici `Invalid` kolları Task 2–3 (üretici) ve Task 6'da (kapılar) kalkar (açıkça belirtilmiş ara durum).
- **Tip tutarlılığı:** `AclDef`/`DefaultAclDef`/`DefaultAclScope` (T1) → üreticiler (T2/T3) → yükleyiciler (T4/T5) → yönetici + Tauri (T6); `privileges_for`/`default_privileges_for`/`default_sections`/`sort_grants` (T1) her iki uçta aynı; `ACL_*` sabitleri (T6) → spec notları (T7).
