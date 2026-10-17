# pg-tools — Alt Proje 7: Yetkiler (GRANT/REVOKE) ve Varsayılan Yetkiler Tasarımı

- **Tarih:** 2026-10-17
- **Durum:** Plan 20 arka ucu uygulandı (PG 10–18 matrisi yeşil); Plan 21 ön yüzü kalan
- **Kapsam:** Explorer'dan bir nesnenin **yetkilerini** görüntüleme ve düzenleme (GRANT/REVOKE; `WITH GRANT OPTION`) ve **varsayılan yetkileri** (`ALTER DEFAULT PRIVILEGES`) düzenleme; önizlemeli, korumalı uygulama.
- **Temel:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (mimari §2, risk modeli §3, arayüz §4, güvenlik §6) — `pgt-ddl` çerçevesi aynen kullanılır; bu belge yalnız yetki modelini, kurallarını ve kararlarını ekler. O spec'in §1.3 "GRANT/REVOKE matrisi, varsayılan yetkiler" kapsam dışı maddesi buraya taşınır.
- **Teslim planları:** Plan 20 (arka uç: model, yetki kataloğu, saf üreticiler, yükleyiciler, yönetici entegrasyonu, PG 10–18 Docker matrisi) → Plan 21 (ön yüz: "Yetkiler…" ve "Varsayılan yetkiler…" sekmeleri, yetki ızgarası).

---

## 1. Kararlar

| Konu | Karar |
|---|---|
| Nesne türleri | Table (sıradan ve bölümlenmiş), View, Materialized View, Sequence, Function, Procedure (PG 11+), Schema, Database, Type, Domain. Kolon düzeyi yetkiler **salt okunur** listelenir. Tablespace, dil, FDW/sunucu, large object, parametre (PG 15+ `GRANT SET`) yetkileri ve `GRANTED BY` kapsam dışı (backlog). |
| Etkin ACL | Yükleme `coalesce(acl, acldefault(tür, sahip))` → `aclexplode`: NULL ACL (varsayılan) de açıkça gösterilir (ör. fonksiyonlarda `PUBLIC EXECUTE`, veritabanında `PUBLIC CONNECT, TEMPORARY`, tiplerde `PUBLIC USAGE`). İlk değişiklik ACL'yi açık hâle getirir (PG davranışı). |
| Düzenlenebilir satırlar | Yalnızca **verenin (grantor) nesne sahibi** olduğu girdiler düzenlenir (süper kullanıcının verdiği yetkiler PG'de sahip adına kaydedilir). Başka bir rolün `WITH GRANT OPTION` ile verdiği yetkiler **salt okunur** (veren adıyla) listelenir; onları geri almak o rolün işidir. |
| Sahip satırı | Sahibin kendi yetkileri gösterilir ve düzenlenebilir (PG izin verir); sahibin kendi yetkisini kaldırması **uyarı** üretir (sahip istediği an geri verebilir). |
| Üretilen ifadeler | Alıcı (grantee) başına fark: eklenen yetkiler `GRANT p, … ON <nesne> TO <alıcı>`; `WITH GRANT OPTION` eklenenler ayrı `GRANT … WITH GRANT OPTION`; kaldırılanlar `REVOKE p, … ON <nesne> FROM <alıcı>`; yetki kalıp yalnız seçenek kaldırılırsa `REVOKE GRANT OPTION FOR p ON … FROM …`. `ALL PRIVILEGES` yazılmaz (açık liste). Sıra: REVOKE'lar, REVOKE GRANT OPTION FOR'lar, GRANT'lar; alıcılar `PUBLIC` önce, sonra ad sırası. |
| CASCADE | `REVOKE` varsayılan olarak `RESTRICT`'tir: alıcının başkalarına verdiği bağımlı yetkiler varsa PG `2BP01` ile reddeder (ipucu). Kullanıcı açıkça **"bağımlı yetkilerle birlikte geri al (CASCADE)"** seçebilir. |
| Risk | `GRANT` Safe; `REVOKE` (ve `GRANT OPTION FOR`) **Destructive** (yetki kaybı; Plan 14 üyelik REVOKE kuralı); `REVOKE … CASCADE` **DataLoss** (başka rollerin verdiği yetkiler de silinir; üretimde ad yazma). Varsayılan yetkiler: `GRANT` ve `REVOKE` **Safe** + not "yalnızca bundan sonra oluşturulan nesneleri etkiler". |
| Nesne adresleme | Tablo/görünüm/MV `ON TABLE <nitelikli ad>`; dizi `ON SEQUENCE`; fonksiyon/procedure `ON FUNCTION|PROCEDURE <qualified(şema, ad)>(<pg_get_function_identity_arguments>)` (imza metni sunucudan; yeniden yükleme eşitliği sunucu metnini doğrular, nihai ifade `composed_is_clean` ile taranır); şema `ON SCHEMA`; veritabanı `ON DATABASE`; type/domain `ON TYPE` / `ON DOMAIN`. |
| Yetki kümeleri | Tablo/görünüm/MV: `SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER` (+ `MAINTAIN` PG 17+); dizi: `USAGE, SELECT, UPDATE`; fonksiyon/procedure: `EXECUTE`; şema: `USAGE, CREATE`; veritabanı: `CONNECT, CREATE, TEMPORARY`; type/domain: `USAGE`. Başka yetki doğrulama hatası. |
| Varsayılan yetkiler | Kapsam: bir rol (`FOR ROLE`) için **genel** (veritabanı düzeyi) ya da **şema** (`IN SCHEMA`). Nesne türleri: Tables, Sequences, Functions (procedure'leri de kapsar), Types, Schemas (PG 10+, yalnız genel). Genel kapsamda etkin değer `coalesce(defaclacl, acldefault(...))`; şema kapsamında yalnız **eklenen** yetkiler düzenlenir (PG şema düzeyinde genelden geri alma yapamaz) ve genel taban salt okunur gösterilir. |
| Kendini kilitleme uyarıları | Bağlı rolden yetki geri alma, bakım veritabanında `PUBLIC`/bağlı rolden `CONNECT` geri alma, sahibin kendi yetkisini kaldırması, şemadan `USAGE` geri alma (bağlı rol): **uyarı** (engellenmez). |
| Kapsam | Database ACL'i sunucu kapsamlıdır (bakım veritabanında uygulanır; Plan 14 kuralı); diğerleri veritabanı kapsamlı. |

## 2. Model (özet)

- `AclDef { kind, schema, name, signature, owner, viewer, explicit, grants: [AclGrant{ grantee: string | null (PUBLIC), privileges: [{ privilege, grantOption }] }], foreign: [ForeignGrant{ grantor, grantee, privilege, grantOption }] (salt okunur), columns: [ColumnGrant] (salt okunur), editable, unsupported }` — `viewer`: bağlı rol (`current_user`; kendini kilitleme uyarıları için), `explicit`: ACL NULL değil.
- `DefaultAclDef { role, schema: string | null, viewer, sections: [{ objectType, grants: [AclGrant], baseline: [AclGrant] (yalnız şema kapsamında; salt okunur genel taban) }], editable, unsupported }`; kapsam `DefaultAclScope { role: string | null (null → bağlı rol), schemaOid: number | null }`.
- Plan istekleri `PlanRequest::Privileges { object, old, new, cascade }` ve `PlanRequest::DefaultPrivileges { scope, old, new }`; önizleme/uygulama mevcut `ddl_preview`/`ddl_apply` kapılarından geçer (kilit, yeniden yükleme eşitliği, plan özeti, onay, üretimde ad yazma). Uygulama sonrası ACL yeniden yüklenir; istenenden farklıysa (ör. bağlı rol sahip değil ve yetki kendi adıyla "veren" olarak kaydedildi) uyarı döner (Plan 14 üyelik uyarısı kalıbı).

## 3. Arayüz (Plan 21 özeti)

- Explorer: desteklenen nesne düğümlerinde **"Yetkiler…"**; veritabanı ve şema düğümünde **"Varsayılan yetkiler…"**.
- Yetki ızgarası: satır = alıcı (PUBLIC, roller; "Alıcı ekle" — `grantable_roles` lookup), sütun = türün yetkileri; hücre üç durumlu (yok / var / var + GRANT OPTION); sahip satırı işaretli; başka verenlerin yetkileri ve kolon yetkileri salt okunur bölümde; CASCADE seçeneği; önizleme + kapılı uygulama (mevcut designer çerçevesi).
- Varsayılan yetkiler sekmesi: rol seçici (varsayılan: bağlı rol), nesne türü başına ızgara; şema kapsamında genel taban salt okunur.

## 4. Güvenlik

- Tanımlayıcılar yalnız `quote_ident`/`qualified`; fonksiyon imzası sunucunun `regprocedure` metni (yeniden yükleme eşitliği + nihai ifade taraması); yetki adları sabit kümeden.
- Yetki listeleri, rol adları ve SQL `tracing`'e yazılmaz.

## 5. Test stratejisi

- Saf üreticiler: insta (her tür; ekleme/kaldırma/seçenek yükseltme–düşürme/CASCADE; PUBLIC; sahip satırı; yabancı veren satırı değişmez; varsayılan yetkiler genel/şema), sürüm kapıları (MAINTAIN PG 17+, procedure PG 11+, Schemas default ACL PG 10+).
- Docker PG 10–18: `apply → load → beklenen`, `alter(loaded, loaded)` boş plan, NULL ACL'nin etkin gösterimi ve ilk değişiklik, yabancı veren (`WITH GRANT OPTION` alan rolün verdiği yetki), `RESTRICT` 2BP01 ve `CASCADE`, varsayılan yetkilerin yeni nesneye uygulanması, `FOR ROLE` yetkisizlik 42501.

## 6. Planlama notu

- **Plan 20 (arka uç):** model + istekler → yetki kataloğu + nesne ACL üreticisi → varsayılan yetki üreticisi → nesne ACL yükleyicisi → varsayılan yetki yükleyicisi → yönetici/komut entegrasyonu → belgeler ve matris.
- **Plan 21 (ön yüz):** menü kancası + telafi → yetki ızgarası modeli → "Yetkiler…" sekmesi → "Varsayılan yetkiler…" sekmesi → smoke listesi.

## 7. Plan 20 sözleşme notları (Plan 21 için)

Kaynak: `crates/pgt-ddl/src/{acl.rs, build/acl.rs, build/default_acl.rs, load_acl.rs, manager.rs}` ve `src-tauri/src/ddl.rs`. Tüm tipler `#[ts(export)]`, camelCase; bindingler `src/ipc/bindings` altındadır.

### 7.1 Model ve alan yolları

- `AclDef`: `kind, schema, name, signature, owner, viewer, explicit, grants, foreign, columns, editable, unsupported`. `viewer` = bağlı rol (`current_user`; kendini kilitleme uyarıları için). `explicit` = ACL sütunu NULL değil; **bilgi alanıdır**, üretici karşılaştırmaz (NULL ACL etkin değerle gösterilir, ilk değişiklik ACL'yi açık yapar).
- `AclGrant { grantee: string | null (PUBLIC), privileges: PrivilegeGrant[] }`, `PrivilegeGrant { privilege, grantOption }`. `grants` alıcıya göre sıralıdır (PUBLIC önce, sonra `COLLATE "C"` ad), yetkiler kanonik sırada (`privileges_for`). Sahip de `grants`'ta alıcıdır.
- `ForeignGrant { grantor, grantee, privilege, grantOption }` ve `ColumnGrant { column, grantor, grantee, privilege, grantOption }` **salt okunurdur**; değişirse alan hatası `foreign` / `columns`.
- `DefaultAclDef { role, schema, viewer, sections, editable, unsupported }`; `DefaultAclSection { objectType, grants, baseline }` (`baseline` yalnız şema kapsamında dolu ve salt okunur; değişirse `sections[i].baseline`). Bölümler her zaman sabit sırada (Tables, Sequences, Functions, Types[, Schemas]); `Schemas` yalnız genel kapsamdadır.
- `DefaultAclScope { role: string | null, schemaOid: number | null }`; **`role = null` → bağlı rol**. Yanıttaki `DefaultAclDef.role` çözülmüş adı taşır.
- Alan hatası yolları ve iletileri (tam liste; üreticilerin ürettiği her yol). Ad kuralı iletileri (`build::check_ident` ailesi): "Ad boş olamaz.", "Ad NUL karakteri içeremez.", "Ad denetim karakteri (satır sonu, sekme vb.) içeremez.", "Ad görünmez biçim ya da yön denetimi karakteri içeremez (U+XXXX).", "Ad en çok 63 bayt olabilir (şu an N bayt).", "Sistem şemaları (pg_*, information_schema) kullanılamaz.".
  - **Nesne yetkileri (`build::acl::privileges`):**
    - `object`: `UNSUPPORTED_KIND_MSG` "Bu nesne türünün yetkileri yoktur."; `IDENTITY_MSG` "Nesne kimliği (tür, şema, ad, imza, sahip, bağlı rol) yetki düzenlemede değiştirilemez." (`explicit` de kimliktir); `READ_ONLY_MSG` "Bu nesnenin yetkileri salt okunurdur; değiştirilemez." (`old.editable = false` iken herhangi bir değişiklik); `COMPOSED_BROKEN` "İfade güvenli biçimde oluşturulamadı." (Function/Procedure dışında nihai tarama).
    - `schema` (hedef; `object_target`): `SCHEMA_MISSING_MSG` "Bu nesne türü için şema gerekli.", `SCHEMA_EXTRA_MSG` "Bu nesne türünde şema olmaz." (Schema/Database), yüklü şema kuralları (boş/NUL, sistem şeması — `pg_*` dahil `pg_temp_N`).
    - `name`: yüklü ad kuralları (boş/NUL; Schema türünde sistem şeması yasağı da).
    - `signature`: `SIGNATURE_MISSING_MSG` "Fonksiyon/procedure için imza gerekli.", `SIGNATURE_EXTRA_MSG` "Bu nesne türünde imza olmaz.", `SIGNATURE_MSG` "İmza metni güvenli biçimde yazılamıyor.", `COMPOSED_BROKEN` (Function/Procedure'de nihai tarama).
    - `foreign`: "Başka verenlerin verdiği yetkiler buradan değiştirilemez."; `columns`: "Kolon yetkileri buradan değiştirilemez.".
    - `old.grants[i].grantee`: yüklü alıcı adı kuralları (boş/NUL/denetim karakteri; `check_old_ref`).
    - `grants[i].grantee`: `PUBLIC_MSG` "PUBLIC için alıcıyı boş bırakın." (yalnız **yeni** alıcı ve tam küçük harf `public`; bkz. E2), `DUP_GRANTEE_MSG` "Bu alıcı zaten listede.", yeni alıcıda `check_ident` kuralları, eskiden gelen alıcıda `check_old_ref` kuralları.
    - `grants[i].privileges[j].privilege`: `INVALID_PRIVILEGE_MSG` "Bu nesne türünde geçersiz yetki.", `MAINTAIN_MSG` "MAINTAIN yetkisi PostgreSQL 17 ve sonrasında kullanılabilir." (tablo türlerinde PG < 17).
    - `grants[i].privileges[j]`: `DUP_PRIVILEGE_MSG` "Bu yetki bu alıcı için zaten listede.".
    - `grants[i].privileges[j].grantOption`: `OWNER_GRANT_OPTION_MSG` "Sahip, yetkileri zaten devredebilir; GRANT OPTION ayarlanamaz.", `PUBLIC_GRANT_OPTION_MSG` "PUBLIC'e GRANT OPTION verilemez.".
  - **Varsayılan yetkiler (`build::default_acl::default_privileges`):**
    - `object`: `IDENTITY_MSG` "Varsayılan yetki kimliği (rol, şema, bağlı rol) değiştirilemez.", `READ_ONLY_MSG` "Bu varsayılan yetkiler salt okunurdur; değiştirilemez.", `COMPOSED_BROKEN` "İfade güvenli biçimde oluşturulamadı.", yüklü `role` (boş/NUL) ve `schema` (boş/NUL, sistem şeması) kuralları.
    - `sections`: `SECTIONS_MSG` "Bölümler sabit sırada ve uygun türlerin tamamıyla olmalıdır."; `sections[i].baseline`: `BASELINE_MSG` "Genel kapsamdan gelen değer buradan değiştirilemez.".
    - `old.sections[i].grants[j].grantee`, `sections[i].grants[j].grantee`, `sections[i].grants[j].privileges[k]`, `sections[i].grants[j].privileges[k].privilege`, `sections[i].grants[j].privileges[k].grantOption`: nesne yetkileriyle aynı kurallar ve iletiler (ortak `validate_rows`; sahip = `role`).

### 7.2 Uyarı, not ve ipucu sabitleri

- Plan uyarıları (`build::acl`; ad/değer içermez): `OWNER_REVOKE_WARNING` ("Sahip kendi yetkisini kaldırıyor; sahip bu yetkiyi istediği an geri verebilir."), `VIEWER_REVOKE_WARNING` ("Bağlı rol bu nesne üzerindeki yetkisini kaybedebilir."), `CONNECT_WARNING` ("Bu veritabanına bağlanma yetkisi kaldırılıyor; bağlı rol (ve PUBLIC üzerinden bağlananlar) yeniden bağlanamayabilir."), `USAGE_WARNING` ("Şemadaki nesnelere erişim kesilebilir.").
- Plan notları: `CASCADE_NOTE` ("Alıcının bu yetkiyle başkalarına verdiği yetkiler de geri alınır."; `cascade = true` ve en az bir REVOKE varken, REVOKE'lar DataLoss; REVOKE yokken `cascade` yok sayılır), `build::default_acl::NOTE` ("Yalnızca bundan sonra oluşturulan nesneleri etkiler; mevcut nesnelerin yetkileri değişmez."; varsayılan yetkilerde tüm ifadeler Safe).
- Yönetici sabitleri (`pgt_ddl::manager`): `ACL_WARNING` (uygulama sonrası `grants` / `sections[*].grants` istenenden farklı → `ApplyOutcome::Applied.warnings`; yeniden yükleme hatası uyarı üretmez), `ACL_DEPENDENTS_HINT` (2BP01 + `privileges`), `ACL_PRIVILEGE_HINT` (42501 + `privileges`), `DEFAULT_ACL_PRIVILEGE_HINT` (42501 + `default_privileges`), `ACL_ROLE_MISSING_HINT` (42704 + iki işlem de; yalnız `GRANT `/`REVOKE `/`ALTER DEFAULT PRIVILEGES ` ile başlayan ifadede ve Type/Domain dışında — bkz. E3), `READ_ONLY_PREFIX` ("Bu nesne buradan düzenlenemez: ") ile salt okunur kapısı.

### 7.3 Tauri komutları (`src-tauri/src/ddl.rs`; `src-tauri/src/main.rs`'te kayıtlı)

- `ddl_load_acl(server_id: Uuid, database_oid: Option<u32>, object: ObjectRef) -> AclDef` (Database için `database_oid` verilmez; bakım bağlantısı).
- `ddl_load_default_acl(server_id: Uuid, database_oid: u32, scope: DefaultAclScope) -> DefaultAclDef`.
- Önizleme/uygulama mevcut `ddl_preview`/`ddl_apply`'dır; istekler `PlanRequest::Privileges { object, old, new, cascade }` (`op = "privileges"`) ve `PlanRequest::DefaultPrivileges { scope, old, new }` (`op = "default_privileges"`). `old`, yeniden yüklenen tanıma birebir eşit olmalıdır (`viewer` dahil), değilse `PLAN_CHANGED`.
- İki komut `src/ipc/commands.ts`'e **henüz eklenmedi** (Plan 21).

### 7.4 Uygulama kararları ve errata

- **E1 (2BP01 ipucu):** Plan 20 Karar 15 "yalnız PG ipucu yoksa" demişti; uygulamada `ACL_DEPENDENTS_HINT` `override_hint_for` ile uygulanır ve PG'nin kendi "Use CASCADE…" ipucunun **yerine geçer** (arayüzde CASCADE bir ifade değil seçenektir). Diğer ACL ipuçları PG ipucu yokken uygulanır.
- **E2 (Karar 10, PUBLIC adı):** Plan 20 Karar 10 `Some("public")`'i büyük/küçük harf duyarsız reddediyordu. PG yalnız tam küçük harf `public`'i ayırır (`CREATE ROLE public` reddedilir; `quote_ident("public")` tırnaksız yazılır ve `PUBLIC` anlamına gelir); `CREATE ROLE "PUBLIC"` / `"Public"` geçerlidir ve `quote_ident` bunları tırnaklayıp gerçek role yazar. Uygulama: `old`'da zaten bulunan alıcı yalnız `check_old_ref` ile gevşek doğrulanır (PUBLIC adı denetimi yok; yüklü `"PUBLIC"`/`"Public"` rolünde `privileges(x, x)` boş plan, REVOKE çalışır); **yeni** alıcıda yalnız tam `public` reddedilir (ileti aynı: "PUBLIC için alıcıyı boş bırakın."), `"PUBLIC"`/`"Public"` kabul edilir. Varsayılan yetkilerde aynı (ortak `validate_rows`).
- **E3 (Karar 15, 42704 / 42809):** `ACL_ROLE_MISSING_HINT` yalnız başarısız ifade `GRANT `/`REVOKE `/`ALTER DEFAULT PRIVILEGES ` ile başlıyorsa ve tür Type/Domain değilse verilir (Type/Domain'de 42704 nesnenin yeniden yüklemeden sonra kaybolması da olabilir; orada yalnız PG iletisi). Plan 18 `ROUTINE_KIND_HINT` (42809, Function ↔ Procedure) yetki işlemlerinde (`privileges`, `default_privileges`) verilmez.
- **E4 (yükleyici ↔ üretici uyumu):** yükleyici, başka gerekçe yokken üreticinin yüklü taraf doğrulayıcıları (`build::acl::loaded_errors`, `build::default_acl::loaded_errors`: hedef/kimlik + eski alıcı adları) tanımı reddedecekse `editable = false` ve `unsupported: "üreticinin kabul etmediği ad ya da şema"` (`load_acl::BUILDER_REJECTS_REASON`) döner (ör. denetim karakterli rol adı, `pg_temp_N` geçici şema). Yönetici salt okunur gerekçesi aynı metindir.
- **R4a (aggregate):** tüm aggregate'ler ACL düzenlemede salt okunurdur (`unsupported: "AGGREGATE (aggregate yetkileri buradan düzenlenmez)"`, `editable = false`); window fonksiyonları düzenlenebilir.
- **Procedure imzası:** `pg_get_function_identity_arguments` metni PG 11–13 ile 14+ arasında farklıdır (PG 14+ `IN a integer`); ikisi de geçerlidir, imza her zaman sunucudan alınan metindir.
- **`pg_default_acl` harfi:** dizi için `defaclobjtype = 'S'` ↔ `acldefault('s', …)` eşlemesi yükleyicide yapılır.
- **PG 18 LARGE OBJECTS:** `defaclobjtype = 'L'` girdileri modellenmez; `unsupported`'a "LARGE OBJECTS varsayılan yetkileri" notu düşer, `editable` **true** kalır (üretici bu girdilere dokunmaz). Veren rolden farklı varsayılan girdi ("veren rolden farklı") tanımı salt okunur yapar.
- Yükleyici salt okunur gerekçeleri: sistem şeması, imza (NUL/denetim karakteri), dizi tipi, multirange, tablo satır tipi, AGGREGATE, bilinmeyen yetki harfi (`"yetki <metin>"`), üreticinin kabul etmediği ad ya da şema (E4). Eklentiye ait nesneler düzenlenebilir.

### 7.5 Bilinen sınırlar

Kolon yetkileri salt okunur; başka verenlerin yetkileri salt okunur; `GRANTED BY` yok; tablespace, dil, FDW/sunucu, large object ve parametre yetkileri yok; PG 18 LARGE OBJECTS varsayılan yetkileri modellenmez; yabancı tablo backlog.

### 7.6 Ön yüz bütçe notu (Plan 21)

Ana paket `index-*.js` **231,724 B**'tır (Plan 20'de değişmedi; yeni TS yalnızca bindingler). Plan 21'de `src/ipc/commands.ts` eklemesi ve Explorer menü kancası ana pakete bayt ekler; **telafi gerektirir** (ör. başka bir kodun tembel dosyaya taşınması). `__vite__mapDeps` 21 girdi ve `index.html` modulepreload yalnız solid/web/Icon kalmalıdır.

### 7.7 Doğrulama (Plan 20)

Tam Docker matrisi `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` fd5163e üzerinde yeşildir (çıkış 0; 47 test-result satırı ok, 0 failed). Sürüm kapılı atlamalar: MAINTAIN PG < 17, procedure PG 10. Son inceleme düzeltmelerinden (E2–E4, başka role ait nesne gidiş-dönüşü) sonra matris yeniden yeşil (çıkış 0; 47 test-result satırı ok, 0 failed).
