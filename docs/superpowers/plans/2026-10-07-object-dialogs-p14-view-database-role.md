# Nesne Diyalogları — View / Materialized View / Database / Role Arka Ucu (Plan 14) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** `pgt-ddl`'e spec'te kalan dört nesne türünün **arka ucunu** eklemek: **View**, **Materialized View**, **Database**, **Role** — yapılandırılmış tanımlar, saf üreticiler (create / alter farkı / drop), katalog yükleyicileri, yeni açılır listeler, yönetici/komut entegrasyonu (sunucu kapsamlı türler için bakım veritabanı hedefi, parola gizleme, veritabanı bağlantısı serbest bırakma) ve PG 10–18 Docker matrisi. **Ön yüz bu planda değil:** arayüz `docs/superpowers/plans/2026-10-07-object-dialogs-p15-view-database-role-on-yuz.md` (**Plan 15**) içindedir; bu plan ön yüze yalnızca **tembel derleme uyumu** için birkaç satır dokunur (Task 1) ve Plan 15'in tükettiği sözleşmeyi (bindingler + "Plan 15 için notlar") üretir. **Bağlantı:** Plan 15, bu planın `main`'e birleşmesinden sonra başlar; Plan 15'in Global Constraints'i bu planın "Sözleşme" bölümünü bağlayıcı sayar.

**Mimari:** Plan 10–12 kalıbı aynen: istemci SQL göndermez, `ObjectDef` gönderir; arka uç `build::{create, alter, drop}` ile planı üretir; `DdlManager` aynı kapılarla uygular (yeniden yükleme eşitliği, plan özeti, onay, üretim onayı, kilit). Yenilikler: (1) **kapsam** — Database ve Role **sunucu kapsamlıdır** (`ObjectKind::server_scoped`): yükleme, açılır listeler ve uygulama `Target::Maintenance` üzerinden yürür, `database_oid` yok sayılır (`Option<u32>`); (2) **parola** — Role parolası tanımda yalnızca **niyet** (`keep | set | clear`) olarak bulunur, değer önizlemeye/plana/özete hiç girmez; yalnızca `ApplyOptions.password` (gizli tip `Secret`) ile uygulamada gelir ve tek bir **gizlenmiş** ifade (`Statement.redacted`) uygulama anında gerçek değerle somutlaştırılır; (3) **transaction dışı** — Database ifadelerinin tamamı tek tek ve transaction dışında çalışır (spec §5), kısmi uygulama mevcut `committed` sayacıyla bildirilir; (4) **görünüm yeniden kurma** — `CREATE OR REPLACE VIEW` varsayılandır; kolon uyumsuzluğu sunucuda anlaşılır ve kullanıcı açıkça `recreate` seçer (aşağıda "Karar 1").

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta, mevcut Docker matrisi (`scripts/test-matrix.ps1`).

**Spec:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§2 mimari ve sözleşme notları, §3 risk modeli, §5 View/MatView/Database/Role, §6 güvenlik/gizlilik, §7 test). Önceki planlar: `2026-10-06-object-dialogs-p10-arka-uc.md` ve `2026-10-06-object-dialogs-p12-tablo-arka-uc.md` — burada tekrarlanmayan **tüm Global Constraints ve Review Focus** maddeleri bağlayıcıdır (tırnaklama, NUL/denetim karakteri reddi, serbest SQL kapıları, kilit/onay/üretim kapıları, günlük gizliliği, Docker kuralları, "alter yalnızca değişen ya da ifadeye yazılan alanları doğrular").

## Kararlar (spec ile çatışmalar işaretli)

1. **View kolon uyumsuzluğu — açık `recreate` (SPEC ÇATIŞMASI §5):** spec "kolon uyumsuzluğunda drop+create'e yükseltilir" der. Uyumluluk yeni sorgunun **çıktı kolonlarına** bağlıdır; bunlar yalnızca sunucuda ayrıştırmayla öğrenilir (katalog oturumu `search_path = pg_catalog` olduğundan kullanıcı sorgusunu doğru çözümleyemez; önizleme başına ayrı bağlantı açmak 150 ms hedefini ve kilit riskini bozar). Bu yüzden üretici saf kalır: sorgu değişince **her zaman** `CREATE OR REPLACE VIEW` üretir (Safe + not "mevcut kolonlar aynı ad/tip/sırada kalmalı; yeni kolonlar yalnız sona eklenebilir"). PG reddederse (42P16 / 42804, "cannot drop columns from view", "cannot change name/data type of view column") transaction geri alınır ve hata yüküne **ipucu** eklenir: "Kolonlar uyumsuz: 'Yeniden oluştur (DROP + CREATE)' seçeneğini işaretleyin." Kullanıcı alter-only `ViewDef.recreate = true` seçer → `DROP VIEW` (CASCADE'siz) + `CREATE VIEW` + sahip + yorum. Otomatik yükseltme yapılmaz (önizlenmemiş plan çalışmaz ilkesi).
2. **Yeniden kurma riski = DataLoss:** düzenleme içindeki DROP+CREATE (view `recreate`, MatView tanım değişikliği) **DataLoss**'tur: GRANT yetkileri, INSTEAD OF tetikleyicileri/kurallar, kolon varsayılanları/yorumları, MV verisi ve **MV indeksleri** kaybolur; bu kayıp "düzenleme" niyetinde beklenmez → üretimde ad yazma. Silme diyaloğundaki düz `DROP VIEW`/`DROP MATERIALIZED VIEW` spec §3 gibi **Destructive**, `CASCADE` DataLoss. Bağımlı nesne varsa DROP CASCADE'siz olduğu için PG 2BP01 verir (detay = bağımlılar), plan geri alınır — bağımlılar sessizce silinmez.
3. **`DROP DATABASE` = DataLoss (SPEC ÇATIŞMASI §3 tablo):** §3 tablosu "database"ı Destructive DROP'lar arasında sayar; ancak veritabanı silmek tüm veriyi siler. Plan 12'de `DROP TABLE` için verilen karar (CASCADE'siz de DataLoss) ile tutarlı olarak `DROP DATABASE [WITH (FORCE)]` **DataLoss**'tur (üretimde ad yazma). Spec §3 satırı Task 9'da düzeltilir.
4. **PG 16+ üyelik seçenekleri (`INHERIT`/`SET` per grant):** spec §1.3 bunları kapsam dışı sayar (görev metni "gating" istemişti → spec'e uyuldu). Yükleyici PG 16+'da `inheritOption`/`setOption` değerlerini **salt okunur** döndürür (gösterim için); üretici bunları hiç yazmaz; yeni üyelik sunucu varsayılanlarıyla verilir; mevcut üyelikte ADMIN değişimi diğer seçeneklere dokunmaz (`GRANT … WITH ADMIN OPTION` / `REVOKE ADMIN OPTION FOR` PG 16'da yalnız admin bayrağını değiştirir). Aynı (rol, üye) için birden çok grantor satırı (PG 16+) → üyelik `unsupported` listesine gider ve düzenlenmez.
5. **Rolün sahibi yoktur:** spec §5 "her türde Sahip" der; PostgreSQL rollerinde sahip kavramı yok → `RoleDef`'te `owner` alanı yok. Yalnızca "üye olduğu roller" (`memberOf`) düzenlenir; ters yön ("bu rolün üyeleri") kapsam dışıdır (üyenin kendi diyaloğundan yapılır).
6. **Parola gizleme tasarımı:** `RoleDef.password: PasswordChange` (`keep | set | clear`) yalnızca niyettir; **değer hiçbir zaman `ObjectDef`'te, `PlanRequest`'te, önizlemede, `plan_hash` girdisinde ya da günlükte bulunmaz.** Üretici `set` için tek ifade üretir: `ALTER ROLE <q> PASSWORD ********` (`Statement.redacted = true`; `********` bilerek **tırnaksızdır**: metin Query Tool'a kopyalanıp çalıştırılırsa sözdizimi hatası verir, parolayı "********" yapmaz). Değer yalnızca `ApplyOptions.password: Option<Secret>` ile `ddl_apply`'a gelir; yönetici kapılardan sonra gizlenmiş ifadeyi `ALTER ROLE <q> PASSWORD <quote_literal(değer)>` ile **yalnızca çalıştırma listesinde** değiştirir; bu metin günlüğe/hata yüküne girmez, başarısız ifade gizlenmişse `position` `None` döner (uzunluk sızmasın). `Secret`'in `Debug`'ı ve `Serialize`'ı `"********"` yazar; düşerken baytları sıfırlanır (en iyi çaba; ek bağımlılık yok). Sunucu tarafı: `log_statement = 'ddl'|'all'`, `pg_stat_statements` ve `pg_stat_activity` düz metni görebilir — arayüz uyarısı Plan 15'tedir (spec §5; istemci tarafı SCRAM özeti bu alt projede yok).
7. **Database ifadeleri transaction dışı (spec §5):** `CREATE/DROP DATABASE` ve `ALTER DATABASE … SET TABLESPACE` zorunlu olarak, diğerleri tutarlılık için transaction dışında ve **tek tek** çalışır; hata ortasında `ApplyOutcome::Failed.committed` kaç ifadenin kalıcı olduğunu söyler. Tüm Database (ve Role) ifadeleri **bakım veritabanında** çalışır.
8. **Bağlı veritabanı:** yöneticinin kendi havuzundaki o veritabanı bağlantıları (Gezgin/katalog) `RENAME`/`DROP`/`SET TABLESPACE` öncesi **kapatılır** (`ConnectionManager::release_database`, pgt-core); bakım veritabanının kendisi yeniden adlandırılamaz/silinemez/taşınamaz (yönetici kapısı, bağlantı açmadan, açık ileti). Başka oturumlar (Query Tool, Dashboard, başka istemciler) 55006 verir → ipucu: "Veritabanına bağlı oturumlar var: Query Tool sekmelerini kapatın ya da Dashboard'dan oturumları sonlandırın; PG 13+'ta silerken WITH (FORCE) kullanılabilir." Otomatik oturum sonlandırma yok.
9. **`DROP ROLE` bağımlılıkları otomatikleştirilmez:** sahip olunan nesne/yetki varsa PG 2BP01 (+ detayda veritabanı başına sayılar) döner, plan geri alınır; `REASSIGN OWNED`/`DROP OWNED` üretilmez (ileti ipucu bunu söyler). Yetkisiz işlem (ör. SUPERUSER/REPLICATION/BYPASSRLS vermek, ADMIN'siz GRANT) 42501 → ipucu: "Bağlı rolün yetkisi yetmiyor; SUPERUSER/REPLICATION/BYPASSRLS yalnızca süper kullanıcı verebilir."
10. **MatView `REFRESH` kapsamda:** alter-only `MatViewDef.refresh: Option<MatViewRefresh>` (`normal | concurrently`) ve `withData` değişimi `REFRESH MATERIALIZED VIEW [CONCURRENTLY] … [WITH NO DATA]` üretir (spec §3: Destructive). MV indeksleri Index türüne aittir; yeniden kurmada yeniden oluşturulmaz (uyarıda adları listelenir).
11. **`VALID UNTIL` ve tablespace temizlenemez:** PG `VALID UNTIL`'ı NULL'a döndüremez → yüklü değer varken `None` doğrulama hatası ("süre sınırını kaldırmak için `infinity` yazın"); tablespace kuralı Plan 10/12 ile aynı (`pg_default` açıkça seçilir). Database yükleyicisi tablespace'i her zaman `Some` döndürür.
12. **Kapsam dışı (backlog):** view/MV kolon yeniden adlandırma (`ALTER VIEW … RENAME COLUMN`), view/MV kolon yorumları ve kolon varsayılanları, MV indekslerinin yeniden kurmada otomatik yeniden oluşturulması, `CREATE DATABASE … STRATEGY/OID/ICU_RULES/COLLATION_VERSION`, `ALTER ROLE … IN DATABASE … SET` (yüklenir ama `unsupported`), rol "üyeleri" (ters yön), `REASSIGN/DROP OWNED`, GRANT matrisi.

## Global Constraints

- Plan 10 ve Plan 12'nin Global Constraints'i aynen geçerlidir: PostgreSQL **10–18** vanilla; tüm tanımlayıcı/değer yerleştirmeleri yalnızca `quote_ident`/`quote_literal`/`qualified`; ad alanlarında boş/NUL/denetim karakteri/görünmez biçim karakteri/63 bayt üstü reddedilir (`check_ident`); şema alanlarında sistem şeması reddedilir (`check_schema_ident`); yüklü (eski) taraf yalnızca kimlik alanlarıyla doğrulanır; alter yalnızca **değişen ya da ifadeye yazılan** alanları doğrular; istemciden SQL alınmaz; `Statement.sql` sonda `;` taşımaz ve tek ifadedir.
- **Serbest SQL alanları (bu plan):** yalnızca `ViewDef.query` ve `MatViewDef.query`. `check_free_sql("query", …)` + `wrap()` (`(<sorgu>\n)`) + nihai ifadede `composed_is_clean`; sıradan `'…'` içinde `\` reddedilir (`E'…'`). Not: `pg_get_viewdef` `standard_conforming_strings = on` altında `\` içeren sabitleri sıradan `'…'` olarak yazar; böyle bir görünümün **sorgusu** değiştirilmek istenirse kapı reddeder (sorguya dokunmayan düzenlemeler serbest; alan hatası `E'…'` ipucunu verir) — bilinen sınır, smoke notuna yazılır.
- **Serbest SQL olmayan değer alanları** yalnızca `quote_literal` ile ve önce katı doğrulamadan geçerek yerleşir: `DatabaseDef.encoding` (sabit sunucu kodlaması kümesi, büyük/küçük harf duyarsız), `lcCollate`/`lcCtype`/`providerLocale` (boş değil, NUL/denetim yok, ≤ 255 bayt), `RoleDef.validUntil` (dilbilgisi: `infinity` | `YYYY-MM-DD[(T| )HH:MM[:SS[.f{1,6}]]][Z|±HH[:MM]]`), `ConfigSetting.value` (NUL/denetim karakteri yok, ≤ 4096 bayt), `comment` (NUL yok). `ConfigSetting.name` (GUC adı) katı dilbilgisi: `^[a-z_][a-z0-9_$]*(\.[a-z_][a-z0-9_$]*)*$`, ≤ 128 karakter; her parça `quote_ident` ile yazılır (anahtar sözcük parçaları tırnaklanır).
- **GUC listesi tırnaklama:** `search_path`, `temp_tablespaces`, `local_preload_libraries`, `session_preload_libraries`, `shared_preload_libraries`, `unix_socket_directories` (pg_dump'ın `variable_is_guc_list_quote` kümesi) için değer PG `SplitGUCList` kuralıyla (virgül ayırıcı, baş/son boşluk kırpılır, `"…"` öğe ve `""` kaçışı) öğelere bölünür ve her öğe ayrı `quote_literal` olur: `SET search_path = '$user', 'public'`; kapanmamış `"` → alan hatası. Diğer adlar tek `quote_literal`.
- **Sürüm kapıları (doğrulama hatası, sessiz atlama yok):** `security_invoker` PG 15+; `DROP DATABASE … WITH (FORCE)` PG 13+; `localeProvider` (`LOCALE_PROVIDER`, `ICU_LOCALE`) PG 15+; `localeProvider = builtin` (`BUILTIN_LOCALE`) PG 17+; üyelik `inheritOption`/`setOption` yalnızca PG 16+ yüklemede dolar.
- **Risk sınıfları (bu plan; spec §3 ile uyumlu, Karar 2–3 hariç):**
  - View: `CREATE VIEW` Safe; `CREATE OR REPLACE VIEW` Safe + not; `RENAME`/`SET SCHEMA`/`OWNER`/`SET|RESET (seçenek)`/`COMMENT` Safe; `recreate` → `DROP VIEW` **DataLoss** + `CREATE VIEW` Safe; silme `DROP VIEW` Destructive, `CASCADE` DataLoss.
  - MatView: `CREATE MATERIALIZED VIEW … WITH DATA` Safe + not ("sorgu çalışır; uzun sürebilir"); `RENAME`/`SET SCHEMA`/`OWNER`/`SET|RESET (fillfactor)`/`COMMENT` Safe; `SET TABLESPACE` Safe + not (yeniden yazar, ACCESS EXCLUSIVE); `REFRESH …` (her biçim) **Destructive** + not (kilit; `CONCURRENTLY` benzersiz indeks ister); tanım değişikliği → `DROP MATERIALIZED VIEW` **DataLoss** + `CREATE …` Safe; silme Destructive, `CASCADE` DataLoss.
  - Database (hepsi `transactional = false`): `CREATE DATABASE` Safe; `RENAME`/`OWNER`/`WITH (…)`/`SET|RESET`/`COMMENT` Safe (ALLOW_CONNECTIONS false ve IS_TEMPLATE değişimi uyarılı); `SET TABLESPACE` Safe + not; `DROP DATABASE [WITH (FORCE)]` **DataLoss**.
  - Role (hepsi transactional): `CREATE ROLE` Safe; `ALTER ROLE … WITH …` Safe (SUPERUSER/REPLICATION/BYPASSRLS/CREATEROLE verilmesi plan uyarısı); `RENAME` Safe (+ MD5 uyarısı); parola `set`/`clear` Safe; `GRANT` Safe; `REVOKE` ve `REVOKE ADMIN OPTION FOR` **Destructive** (yetki kaybı); `SET|RESET` Safe; `COMMENT` Safe; `DROP ROLE` Destructive (CASCADE yok; `cascade = true` doğrulama hatası).
- **Plan sırası (alter, sonraki ifadeler YENİ adları kullanır):**
  - View: (1) `RENAME TO`, (2) `SET SCHEMA`, (3) sorgu değiştiyse `CREATE OR REPLACE VIEW <yeni> [WITH (<tüm yeni seçenekler>)] AS (<sorgu>\n)` (PG seçenek listesini **tümüyle değiştirir**, boş liste dahil) **ya da** sorgu değişmediyse seçenek farkı `ALTER VIEW … SET (…)`/`RESET (…)`, (4) `COMMENT ON VIEW`, (5) `OWNER TO` (**en sonda**: OWNER TO eski sahibin haklarını ister, öncekiler bu yüzden yetki bulur; önce gelseydi PG 16+'da INHERIT'siz/SET yetkili role devirden sonra COMMENT 42501 verirdi — Task 6 inceleme M-1, Database ile aynı kural). `recreate = true` ve sorgu değişmişse: (1–3) yerine `DROP VIEW <eski>` + `CREATE VIEW <yeni> …` (yeni ad/şema doğrudan), ardından yorum (doluysa) ve sahip **her zaman** (yeni nesne bağlı rolün olur; en sonda). `recreate = true` ama sorgu aynı → `recreate` yok sayılır + uyarı.
  - MatView: tanım (`query`) değiştiyse yeniden kurma: `DROP MATERIALIZED VIEW <eski>` + `CREATE MATERIALIZED VIEW <yeni> [WITH (fillfactor = n)] [TABLESPACE t] AS (<sorgu>\n) WITH [NO] DATA` + yorum (doluysa) + sahip (her zaman, en sonda); `refresh` yok sayılır (uyarı). Aksi halde: (1) `RENAME TO`, (2) `SET SCHEMA`, (3) `SET TABLESPACE`, (4) `SET (fillfactor = n)`/`RESET (fillfactor)`, (5) `COMMENT ON MATERIALIZED VIEW`, (6) REFRESH (aşağıdaki tablo), (7) `OWNER TO` (**en sonda**, View ile aynı gerekçe; bedeli: REFRESH sorguyu eski sahip olarak çalıştırır).
  - Database: (1) `RENAME TO`, (2) `WITH [ALLOW_CONNECTIONS b] [CONNECTION LIMIT n] [IS_TEMPLATE b]` (tek ifade, yalnız değişenler), (3) `SET TABLESPACE`, (4) ayarlar: önce kaldırılanlar `RESET <ad>` (ad sırasıyla), sonra eklenen/değişen `SET <ad> = <değer>` (ad sırasıyla), (5) `COMMENT ON DATABASE`, (6) `OWNER TO` (**en sonda**: ifadeler transaction dışıdır ve sonrakiler sahiplik ister; INHERIT'siz role devir önce yapılırsa kalanlar 42501 ile yarım kalırdı — Task 4 inceleme M-2).
  - Role: (1) `RENAME TO`, (2) `ALTER ROLE <q> WITH <değişen öznitelikler> [CONNECTION LIMIT n] [VALID UNTIL '…']` (tek ifade), (3) parola (`set` → gizlenmiş ifade; `clear` → `ALTER ROLE <q> PASSWORD NULL`), (4) üyelik çıkarma `REVOKE <r> FROM <q>` ve admin kaldırma `REVOKE ADMIN OPTION FOR <r> FROM <q>`, (5) üyelik ekleme `GRANT <r> TO <q> [WITH ADMIN OPTION]` ve admin verme `GRANT <r> TO <q> WITH ADMIN OPTION`, (6) ayarlar RESET sonra SET, (7) `COMMENT ON ROLE`. Üyelikler rol adı sırasıyla (deterministik).
- **Create-only alanlar** (Database: `template`, `encoding`, `localeProvider`, `lcCollate`, `lcCtype`, `providerLocale`): alter'da eski ≠ yeni → alan hatası "Veritabanı oluşturulduktan sonra değiştirilemez." Yükleyici `template`'i her zaman `None` (katalogda saklanmaz), diğerlerini `Some(gerçek değer)` döndürür.
- **Kapsam:** `ObjectKind::{Database, Role}.server_scoped() == true`. Yönetici bu türler için `database_oid`'i yok sayar ve `Target::Maintenance` kullanır (yükleme, nesne denetimi, `find_created`, uygulama); `DdlTarget.databaseOid` bu türlerde **her zaman `null`**. Veritabanı kapsamlı türde `database_oid = None` → `Invalid("Bu nesne türü bir veritabanı gerektirir.")`. `LookupRequest`'te sunucu genelindeki istekler (`Roles`, `GrantableRoles`, `Tablespaces`, `Encodings`, `TemplateDatabases`, `Locales`, `Settings`) `None` ile bakım veritabanında çalışır; diğerleri `None` ile `Invalid`.
- **Günlük/gizlilik:** SQL metni, ad, değer, **parola** ve URL asla `tracing`'e, hata iletisine, `PlanView`'e ya da diske yazılmaz; `Secret` yalnızca `apply` içinde çözülür. Docker testlerinde test rolü parolaları rastgele üretilir ve hiçbir yere yazdırılmaz; `PGT_TEST_URLS` yalnızca test komutunun ortamında.
- **Docker:** yalnızca `scripts/test-matrix.ps1` (`pgt-matrix-<v>` adlı geçici kapsayıcılar; bitince silinir); başka kapsayıcılara dokunulmaz. Küme düzeyindeki test nesneleri (veritabanı, rol) her testte benzersiz önekli ad alır (`pgt_ddl_dbk_<sürüm>_<etiket>`, `pgt_ddl_rk_<etiket>`) ve `guarded` temizliğiyle silinir; testler paralel koştuğu için ad çakışması yoktur.
- **Ön yüz derleme uyumu (yalnızca Task 1):** yeni `ObjectKind` değerleri ve yeni zorunlu alanlar (`Statement.redacted`, `ApplyOptions.password`, `PlanRequest.drop.force`) TS tiplerini değiştirir. Uyum **yalnızca tembel designer dosyalarında** sağlanır (`FORMS` `Partial` + geçici `formMissing` koruması, `KIND_LABELS`, `defName`, `applyOptionsFor` → `password: null`, `dropRequest` → `force: false`, test fikstürleri); `ddl-kinds.ts`'e yeni tür **eklenmez** (menü Plan 15'te). Ana `index-*.js` **tam 233,088 B** kalmalı; `index.html` yalnızca solid/web/Icon parçalarını önyükler; ana `__vite__mapDeps` değişmez.
- `ts-rs`, `Box::pin`, commit eki, git worktree yasağı Plan 10'daki gibi; `src/ipc/bindings` satır sonu gürültüsü `git checkout -- src/ipc/bindings`; yeni binding dosyaları commit edilir. Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Yorumlar Türkçe, çevredeki yoğunlukta.

## Review Focus

- **Parola gizliliği (en yüksek öncelik):** parola değeri `ObjectDef`/`PlanRequest`/`PlanView`/`plan_hash`/`tracing`/`ErrorPayload`/`ApplyOutcome`/`Debug` çıktısında hiçbir yoldan görünmez; gizlenmiş ifade yalnızca kapılardan sonra çalıştırma listesinde somutlaşır; gizlenmiş ifade yoksa parola reddedilir, varsa ve parola yoksa `role_password_missing`; başarısız gizlenmiş ifadede `position = None`; `********` tırnaksızdır (kopyalanan betik çalışmaz).
- **Enjeksiyon/tırnaklama:** view/MV sorgusu tek serbest SQL alanıdır ve tüm kapılardan geçer (alan + nihai ifade); GUC adı/değeri, encoding/locale/valid-until değerleri, rol/veritabanı/üyelik adları hostile girdilerle (`;`, `'`, `"`, `\`, `--`, `/*`, NUL, denetim karakteri, Unicode, anahtar sözcük, 63/64 bayt) tek ifade kalır.
- **Transaction dışı ve kısmi uygulama:** Database planlarında her ifade `transactional = false`; hata ortasında `committed` doğru; bakım veritabanı koruması; kendi havuz bağlantılarının serbest bırakılması; 55006 ipucu.
- **Yıkıcı işlemler ve kapılar:** DataLoss sınıfları (DROP DATABASE, view `recreate`, MV yeniden kurma, CASCADE) üretimde `production_confirmation_required`; REVOKE/REFRESH/DROP Destructive `confirmation_required`; kilitli sunucuda dört tür için `locked`; yeniden yükleme eşitliği (bayat `old`, oynanmış `def`, `password ≠ keep` gönderen eski taraf) `plan_changed`.
- **Gidiş-dönüş PG 10–18:** `create → load → eşit` (normalleştirme kuralları belgeli), `alter(old,new) → load → new`, `alter(loaded, loaded)` **boş plan** (view sorgusu `pg_get_viewdef` sabit noktası, `validUntil` UTC biçimi, ayar sırası/biçimi, PG 16 üyelik seçenekleri); sürüm kapıları (security_invoker 15+, FORCE 13+, ICU 15+, builtin 17+).
- **Kapsam ve hedefler:** Database/Role işlemleri `Target::Maintenance`'ta; `target_of` yeni düğümler (`dbs`, `db:N`, `roles`, `role:N`, `views`, `view:N`, `matviews`, `mv:N`) ve `refreshNodeId`; görünüm/MV kolon düğümleri hâlâ boş.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-ddl/src/model.rs` | `ObjectKind::{View, MaterializedView, Database, Role}`, `server_scoped()`, `ViewDef`, `ViewColumn`, `CheckOption`, `MatViewDef`, `MatViewRefresh`, `DatabaseDef`, `LocaleProvider`, `ConfigSetting`, `RoleDef`, `RoleMembership`, `PasswordChange`, `ObjectDef` varyantları |
| `crates/pgt-ddl/src/secret.rs` | `Secret` (gizli parola tipi) |
| `crates/pgt-ddl/src/plan.rs` | `Statement.redacted`, `REDACTED` |
| `crates/pgt-ddl/src/build/{mod,settings,view,matview,database,role}.rs` (+ anlık görüntüler) | Dispatch, `DropOptions`, ortak ayar (GUC) yardımcısı, dört türün üreticileri |
| `crates/pgt-ddl/src/load.rs`, `sql/load_view*.sql`, `load_matview*.sql`, `load_database*.sql`, `load_role*.sql` | Yükleyiciler (sürüm varyantlı) |
| `crates/pgt-ddl/src/lookup.rs`, `sql/lookup_{grantable_roles,template_databases,locales*,settings}.sql` | Yeni açılır listeler + kapsam |
| `crates/pgt-ddl/src/manager.rs`, `apply.rs`, `target.rs`, `error_codes.rs`, `lib.rs` | Kapsam/hedef çözümü, parola kapısı ve somutlaştırma, bakım veritabanı kapısı, ipuçları, `find_created`, `defaults`, `identity_changed`, `target_of` |
| `crates/pgt-core/src/connection.rs` | `ConnectionManager::release_database` |
| `src-tauri/src/ddl.rs` | `database_oid: Option<u32>` |
| `crates/pgt-ddl/tests/{view_roundtrip,cluster_roundtrip,manager,logging,json}.rs`, `tests/common/mod.rs` | Docker gidiş-dönüş, yönetici kapıları, gizlilik |
| `src/features/designer/{DesignerView.tsx,designer-model.ts,drop-model.ts}`, `src/app/strings-designer.ts`, designer test fikstürleri | Yalnızca tembel derleme uyumu (Task 1) |

---

### Task 1: Model, gizli tip, plan alanları, dispatch iskeleti ve tembel derleme uyumu

**Dosyalar:**
- Değiştir: `crates/pgt-ddl/src/model.rs`, `plan.rs`, `build/mod.rs`, `manager.rs` (yalnızca `PlanRequest::Drop.force`, `ApplyOptions.password`, `plan_view` drop seçenekleri), `error_codes.rs`, `lib.rs`, mevcut testlerdeki `Statement {…}`/`ApplyOptions {…}`/`PlanRequest::Drop {…}` yapı değişmezleri (mekanik: `redacted: false`, `password: None`, `force: false`)
- Oluştur: `crates/pgt-ddl/src/secret.rs`
- Değiştir (TS, tembel): `src/features/designer/DesignerView.tsx`, `designer-model.ts`, `drop-model.ts`, `src/app/strings-designer.ts`, `designer-controller.test.ts`, `designer-model.test.ts`, `drop-controller.test.ts` (fikstürler)

**Arayüzler:**
- Tüketir: mevcut model, `Plan`/`Statement`, `PlanRequest`, `ApplyOptions`.
- Üretir (`model.rs`; hepsi `Serialize + Deserialize + TS`, camelCase, `#[ts(export)]`, enum'lar snake_case, sayılar `#[ts(type = "number")]`):

```rust
#[serde(rename_all = "snake_case")]
pub enum ObjectKind { Schema, Sequence, Extension, Index, Table, View, MaterializedView, Database, Role }

impl ObjectKind {
    /// Sunucu kapsamlı (küme düzeyi) nesne: yükleme/uygulama bakım veritabanında; `database_oid` yok sayılır.
    pub fn server_scoped(self) -> bool { matches!(self, ObjectKind::Database | ObjectKind::Role) }
}

#[serde(rename_all = "snake_case")] pub enum CheckOption { Local, Cascaded }

/// Görünüm/MV kolonu: yalnızca yüklemede dolar (salt okunur gösterim; üretici okumaz).
pub struct ViewColumn { pub name: String, pub data_type: String }

pub struct ViewDef {
    pub schema: String, pub name: String, pub owner: Option<String>,
    /// Serbest SQL (SELECT/VALUES/WITH … sorgusu); `(…\n)` sarmalıyla yerleşir.
    pub query: String,
    pub check_option: Option<CheckOption>,
    pub security_barrier: bool,
    /// PG 15+.
    pub security_invoker: bool,
    pub comment: Option<String>,
    /// Yalnızca alter, kalıcı değil: sorgu değişince DROP + CREATE (Karar 1). Yüklemede `false`.
    #[serde(default)] pub recreate: bool,
    /// Yalnızca yükleme.
    #[serde(default)] pub columns: Vec<ViewColumn>,
    /// Yalnızca yükleme: yeniden kurmada kaybolacak modellenmeyen ekler ("TRIGGER t", "RULE r", "DEFAULT c", "COMMENT c").
    #[serde(default)] pub unsupported: Vec<String>,
}

#[serde(rename_all = "snake_case")] pub enum MatViewRefresh { Normal, Concurrently }

pub struct MatViewDef {
    pub schema: String, pub name: String, pub owner: Option<String>,
    pub query: String,                                  // serbest SQL
    pub tablespace: Option<String>,
    #[ts(type = "number | null")] pub fillfactor: Option<u32>,
    /// Create: WITH [NO] DATA; yükleme: `relispopulated`; alter: değişimi REFRESH üretir.
    pub with_data: bool,
    pub comment: Option<String>,
    /// Yalnızca alter, kalıcı değil (Karar 10). Yüklemede `None`.
    #[serde(default)] pub refresh: Option<MatViewRefresh>,
    #[serde(default)] pub columns: Vec<ViewColumn>,     // yalnızca yükleme
    /// Yalnızca yükleme: "INDEX ad", "WITH (k=v)" (fillfactor dışı), "COMMENT c".
    #[serde(default)] pub unsupported: Vec<String>,
}

#[serde(rename_all = "snake_case")] pub enum LocaleProvider { Libc, Icu, Builtin }

/// `ALTER DATABASE|ROLE … SET name = value` girdisi (rol/veritabanından bağımsız ayar).
pub struct ConfigSetting { pub name: String, pub value: String }

pub struct DatabaseDef {
    pub name: String, pub owner: Option<String>,
    // --- yalnızca create (alter'da değişirse alan hatası); yükleme: template None, diğerleri Some
    pub template: Option<String>,
    pub encoding: Option<String>,
    /// PG 15+.
    pub locale_provider: Option<LocaleProvider>,
    pub lc_collate: Option<String>,
    pub lc_ctype: Option<String>,
    /// `ICU_LOCALE` (icu, PG 15+) ya da `BUILTIN_LOCALE` (builtin, PG 17+); libc'de None olmalı.
    pub provider_locale: Option<String>,
    // --- düzenlenebilir
    pub tablespace: Option<String>,
    #[ts(type = "number")] pub connection_limit: i32,   // -1 = sınırsız
    pub is_template: bool,
    pub allow_connections: bool,
    pub settings: Vec<ConfigSetting>,
    pub comment: Option<String>,
}

/// Parola niyeti; değer asla tanımda değildir (Karar 6).
#[serde(rename_all = "snake_case")] #[derive(Default)] pub enum PasswordChange { #[default] Keep, Set, Clear }

pub struct RoleMembership {
    /// Üye olunan rol (önceden tanımlı `pg_*` roller dahil).
    pub role: String,
    pub admin: bool,
    /// Yalnızca yükleme, PG 16+ (salt okunur; üretici yazmaz).
    #[serde(default)] pub inherit_option: Option<bool>,
    #[serde(default)] pub set_option: Option<bool>,
}

pub struct RoleDef {
    pub name: String,
    pub login: bool, pub superuser: bool, pub createdb: bool, pub createrole: bool,
    pub inherit: bool, pub replication: bool, pub bypassrls: bool,
    #[ts(type = "number")] pub connection_limit: i32,  // -1 = sınırsız
    /// `infinity` ya da ISO zaman damgası; yükleme UTC biçimi `YYYY-MM-DDTHH:MM:SS[.ffffff]Z`.
    pub valid_until: Option<String>,
    /// Yalnızca yazılır; yükleme her zaman `keep`.
    #[serde(default)] pub password: PasswordChange,
    pub member_of: Vec<RoleMembership>,
    pub settings: Vec<ConfigSetting>,
    pub comment: Option<String>,
    /// Yalnızca yükleme: "IN DATABASE d: k=v", "MEMBER r (birden çok grantor)".
    #[serde(default)] pub unsupported: Vec<String>,
}

// ObjectDef'e: View(ViewDef) "view", MaterializedView(MatViewDef) "materialized_view",
// Database(DatabaseDef) "database", Role(RoleDef) "role". kind()/name() kolları.
```

- Üretir (`secret.rs`):

```rust
//! Uygulamada gelen parola: Debug/Serialize asla değeri yazmaz; düşerken bayt sıfırlanır (en iyi çaba).
#[derive(Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(transparent)]
#[ts(export, type = "string")]
pub struct Secret(String);
impl Secret {
    pub fn expose(&self) -> &str { &self.0 }
    #[cfg(test)] pub fn new(s: &str) -> Self { Secret(s.to_string()) }
}
impl std::fmt::Debug for Secret { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("Secret(********)") } }
impl Serialize for Secret { fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(crate::plan::REDACTED) } }
impl Drop for Secret {
    fn drop(&mut self) {
        let mut b = std::mem::take(&mut self.0).into_bytes();
        b.fill(0);
        std::hint::black_box(&b);
    }
}
```

- Üretir (`plan.rs`): `pub const REDACTED: &str = "********";` ve `Statement`'e `#[serde(default)] pub redacted: bool` (doc: "Metin gizli değer yerine `REDACTED` taşır; gerçek ifade yalnızca uygulamada kurulur. Arayüz bu ifadeyi Query Tool'a aktarmaz."). `plan_hash` alanı da özetler (değişmez kalıp).
- Üretir (`manager.rs`): `PlanRequest::Drop { object, def, cascade, #[serde(default)] force: bool }` (`force` yalnızca Database: `WITH (FORCE)`); `ApplyOptions { …, #[serde(default)] #[ts(type = "string | null")] password: Option<Secret> }` (`Debug` gizli).
- Üretir (`build/mod.rs`): `#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)] pub struct DropOptions { pub cascade: bool, pub force: bool }`, `pub fn drop_with(def: &ObjectDef, o: DropOptions, ctx: &BuildCtx) -> Result<Plan, Vec<FieldError>>`; mevcut `drop(def, cascade, ctx)` = `drop_with(def, DropOptions { cascade, force: false }, ctx)` (çağıranlar değişmez). Ortak kural `drop_with` içinde: `force && kind != Database` → `err("force", "WITH (FORCE) yalnızca veritabanı silmede kullanılır.")`; `cascade && kind ∈ {Database, Role}` → `err("cascade", "Bu tür CASCADE ile silinemez.")`. `plan_view`'in Drop kolu `drop_with(def, DropOptions { cascade, force }, …)` çağırır. `pub(crate) fn stmt_nt(sql, risk, note) -> Statement` (transaction dışı yardımcı; `stmt` gibi, `transactional: false`). Yeni dört tür için `validate/validate_identity/create/alter/drop_with` kolları **bu görevde** tek yer tutucu hatayla bağlanır: `err("kind", "Bu tür henüz desteklenmiyor.")` (Task 2–5 değiştirir); `validate_identity` kolları şimdiden gerçektir (View/MatView: `check_schema_ident("schema")` + `check_ident("name")`; Database/Role: `check_ident("name")`).
- Üretir (`error_codes.rs`): `ROLE_PASSWORD_MISSING = "Bu plan rol parolası ayarlıyor; parola girilmedi."` → `DdlErrorCode::RolePasswordMissing` (`"role_password_missing"`; pgt-core'un bağlantı `password_required` koduyla **karışmasın** diye ayrı ad), `DDL_ERROR_CODES` 6 öğe; kodsuz düz iletiler: `PASSWORD_NOT_USED = "Bu planda parola kullanılmıyor."`, `MAINTENANCE_DATABASE = "Bakım veritabanı yeniden adlandırılamaz, silinemez ya da taşınamaz; sunucu ayarlarından başka bir bakım veritabanı seçin."`, `NEEDS_DATABASE = "Bu nesne türü bir veritabanı gerektirir."`.
- Üretir (TS uyum, tembel): `designer-model.ts` `KIND_LABELS`'e `view: "View", materialized_view: "Materialized View", database: "Database", role: "Role"`; `defName`'de view/materialized_view şema nitelikli; `applyOptionsFor` dönüşüne `password: null`; `drop-model.ts` `dropRequest(object, def, cascade, force = false)` → `{ op: "drop", object, def, cascade, force }`; `DesignerView.tsx` `FORMS: Partial<Record<ObjectKind, FormEntry>>` + `entry()` yoksa tek satır şerit `tdes.view.formMissing` ("Bu tür için form henüz yok.") — Plan 15 kaldırır; `strings-designer.ts`'e yalnızca bu bir satır; designer test fikstürlerinde `Statement` nesnelerine `redacted: false`.

- [ ] **Adım 1: Başarısız testler.** `model.rs`: dört yeni tür için JSON gidiş-dönüş (etiketler `view`/`materialized_view`/`database`/`role`; `checkOption: "cascaded"`, `localeProvider: "icu"`, `password: "set"`, `refresh: "concurrently"`, `connectionLimit: -1`); eksik yükleme/alter-only alanların varsayılanı (`recreate=false`, `columns=[]`, `unsupported=[]`, `refresh=null`, `password="keep"`, `inheritOption=null`); geçersiz enum reddi (`password: "Set"`, `checkOption: "LOCAL"`, `localeProvider: "ICU"`); `ObjectKind::server_scoped` tablosu (yalnızca database/role true); `kind()/name()` her varyant. `secret.rs`:

```rust
#[test]
fn secret_never_prints_or_serializes_its_value() {
    let s: Secret = serde_json::from_value(serde_json::json!("p@ss'w\\rd")).unwrap();
    assert_eq!(s.expose(), "p@ss'w\\rd");
    assert!(!format!("{s:?}").contains("p@ss"));
    assert_eq!(serde_json::to_value(&s).unwrap(), serde_json::json!("********"));
    let o = ApplyOptions { apply_id: Uuid::nil(), lock_timeout_ms: 0, expected_hash: "h".into(), confirmed: false, confirmed_production: false, password: Some(s) };
    assert!(!format!("{o:?}").contains("p@ss"));
    assert!(!serde_json::to_string(&o).unwrap().contains("p@ss"));
}
```

  (Bu test `ApplyOptions`'ı kullandığı için `manager.rs` testlerindedir; `secret.rs` kendi testinde yalnızca `Secret`'i dener.) `plan.rs`: `redacted` hash'e girer (`redacted` farklı → hash farklı) ve JSON'da `redacted: false` varsayılanı (alan yokken). `build/mod.rs`: `drop_with` ortak kuralları (`force` + Schema → `force` hatası; `cascade` + Database/Role → `cascade` hatası), yer tutucu kollar `kind` hatası, `validate_identity` yeni kollar (`pg_catalog` şemalı view → `schema`; `"x\n"` adlı rol → `name`). `manager.rs`: `PlanRequest::Drop` JSON'unda `force` yokken `false`; `plan_view(Drop{force:true, def: Schema})` → `errors[0].path == "force"`. `error_codes.rs`: yeni kod listesi testi (mevcut kalıp).
- [ ] **Adım 2:** `cargo test -p pgt-ddl` — derleme/başarısızlık gör. **Adım 3: Uygula** (mekanik yapı değişmezi güncellemeleri dahil; mevcut tüm testler ve anlık görüntüler değişmeden geçer).
- [ ] **Adım 4: TS uyumu.** `cargo test -p pgt-ddl` binding dosyalarını üretir (`ViewDef.ts`, `ViewColumn.ts`, `CheckOption.ts`, `MatViewDef.ts`, `MatViewRefresh.ts`, `DatabaseDef.ts`, `LocaleProvider.ts`, `ConfigSetting.ts`, `RoleDef.ts`, `RoleMembership.ts`, `PasswordChange.ts`, `Secret.ts`; değişen `ObjectKind.ts`, `ObjectDef.ts`, `Statement.ts`, `PlanRequest.ts`, `ApplyOptions.ts`, `DdlErrorCode.ts`). Tembel uyum değişikliklerini yap; `bun run typecheck`, `bun run test` (tümü yeşil), **boyut doğrulaması**:

  ```powershell
  bun run build
  (Get-Item dist/assets/index-*.js).Length     # tam 233088
  Select-String -Path dist/index.html -Pattern 'modulepreload'   # yalnızca solid / web / Icon parçaları
  ```

  ve `git diff --stat -- src/app/App.tsx src/features/query src/features/explorer src/ipc/commands.ts src/features/designer/ddl-kinds.ts` → **boş**.
- [ ] **Adım 5:** `cargo clippy -p pgt-ddl --all-targets -- -D warnings`; bindingleri ekle. **Commit** (`feat(ddl): View/MatView/Database/Role modeli, Secret, Statement.redacted, drop FORCE seçeneği; tembel derleme uyumu`).

---

### Task 2: Ortak ayar (GUC) yardımcısı ve View üreticisi

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/settings.rs`, `crates/pgt-ddl/src/build/view.rs` (+ anlık görüntüler)
- Değiştir: `crates/pgt-ddl/src/build/mod.rs` (View kolları)

**Arayüzler:**
- Tüketir: Task 1 modeli, `freesql::{check_free_sql, wrap, composed_is_clean}`, `check_ident`/`check_schema_ident`/`check_text`, `stmt`, `quote_*`.
- Üretir (`settings.rs`, saf):

```rust
/// pg_dump `variable_is_guc_list_quote` kümesi (değeri öğe öğe tırnaklanan liste ayarları).
pub const LIST_QUOTE_GUCS: [&str; 6] = ["local_preload_libraries", "search_path", "session_preload_libraries",
    "shared_preload_libraries", "temp_tablespaces", "unix_socket_directories"];
/// `settings[i].name|value` doğrulaması; yinelenen ad → `settings[i].name`.
pub fn validate_settings(list: &[ConfigSetting], errs: &mut Vec<FieldError>);
/// PG `SplitGUCList`: virgülle ayrılmış, `"…"` öğeler (`""` kaçış), baş/son boşluk kırpılır. Kapanmamış tırnak → Err.
pub fn split_guc_list(value: &str) -> Result<Vec<String>, String>;
/// `<parça>.<parça>` → her parça quote_ident.
pub fn guc_name_sql(name: &str) -> String;
/// `name = <değer>`: liste ayarında öğe başına quote_literal (', ' ile), diğerlerinde tek quote_literal.
pub fn guc_assignment_sql(s: &ConfigSetting) -> String;
/// Fark: (kaldırılan adlar, eklenen/değişen ayarlar), her ikisi ad sırasıyla (deterministik).
pub fn diff_settings<'a>(old: &'a [ConfigSetting], new: &'a [ConfigSetting]) -> (Vec<&'a str>, Vec<&'a ConfigSetting>);
```

- Üretir (`view.rs`): `pub fn validate(d: &ViewDef, ctx: &BuildCtx) -> Vec<FieldError>`, `pub fn create(d) -> Plan`, `pub fn alter(old, new, ctx) -> Result<Plan, Vec<FieldError>>`, `pub fn drop(d, cascade) -> Plan`.

**Doğrulama (FieldError yolları):** `schema` (`check_schema_ident`), `name`, `owner`, `comment`, `query` (`check_free_sql`; yalnızca create'te ve alter'da sorgu değiştiyse), `securityInvoker` (`true` ve `version_num < 150000` → "security_invoker PG 15+ gerektirir"), nihai ifade taraması (`composed_is_clean` başarısız → `query` yolunda "Sorgu, yerleştirildiği ifadeyi bozuyor"). Alter: `old` için yalnızca kimlik (`old.schema`, `old.name`); `new` için değişen alanlar; `new.columns`/`new.unsupported` üretici tarafından **okunmaz**.

**Üretim:**
- Seçenek listesi (deterministik sıra): `check_option = local|cascaded`, `security_barrier = true`, `security_invoker = true` — yalnızca `Some`/`true` olanlar; liste boşsa `WITH` yazılmaz.
- create: `CREATE VIEW <schema.name> [WITH (<seçenekler>)] AS <wrap(query)>` (Safe) + `ALTER VIEW <q> OWNER TO <o>` (owner varsa) + `COMMENT ON VIEW <q> IS <lit>` (doluysa).
- alter: Global Constraints'teki View sırası. `CREATE OR REPLACE VIEW` notu: "Mevcut kolonlar aynı ad, tip ve sırada kalmalı; yeni kolonlar yalnız sona eklenebilir. Uyumsuzsa uygulama geri alınır; 'Yeniden oluştur' seçeneğini kullanın." Seçenek farkı (sorgu aynıyken): değişip `Some`/`true` olanlar tek `ALTER VIEW <q> SET (…)`, `None`/`false`'a dönenler tek `ALTER VIEW <q> RESET (…)` (ikisi de varsa önce RESET). `recreate`: `DROP VIEW <eski>` (DataLoss, not "Görünüm silinip yeniden oluşturulur: GRANT yetkileri, tetikleyiciler/kurallar ve kolon varsayılanları/yorumları kaybolur; bağımlı nesne varsa uygulama başarısız olur") + `CREATE VIEW <yeni> …` + yorum + `ALTER VIEW <yeni> OWNER TO <new.owner ?? old.owner>` (sahip her zaman yazılır, en sonda); plan uyarısı: "Yeniden oluşturma: yetkiler (GRANT) yeniden verilmelidir" ve `old.unsupported` doluysa "Kaybolacaklar: <liste>".
- drop: `DROP VIEW <q>` Destructive / `DROP VIEW <q> CASCADE` DataLoss (uyarı "Bağımlı görünümler de silinir").
- Boş değişiklik → boş plan; `recreate` sorgu değişmeden → boş plan + uyarı ("Sorgu değişmedi; yeniden oluşturma yok sayıldı").

- [ ] **Adım 1: Başarısız testler.** `settings.rs`:

```rust
#[test]
fn guc_lists_are_split_and_quoted_per_element() {
    let s = |n: &str, v: &str| ConfigSetting { name: n.into(), value: v.into() };
    assert_eq!(guc_assignment_sql(&s("search_path", "\"$user\", public")), "search_path = '$user', 'public'");
    assert_eq!(guc_assignment_sql(&s("search_path", "\"My \"\"S\"\"\", app")), "search_path = 'My \"S\"', 'app'");
    assert_eq!(guc_assignment_sql(&s("work_mem", "64MB")), "work_mem = '64MB'");
    assert_eq!(guc_assignment_sql(&s("app.mode", "it's")), "app.mode = 'it''s'");
    assert_eq!(guc_assignment_sql(&s("app.user", "x")), "app.\"user\" = 'x'");        // anahtar sözcük parçası tırnaklı
    assert_eq!(guc_assignment_sql(&s("app.p", "a\\b")), "app.p = E'a\\\\b'");
    assert!(split_guc_list("\"unterminated, x").is_err());
    assert_eq!(split_guc_list(" a ,b,  \"c d\" ").unwrap(), vec!["a", "b", "c d"]);
}
#[test]
fn setting_names_and_values_are_validated() {
    // Kabul: work_mem, app.mode, a_b.c$d; Ret: "", "Work_Mem", "a-b", "a..b", ".a", "a.", "a b", "a;b", "1a", 129 karakter,
    // "a\0", değerde NUL, değerde \n, 4097 baytlık değer, yinelenen ad (settings[1].name).
}
#[test]
fn settings_diff_is_sorted_reset_then_set() { /* old {a=1,b=2,c=3} new {c=3,b=9,d=4} → removed ["a"], set [b=9, d=4] */ }
```

  `view.rs` (tablo odaklı + `insta`): create varyantları (seçeneksiz; check_option local/cascaded; security_barrier; security_invoker × `140000` hata / `150000` tamam; owner + yorum), alter tek tek (rename; şema; sorgu → `CREATE OR REPLACE` + notu; sorgu + seçenek değişimi aynı anda → tek C-O-R ve `WITH` yeni seçeneklerin tamamı, ALTER SET/RESET **yok**; seçenek yalnız SET; yalnız RESET; SET+RESET sırası; owner; yorum `IS NULL`), birleşik sıra anlık görüntüsü (rename + şema + sorgu + owner + yorum: C-O-R yeni şema.ad'ı kullanır), `recreate` (DROP eski ad + CREATE yeni ad + OWNER her zaman + yorum; risk DataLoss; uyarılar; `old.unsupported` listesi uyarıda), `recreate` sorgu aynı → boş + uyarı, `alter(x,x)` boş, drop matrisi, **enjeksiyon matrisi**: `query` için `"1); DROP TABLE t; --"`, `"x'"`, `"SELECT 1 --"`, `"SELECT 1;"`, `"SELECT '\\'"`, `"(SELECT 1"`, `"SELECT 1)"`, `"$$"`, `"SELECT 1$$x$$"` → `query` hatası; kabul edilenler (`"SELECT 1 -- yorum\n"`, `"SELECT ';'"`, `"SELECT E'\\\\'"`, `"WITH a AS (SELECT 1) SELECT * FROM a"`, `"VALUES (1), (2)"`) → `assert_single_statements`; ad alanlarında hostile adlar (`"v\"; DROP VIEW x; --"`, anahtar sözcük `select`, Unicode, 63/64 bayt, `"a\nb"`) → tek ifade / alan hatası; yüklü sorgu değişmeden kalan ve kapıya takılacak (`'a\b'` içeren) bir görünüm **yeniden adlandırılabilir** (sorgu doğrulanmaz) ama sorgusu değişince `query` hatası.
- [ ] **Adım 2–4:** Başarısızlığı gör; uygula; `cargo test -p pgt-ddl` + clippy temiz; anlık görüntüler commit edilir (`.snap.new` yok).
- [ ] **Adım 5: Commit** (`feat(ddl): ortak GUC ayar yardımcısı ve View üreticisi (CREATE OR REPLACE, seçenekler, açık yeniden oluşturma)`).

---

### Task 3: Materialized View üreticisi

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/matview.rs` (+ anlık görüntüler); Değiştir: `build/mod.rs`

**Arayüzler:**
- Tüketir: Task 1–2 (`freesql`, ortak doğrulayıcılar); Table'daki tablespace temizleme kuralının iletisi (aynı metin).
- Üretir: `matview::{validate, create, alter, drop}` (View ile aynı imzalar).

**Kurallar:**
- Doğrulama: `schema`, `name`, `owner`, `comment`, `tablespace` (ad), `fillfactor` 10–100, `query` (create'te ve alter'da sorgu değiştiyse), nihai ifade taraması; alter'da `old.tablespace` dolu ve `new.tablespace == None` → `tablespace` hatası ("pg_default'u açıkça seçin"); REFRESH kuralları:

| `old.withData` → `new.withData` | `new.refresh` | Sonuç |
|---|---|---|
| false → true | `None` ya da `Normal` | `REFRESH MATERIALIZED VIEW <q>` |
| false → true | `Concurrently` | `refresh` hatası: "Veri yokken CONCURRENTLY kullanılamaz." |
| true → false | `None` | `REFRESH MATERIALIZED VIEW <q> WITH NO DATA` |
| true → false | `Some(_)` | `refresh` hatası: "Veriyi boşaltırken yenileme seçilemez." |
| true → true | `Normal` / `Concurrently` | `REFRESH MATERIALIZED VIEW [CONCURRENTLY] <q>` |
| false → false | `Some(_)` | `refresh` hatası: "Önce 'Veriyle doldur'u seçin." |

  REFRESH notları: Normal → "ACCESS EXCLUSIVE kilit: yenileme süresince okunamaz"; Concurrently → "Benzersiz (UNIQUE) bir indeks gerekir; okumalar sürer"; WITH NO DATA → "Veri boşaltılır; sorgulanamaz hale gelir". Hepsi **Destructive**.
- create: `CREATE MATERIALIZED VIEW <schema.name> [WITH (fillfactor = n)] [TABLESPACE <q>] AS <wrap(query)> WITH [NO] DATA` (Safe; `withData` ise not "Sorgu çalıştırılır; uzun sürebilir") + owner + `COMMENT ON MATERIALIZED VIEW`; `refresh` create'te dolu → `refresh` hatası.
- alter: Global Constraints'teki MatView sırası. Yeniden kurma (sorgu değişti): `DROP MATERIALIZED VIEW <eski>` (**DataLoss**, not "Veri, indeksler ve yetkiler silinir; bağımlı görünüm varsa başarısız olur") + `CREATE …` (yeni ad/şema/tablespace/fillfactor/withData) + yorum + `ALTER MATERIALIZED VIEW <yeni> OWNER TO <new.owner ?? old.owner>` (en sonda); uyarılar: "İndeksler yeniden oluşturulmaz: <old.unsupported içindeki INDEX adları>" (varsa), "Yetkiler (GRANT) yeniden verilmelidir"; `refresh` dolu → yok sayılır + uyarı.
- drop: `DROP MATERIALIZED VIEW <q>` Destructive / `CASCADE` DataLoss.

- [ ] **Adım 1: Başarısız testler** (`insta` + açık `assert_eq!`): create (WITH DATA/NO DATA, fillfactor, tablespace, owner, yorum), alter tek tek (rename, şema, tablespace + temizleme hatası, fillfactor set/reset, owner, yorum), REFRESH tablosunun **her satırı** (ifade metni, risk, not, hata yolu), yeniden kurma (sorgu değişimi + rename + tablespace aynı planda: DROP eski + CREATE yeni değerlerle; risk DataLoss; INDEX uyarısı; `refresh` yok sayılır uyarısı), birleşik sıra anlık görüntüsü, `alter(x,x)` boş, drop matrisi, enjeksiyon matrisi (Task 2 sorgu girdileri + ad alanları), risk sınıfları tablosu (tek test: her ifade türü → beklenen risk).
- [ ] **Adım 2–4:** Uygula; `cargo test -p pgt-ddl` + clippy temiz.
- [ ] **Adım 5: Commit** (`feat(ddl): Materialized View üreticisi (REFRESH kuralları, yeniden kurma DataLoss)`).

---

### Task 4: Database üreticisi (transaction dışı, create-only alanlar, sürüm kapıları)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/database.rs` (+ anlık görüntüler); Değiştir: `build/mod.rs`

**Arayüzler:**
- Tüketir: Task 1 (`stmt_nt`, `DropOptions`), Task 2 (`settings`).
- Üretir: `database::{validate, create, alter, drop}`; `pub const SERVER_ENCODINGS: &[&str]` (PG sunucu kodlamaları: `BIG5`/`GB18030`/`GBK`/`JOHAB`/`SJIS`/`SHIFT_JIS_2004`/`UHC` **hariç** — bunlar yalnızca istemci kodlaması): `EUC_CN, EUC_JP, EUC_JIS_2004, EUC_KR, EUC_TW, ISO_8859_5, ISO_8859_6, ISO_8859_7, ISO_8859_8, KOI8R, KOI8U, LATIN1, LATIN2, LATIN3, LATIN4, LATIN5, LATIN6, LATIN7, LATIN8, LATIN9, LATIN10, MULE_INTERNAL, SQL_ASCII, UTF8, WIN866, WIN874, WIN1250, WIN1251, WIN1252, WIN1253, WIN1254, WIN1255, WIN1256, WIN1257, WIN1258` (lookup `Encodings` da bunu döndürür).

**Doğrulama:** `name` (`check_ident`), `owner`, `template` (ad), `encoding` (küme; büyük/küçük harf duyarsız, yazıldığı gibi `quote_literal`), `lcCollate`/`lcCtype`/`providerLocale` (değer kuralı), `localeProvider` (PG < 15 → hata; `builtin` ve PG < 17 → hata), `providerLocale` (`icu`/`builtin` iken zorunlu; `libc` ya da `None` iken dolu → hata), `tablespace` (ad; alter'da temizleme hatası), `connectionLimit` (`-1..=i32::MAX`), `settings` (Task 2), `comment`. Alter: create-only alanlar `old` ≠ `new` → kendi yolunda "Veritabanı oluşturulduktan sonra değiştirilemez." Drop: `def.isTemplate == true` → `isTemplate` hatası ("Şablon veritabanı silinemez; önce 'Şablon' işaretini kaldırın"); `force` ve PG < 13 → `force` hatası.

**Üretim (HER ifade `stmt_nt`, `transactional = false`):**
- create: `CREATE DATABASE <q> [OWNER = <q>] [TEMPLATE = <q>] [ENCODING = <lit>] [LOCALE_PROVIDER = libc|icu|builtin] [LC_COLLATE = <lit>] [LC_CTYPE = <lit>] [ICU_LOCALE = <lit> | BUILTIN_LOCALE = <lit>] [TABLESPACE = <q>] ALLOW_CONNECTIONS = true|false CONNECTION LIMIT = n IS_TEMPLATE = true|false` (son üçü her zaman yazılır — deterministik; Safe, not "Transaction dışında çalışır"); kodlama/yerel ayar dolu ve şablon `template0` değilse uyarı: "Kodlama ya da yerel ayar şablondan farklıysa TEMPLATE = template0 seçin."; ardından ayarlar `ALTER DATABASE <q> SET …` (her biri ayrı), `COMMENT ON DATABASE <q> IS <lit>`.
- alter: Global Constraints'teki Database sırası; `RENAME` notu "Veritabanına bağlı oturum olmamalı (bu uygulamanın Gezgin bağlantıları kapatılır)"; `SET TABLESPACE` notu "Veritabanı dosyaları taşınır; bağlı oturum olmamalı; uzun sürebilir"; `ALLOW_CONNECTIONS false` uyarısı "Yeni bağlantılar reddedilir (Gezgin dahil)"; `IS_TEMPLATE true` uyarısı "Herkes (CREATEDB yetkili) bu veritabanını şablon olarak kopyalayabilir".
- drop: `DROP DATABASE <q>` / PG 13+ `DROP DATABASE <q> WITH (FORCE)` — **DataLoss**, not "Tüm veriler kalıcı silinir"; FORCE notu "Bağlı tüm oturumlar sonlandırılır (bu uygulamanın Query Tool sekmeleri dahil)".

- [ ] **Adım 1: Başarısız testler** (`insta` + açık): create varyantları (yalnız ad; tüm seçenekler libc; icu × `140000` hata/`150000` tamam; builtin × `160000` hata/`170000` tamam; `providerLocale` zorunluluk/yasak; template0 uyarısı; ayarlar + yorum sırası), **her ifadenin `transactional == false`** olduğu (tek test, tüm create/alter/drop çıktıları), alter tek tek (rename; owner; WITH birleşimi — yalnız değişenler, sabit sıra ALLOW_CONNECTIONS, CONNECTION LIMIT, IS_TEMPLATE; tablespace + temizleme hatası; ayar RESET/SET sırası; yorum), create-only alan değişimi → her alan için hata yolu, birleşik sıra anlık görüntüsü, `alter(x,x)` boş, drop (FORCE × `120000` hata/`130000` tamam; şablon → hata; risk DataLoss), `connectionLimit` sınırları (`-2` hata, `-1`/`0`/`i32::MAX` tamam), encoding (küme dışı `BIG5` ve `utf-8` hata, `utf8` tamam), enjeksiyon matrisi (ad, owner, template, tablespace, locale değerleri `"C'; DROP DATABASE x; --"`, `"en_US\0"`, `"a\nb"`, ayar adı/değeri) → tek ifade ya da alan hatası.
- [ ] **Adım 2–4:** Uygula; `cargo test -p pgt-ddl` + clippy temiz.
- [ ] **Adım 5: Commit** (`feat(ddl): Database üreticisi (transaction dışı, create-only alanlar, ICU/builtin ve FORCE sürüm kapıları)`).

---

### Task 5: Role üreticisi (öznitelikler, gizlenmiş parola, üyelikler, ayarlar)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/build/role.rs` (+ anlık görüntüler); Değiştir: `build/mod.rs`

**Arayüzler:**
- Tüketir: Task 1 (`REDACTED`, `Statement.redacted`), Task 2 (`settings`).
- Üretir: `role::{validate, create, alter, drop}` ve uygulayıcının kullanacağı tek somutlaştırma işlevi:

```rust
/// Gizlenmiş parola ifadesinin önizleme metni (tırnaksız `********`: kopyalanıp çalıştırılırsa sözdizimi hatası verir).
pub(crate) fn password_redacted(role: &str) -> Statement {
    Statement { sql: format!("ALTER ROLE {} PASSWORD {}", quote_ident(role), REDACTED), risk: Risk::Safe, transactional: true,
                note: Some("Parola gizlendi; uygulamada girilen değer kullanılır.".into()), redacted: true }
}
/// YALNIZCA uygulayıcı çağırır (kapılardan sonra, çalıştırma listesinde). Değer günlüğe/hataya yazılmaz.
pub(crate) fn password_sql(role: &str, secret: &Secret) -> String {
    format!("ALTER ROLE {} PASSWORD {}", quote_ident(role), quote_literal(secret.expose()))
}
/// Parola değerinin kabul kuralı (uygulama kapısı): boş değil, NUL yok.
pub(crate) fn check_secret(secret: &Secret) -> Result<(), &'static str>;
```

**Doğrulama:** `name` (`check_ident`; ayrıca `public`/`none` (tam eşleşme) ve `pg_` öneki → "Bu rol adı ayrılmıştır"), `connectionLimit` (`-1..=i32::MAX`), `validUntil` (dilbilgisi; alter'da `old` dolu ve `new == None` → "Süre sınırı kaldırılamaz; süresiz için `infinity` yazın"), `memberOf[i].role` (`check_ident`; boş; yinelenen; rolün kendisi → "Rol kendisine üye olamaz"), `settings`, `comment`, `password` (create'te `clear` → "Yeni rolde parola kaldırılamaz"). `memberOf[i].inheritOption/setOption` üretici tarafından **okunmaz**.

**Üretim (hepsi transactional):**
- Öznitelik metni (deterministik sıra, create'te tümü yazılır): `LOGIN|NOLOGIN SUPERUSER|NOSUPERUSER CREATEDB|NOCREATEDB CREATEROLE|NOCREATEROLE INHERIT|NOINHERIT REPLICATION|NOREPLICATION BYPASSRLS|NOBYPASSRLS CONNECTION LIMIT n [VALID UNTIL <lit>]`.
- create: `CREATE ROLE <q> WITH <öznitelikler>` + (`password == set` → `password_redacted(name)`) + üyelikler (rol adı sırasıyla) `GRANT <r> TO <q> [WITH ADMIN OPTION]` + ayarlar `ALTER ROLE <q> SET …` + `COMMENT ON ROLE`. Plan uyarıları: SUPERUSER/REPLICATION/BYPASSRLS/CREATEROLE true ise "<öznitelik> veriliyor: <etki>"; parola `set` ise "Parola sunucuya düz metin gider; log_statement ve pg_stat_statements açıksa sunucu günlüğüne düşebilir".
- alter: Global Constraints'teki Role sırası. `RENAME` notu + uyarı "MD5 ile saklanan parola ad değişince temizlenir (SCRAM etkilenmez)"; `ALTER ROLE … WITH` yalnızca değişen öznitelikleri yazar (CONNECTION LIMIT ve VALID UNTIL dahil); NOLOGIN'e geçiş notu "Mevcut oturumlar sürer; yeni girişler reddedilir"; `clear` → `ALTER ROLE <q> PASSWORD NULL`; üyelik eşlemesi **rol adıyla**: çıkarılan → `REVOKE <r> FROM <q>` (Destructive, not "Bu rolün yetkileri kaybolur"), admin true→false → `REVOKE ADMIN OPTION FOR <r> FROM <q>` (Destructive), eklenen → `GRANT`, admin false→true → `GRANT <r> TO <q> WITH ADMIN OPTION`; ayarlar RESET→SET; yorum.
- drop: `DROP ROLE <q>` Destructive, not "Rolün sahip olduğu nesne ya da yetki varsa başarısız olur (REASSIGN OWNED / DROP OWNED gerekir)".

- [ ] **Adım 1: Başarısız testler** (`insta` + açık):

```rust
#[test]
fn password_is_redacted_in_every_plan_and_hash_is_value_free() {
    let mut r = role("app"); r.password = PasswordChange::Set;
    let p = create(&r, &ctx());
    let red: Vec<_> = p.statements.iter().filter(|s| s.redacted).collect();
    assert_eq!(red.len(), 1);
    assert_eq!(red[0].sql, "ALTER ROLE app PASSWORD ********");
    assert!(!red[0].sql.contains('\''), "maske tırnaksız olmalı (kopyalanan betik çalışmasın)");
    // Aynı istek her zaman aynı hash (değer isteğin parçası değildir).
    assert_eq!(plan_hash(&p.statements), plan_hash(&create(&r, &ctx()).statements));
    // Somutlaştırma yalnızca tırnaklı değer üretir; tek ifade kalır.
    let sql = password_sql("app", &Secret::new("x'; DROP ROLE app; --"));
    assert_eq!(sql, "ALTER ROLE app PASSWORD 'x''; DROP ROLE app; --'");
    assert!(scan_semicolons(&sql).is_clean());
    assert_eq!(password_sql("Ünï", &Secret::new("a\\b")), "ALTER ROLE \"Ünï\" PASSWORD E'a\\\\b'");
    assert!(check_secret(&Secret::new("")).is_err() && check_secret(&Secret::new("a\0b")).is_err());
}
```

  Diğerleri: create öznitelik tablosu (her bayrak true/false), connection limit, valid until (`infinity`, `2030-01-01`, `2030-01-01T10:00:00Z`, `2030-01-01 10:00:00+03`, `2030-01-01T10:00:00.123456Z` kabul; `2030-13-01`, `tomorrow`, `2030-01-01'; --`, `now()`, boş ret), parola `set` (redacted, Safe, uyarı), `clear` create'te hata, alter'da `clear` → `PASSWORD NULL` (redacted değil), rename + parola sırası (gizlenmiş ifade **yeni** adı kullanır), üyelik ekle/çıkar/admin iki yön (risk sınıfları), üyelik sıralaması, yinelenen/kendine üyelik hatası, `pg_read_all_data` gibi önceden tanımlı rol üyeliği kabul, PG 16 `inheritOption/setOption` dolu yüklü üyeliğin admin değişiminde yalnız admin ifadesi üretilir (seçenekler yazılmaz), ayrılmış adlar (`public`, `none`, `pg_x` ret; `Public`, `pgx` kabul), `validUntil` temizleme hatası, birleşik sıra anlık görüntüsü (rename + WITH + parola + revoke + grant + ayar + yorum), `alter(x,x)` boş, drop (`cascade` → hata Task 1 kuralıyla), enjeksiyon matrisi (ad, üyelik rolü, valid until, ayar).
- [ ] **Adım 2–4:** Uygula; `cargo test -p pgt-ddl` + clippy temiz.
- [ ] **Adım 5: Commit** (`feat(ddl): Role üreticisi (öznitelikler, gizlenmiş parola, üyelikler, ayarlar)`).

---

### Task 6: View ve MatView yükleyicileri; gidiş-dönüş testleri (Docker)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/sql/load_view.sql` (+ `.pg10.sql`: `security_invoker` yok — reloptions'ta zaten bulunmaz, tek dosya yeterliyse varyant açılmaz), `load_matview.sql`, `load_relation_columns.sql`, `crates/pgt-ddl/tests/view_roundtrip.rs`
- Değiştir: `crates/pgt-ddl/src/load.rs`

**Arayüzler:**
- Üretir: `load(.., ObjectRef { kind: View | MaterializedView, oid })` → `ObjectDef::View/MaterializedView`.
- Yükleyici ayrıntıları (normalleştirme sözleşmesi):
  - Ortak: `pg_class` relkind `v` (View) / `m` (MatView) — tür uyuşmazlığı `NotFound`; sistem şeması → `Invalid(SYSTEM_ERROR)`; `relpersistence = 't'` → `Invalid("geçici görünüm düzenlenemez")`; sahip `Some`; yorum boşsa `None`.
  - `query` = `pg_get_viewdef(oid, true)` metni Rust'ta `trim()` + sondaki tek `;` + `trim_end()` (dış parantez **soyulmaz**; PG zaten sarmalamaz). **Sabit nokta sözleşmesi:** yüklü sorguyla yeniden oluşturulan görünümün yüklü sorgusu aynıdır (testle).
  - View seçenekleri `reloptions`'tan: `check_option=local|cascaded`, `security_barrier=true|on` (yalnız doğru değerler true), `security_invoker=true|on`; bilinmeyen anahtar yoktur.
  - `columns` = `pg_attribute` (`attnum > 0 AND NOT attisdropped`, attnum sırası; `format_type`).
  - View `unsupported` (ad sırasıyla): INSTEAD OF tetikleyicileri (`pg_trigger`, `NOT tgisinternal`) → `"TRIGGER <ad>"`; `_RETURN` dışındaki kurallar (`pg_rewrite`) → `"RULE <ad>"`; kolon varsayılanları (`pg_attrdef`) → `"DEFAULT <kolon>"`; kolon yorumları → `"COMMENT <kolon>"`.
  - MatView: `tablespace` (yalnızca `reltablespace = 0` → `None`; açık ad, varsayılanı başka olan veritabanında `pg_default` dahil, olduğu gibi — Task 6 inceleme m-1), `fillfactor` reloptions, diğer reloptions → `"WITH (k=v)"`, `with_data = relispopulated`, `refresh = None`, `unsupported` + `"INDEX <ad>"` (`pg_index.indrelid`, ad sırasıyla) + kolon yorumları.
- [ ] **Adım 1: Başarısız Docker testleri** (`tests/view_roundtrip.rs`; `common::setup` kalıbı, `PGT_TEST_URLS` yoksa atlanır; ifadeler `Env::exec_tx` ile çalıştırılır — manager Task 8'de):

```rust
async fn view_create_then_load_matches()          // seçeneksiz/check_option local+cascaded/security_barrier/security_invoker(PG15+; altında atlandığı yazdırılır);
                                                   // owner + yorum; columns dolu; unsupported boş; create(L1) ikinci görünümde sorgu sabit noktası
async fn view_unchanged_edit_is_empty_plan()       // her varyant için alter(load, load) == boş
async fn view_replace_keeps_grants_and_options_follow_new_def() // GRANT SELECT verilmiş görünümde sorgu (sona kolon ekle) + seçenek değişimi → load == new; GRANT duruyor; seçenek kaldırma (security_barrier true→false) C-O-R ile siliniyor
async fn view_incompatible_replace_fails_and_recreate_succeeds() // kolon silen sorgu → try_exec_tx Err (42P16 ya da 42804), görünüm değişmemiş; recreate=true planı uygulanınca load == new (columns farklı); GRANT kaybolmuş
async fn view_recreate_with_dependent_fails_cleanly() // v2 v1'e bağlı: v1 recreate → 2BP01, v1 ve v2 aynen duruyor
async fn view_rename_schema_owner_comment_combined()
async fn view_unsupported_lists_trigger_rule_default_comment() // INSTEAD OF trigger + kural + kolon DEFAULT + kolon COMMENT → unsupported 4 öğe; başka bir alter (yorum) sonrası hepsi duruyor
async fn view_body_with_backslash_literal_is_still_renamable()  // CREATE VIEW … AS SELECT E'a\\b' → yüklü sorgu 'a\b'; rename planı uygulanır; sorgu değişimi `query` alan hatası
async fn matview_create_with_and_without_data()   // relispopulated; fillfactor; tablespace pg_default → None; owner + yorum; sabit nokta
async fn matview_refresh_transitions()            // false→true, true→false, true→true normal; concurrently: UNIQUE indeks yokken 55000 hatası, varken başarı
async fn matview_redefine_drops_indexes_and_data() // indeksli MV sorgu değişimi: plan DataLoss + INDEX uyarısı; uygulama sonrası indeks yok, load == new (unsupported'tan INDEX kalkmış)
async fn matview_alter_tablespace_fillfactor_owner_comment_rename_schema()
async fn system_and_temp_views_are_refused()       // pg_catalog.pg_tables → Invalid; geçici görünüm → Invalid; tablo oid'iyle View yükleme → NotFound
```

- [ ] **Adım 2: Uygula** yükleyiciyi. Gidiş-dönüşün ortaya çıkardığı üretici hataları **bu görevde** regresyon birim testiyle düzeltilir (raporda listelenir).
- [ ] **Adım 3:** Docker'lı: tek PG 18 ile `cargo test -p pgt-ddl --test view_roundtrip`; sonra `scripts/test-matrix.ps1 -Versions 10,13,14,15,17,18 -SkipPerf` (PG 15 security_invoker sınırı; PG 10/13 `pg_get_viewdef` biçimi) — kapsayıcılar silinir.
- [ ] **Adım 4:** clippy temiz. **Commit** (`feat(ddl): View/MatView yükleyicileri ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 7: Database ve Role yükleyicileri, yeni açılır listeler; gidiş-dönüş testleri (Docker)

**Dosyalar:**
- Oluştur: `crates/pgt-ddl/src/sql/load_database.pg10.sql`, `load_database.pg15.sql` (`datlocprovider`, `daticulocale`), `load_database.sql` (PG 17+: `datlocale`), `load_role.sql`, `load_role_members.pg10.sql`, `load_role_members.sql` (PG 16+: `inherit_option`, `set_option`, grantor sayısı), `load_db_role_settings.sql`, `lookup_grantable_roles.sql`, `lookup_template_databases.sql`, `lookup_locales.pg10.sql`, `lookup_locales.pg15.sql`, `lookup_locales.sql` (PG 17+), `lookup_settings.sql`, `crates/pgt-ddl/tests/cluster_roundtrip.rs`
- Değiştir: `crates/pgt-ddl/src/load.rs`, `crates/pgt-ddl/src/lookup.rs`, `crates/pgt-ddl/tests/common/mod.rs` (küme nesnesi yardımcıları: `cluster_name(ts, kind, tag)`, `drop_test_database`, `drop_test_role`)

**Arayüzler:**
- Üretir: `load(.., ObjectRef { kind: Database | Role, oid })` (bu görevde `load` iç yardımcıları `Target`'i parametre alır: `load_database(mgr, server, Target::Maintenance, oid)`); `LookupRequest` yeni varyantlar ve kapsam:

```rust
pub enum LookupRequest {
    /* mevcutlar … */
    /// Üyelik için tüm roller (önceden tanımlı `pg_*` dahil): detail = "önceden tanımlı" | "superuser" | null.
    GrantableRoles,
    /// Sunucu kodlamaları (`SERVER_ENCODINGS`, sabit; sunucuya gidilmez).
    Encodings,
    /// Şablon olarak kullanılabilecek veritabanları (`datistemplate OR datallowconn`); detail = "şablon" | null.
    TemplateDatabases,
    /// Yerel ayar adları: libc → collcollate (+ C, POSIX), icu → ICU yerel ayarları, builtin (PG 17+) → yerleşik.
    Locales { provider: LocaleProvider },
    /// `ALTER … SET` ile ayarlanabilen parametreler (`pg_settings.context IN ('user','superuser')`): detail = "tip · kısa açıklama".
    Settings,
}
impl LookupRequest {
    /// `database_oid = None` ile (bakım veritabanında) çalışabilen istekler.
    pub fn server_wide(&self) -> bool;   // Roles, GrantableRoles, Tablespaces, Encodings, TemplateDatabases, Locales, Settings
}
```

- Yükleyici ayrıntıları:
  - **Database** (`pg_database` + `pg_tablespace` + `shobj_description` + `pg_db_role_setting (setrole = 0)`): `owner` `Some`; `template = None`; `encoding = pg_encoding_to_char(encoding)`; `lcCollate = datcollate`, `lcCtype = datctype`; `localeProvider` PG 15+ `Some(c→libc, i→icu, b→builtin)`, PG < 15 `None`; `providerLocale` PG 15–16 `daticulocale`, PG 17+ `datlocale` (libc'de `None`); `tablespace = Some(spcname)` (**her zaman** dolu); `connectionLimit`, `isTemplate`, `allowConnections`; `settings` = `setconfig` öğeleri `name=value` (ilk `=`'den bölünür), ada göre sıralı; yorum boşsa `None`.
  - **Role** (`pg_roles` + `pg_auth_members` + `pg_db_role_setting` + `shobj_description(…, 'pg_authid')`): `pg_` önekli rol → `Invalid("sistem rolü düzenlenemez")`; öznitelikler; `validUntil` SQL'de `CASE WHEN rolvaliduntil IS NULL THEN NULL WHEN NOT isfinite(rolvaliduntil) THEN rolvaliduntil::text ELSE to_char(rolvaliduntil AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS') || CASE WHEN extract(microseconds FROM rolvaliduntil)::bigint % 1000000 <> 0 THEN to_char(rolvaliduntil AT TIME ZONE 'UTC', '.US') ELSE '' END || 'Z' END` (oturum saat diliminden bağımsız; `.US` sondaki sıfırları korur — sabit biçim); `password = Keep`; `memberOf` rol adı sırasıyla (PG 16+: aynı rol için birden çok satır → `unsupported` "MEMBER <r> (birden çok grantor)"; tek satırda `inheritOption/setOption` dolu; PG < 16 `None`); `settings` `setdatabase = 0`; `setdatabase <> 0` satırları → `unsupported` "IN DATABASE <db>: <k=v>"; yorum.
- Normalleştirme sözleşmesi (testle): `alter(load, load)` boş; `create(def)` → `load` = `def` **şu farklarla**: Database `template → None`, create'te `None` bırakılan create-only alanlar sunucu değerini alır, `tablespace None → Some("pg_default")`; Role `password → keep`, PG 16+ üyelik seçenekleri dolar; ayar değerleri sunucu biçimine normalleşebilir (`search_path` öğeleri) — testler kurallı değerler kullanır.

- [ ] **Adım 1: Başarısız Docker testleri** (`tests/cluster_roundtrip.rs`; her test benzersiz `pgt_ddl_dbk_<v>_<etiket>` / `pgt_ddl_rk_<etiket>` adları; `guarded` temizliği; ifadeler bakım bağlantısında `admin_client(ts, maintenance_db)` ile — transaction dışı ifadeler tek tek):

```rust
async fn database_create_then_load_matches()       // owner, TEMPLATE template0, ENCODING UTF8, LC_COLLATE/LC_CTYPE 'C', connlimit 7, allow_connections, is_template false,
                                                   // ayarlar (work_mem=64MB, search_path="$user", public), yorum → load (normalleştirme ile) eşit
async fn database_icu_and_builtin_providers()      // PG15+: icu (ICU collation yoksa atlandığı yazdırılır); PG17+: builtin C.UTF-8; altında atlama yazdırılır
async fn database_alter_round_trips()              // rename, owner, WITH birleşimi, is_template true→false, ayar SET/RESET/değişim, yorum; her biri load == new
async fn database_unchanged_edit_is_empty_plan()
async fn database_rename_blocked_by_session_reports_55006() // başka oturum bağlıyken RENAME → 55006, ad aynı
async fn role_create_then_load_matches()           // tüm öznitelik kombinasyonları (superuser dahil; test bağlantısı süper kullanıcı), connlimit, validUntil (UTC + kesirli + infinity),
                                                   // üyelikler (admin true/false, pg_read_all_stats), ayarlar, yorum
async fn role_alter_round_trips()                  // her öznitelik, rename, üyelik ekle/çıkar/admin iki yön, ayar, yorum, validUntil değişimi
async fn role_unchanged_edit_is_empty_plan()       // PG 16+ üyelik seçenekleri dolu iken de boş
async fn role_pg16_grant_options_are_read_only_and_preserved() // PG16+: GRANT r TO m WITH INHERIT FALSE, SET FALSE → inheritOption/setOption false; admin değişimi sonrası seçenekler aynı
async fn role_unsupported_settings_and_multi_grantor() // ALTER ROLE IN DATABASE … SET → unsupported; başka alter sonrası duruyor; PG16+ iki grantor → MEMBER … unsupported
async fn role_drop_with_owned_objects_fails_cleanly()  // rol tablo sahibi: DROP ROLE → 2BP01 (+detay), rol duruyor
async fn system_role_is_refused()                  // pg_monitor oid → Invalid
async fn new_lookups_return_expected_shapes()      // GrantableRoles (pg_* dahil, detail), Encodings (UTF8 var, SJIS yok), TemplateDatabases (template1 var, template0 "şablon"),
                                                   // Locales{libc} (C ve POSIX dahil), Locales{icu} (PG15+ boş değil, ICU varsa), Locales{builtin} (PG17+), Settings (work_mem var, shared_buffers yok)
```

- [ ] **Adım 2: Uygula** yükleyiciler ve lookup'lar (`Encodings` sunucuya gitmez). Üretici hataları bu görevde regresyon testiyle düzeltilir.
- [ ] **Adım 3:** Docker'lı: tek PG 18 ile `cargo test -p pgt-ddl --test cluster_roundtrip`; sonra `scripts/test-matrix.ps1 -Versions 10,13,15,16,17,18 -SkipPerf` (PG 15 ICU sınırı, PG 16 üyelik seçenekleri, PG 17 `datlocale`/builtin) — kapsayıcılar silinir.
- [ ] **Adım 4:** clippy temiz. **Commit** (`feat(ddl): Database/Role yükleyicileri, sunucu genelinde açılır listeler ve PG 10–18 gidiş-dönüş testleri`).

---

### Task 8: Yönetici, uygulayıcı, hedef ve komut entegrasyonu (kapsam, parola kapısı, bakım veritabanı)

**Dosyalar:**
- Değiştir: `crates/pgt-core/src/connection.rs` (+ birim testi), `crates/pgt-ddl/src/manager.rs`, `apply.rs`, `load.rs` (genel `load` imzası), `lookup.rs` (kapsam), `target.rs`, `lib.rs`, `src-tauri/src/ddl.rs`
- Test: `crates/pgt-ddl/tests/manager.rs`, `tests/logging.rs`, `tests/json.rs`, `target.rs` birim testleri, `manager.rs` iç Docker testleri (`open_apply_conn` yeni imzası)

**Arayüzler:**
- Üretir (`pgt-core`):

```rust
impl ConnectionManager {
    /// Havuzdaki `Target::Database(oid)` bağlantısını kapatır ve ad önbelleğinden siler (oturum bağlı kalır).
    /// Veritabanı yeniden adlandırma/silme/taşıma öncesi ve sonrası çağrılır; bağlı değilse sessizce döner.
    pub async fn release_database(&self, server: Uuid, oid: u32);
}
```

- Üretir (`pgt-ddl`):
  - `DdlManager::{preview, apply, load, lookup, defaults}` `database_oid: impl Into<Option<u32>> + Send` alır (mevcut `u32` çağrıları derlenmeye devam eder). İç çözüm: `fn target_for(kind: ObjectKind, db: Option<u32>) -> Result<Target>` (sunucu kapsamlı → `Maintenance`; aksi `Database(oid)` ya da `Invalid(NEEDS_DATABASE)`); lookup için `server_wide()` kuralı. `load::load(mgr, server, database_oid: impl Into<Option<u32>>, obj)` aynı kuralla.
  - `apply::open_apply_conn(mgr, server, target: Target, lock_timeout_ms)`.
  - `Guard`'a `maintenance_db: String`.
  - **Kapı sırası (genişler; yalnız (2a), (9), (10) yeni):** (1) kayıtsız, (2) kilit, (2a) hedef çözümü (`target_for`), (3) plan (geçersiz/boş), (4) nesne denetimi (yükleme `target_for` ile), (4a) **bakım veritabanı kapısı**: Database alter'da ad ya da tablespace değişiyorsa veya Database drop'ta, yüklenen ad `guard.maintenance_db` ise `Invalid(MAINTENANCE_DATABASE)`, (5) hash, (6) onay, (7) üretim, (8) lock_timeout, (9) **parola**: planda `redacted` ifade var ve `opts.password` yok → `Invalid(ROLE_PASSWORD_MISSING)`; parola var ama `redacted` yok → `Invalid(PASSWORD_NOT_USED)`; `check_secret` başarısız → `Invalid("Parola boş olamaz ve NUL içeremez.")`, (10) **somutlaştırma**: `fn materialize(statements: &[Statement], req: &PlanRequest, secret: Option<&Secret>) -> Vec<Statement>` — `redacted` ifadenin `sql`'i `role::password_sql(<istekteki yeni RoleDef.name>, secret)` olur (yalnızca bu yerel kopyada; `PlanView`'e dönmez); çalıştırma sonrası somutlaşmış ifade metni `Drop`'ta sıfırlanan sarmalayıcıda tutulur (en iyi çaba).
  - **Database özel çalıştırma:** Database alter (ad/tablespace değişimi) ve drop için `execute` öncesi ve **sonrası** `mgr.release_database(server, object.oid)`; Database create/alter/drop ve Role için bağlantı `Target::Maintenance`.
  - **Hata yükü ipuçları** (yalnız PG ipucu yoksa; SQL/değer içermez): `failure_payload_for(kind, e)` — 55006 + Database → Karar 8 metni; 42501 (her tür) → Karar 9 metni; 2BP01 + Role → "Rolün sahip olduğu nesneler/yetkiler var: önce REASSIGN OWNED / DROP OWNED (her veritabanında) çalıştırın."; 2BP01 + View/MatView → "Bağımlı nesneler var; silmek için CASCADE seçin ya da bağımlıları kaldırın."; 42P16/42804 + View + mesaj "view column" içeriyor → Karar 1 metni. Başarısız ifade `redacted` ise `position = None`.
  - `find_created`: View (`relkind = 'v'`), MatView (`'m'`), Database (`pg_database.datname`, `Maintenance`), Role (`pg_roles.rolname`, `Maintenance`).
  - `identity_changed`: View `(schema,name)` farkı **veya** (`new.recreate` ve sorgu farklı); MatView `(schema,name)` farkı **veya** sorgu farklı; Database/Role ad farkı.
  - `defaults`: View/MatView (`schema_oid` zorunlu, `table_oid` yasak): `ViewDef { query: "", check_option: None, security_barrier: false, security_invoker: false, … }`, `MatViewDef { with_data: true, … }`; Database (bağlam boş olmalı): `connection_limit: -1, allow_connections: true, is_template: false`, diğerleri `None`/boş; Role (bağlam boş): `login: false, inherit: true`, diğer bayraklar `false`, `connection_limit: -1`, `password: Keep`.
  - `target_of` (`target.rs`): `[Coll(Databases)]` → `create: Database`; `[Db(o)]` → `object: Database(o)`; `[Coll(Roles)]` → `create: Role`; `[Role(o)]` → `object: Role(o)`; `[.., Schema(s), Coll(Views)]` → `create: View{schema_oid: s}`; `[.., Schema(_), View(o)]` → `object: View(o)`; `MatViews`/`MatView(o)` aynı kalıp; **sunucu kapsamlı sonuçlarda `database_oid = None`** (yol `Db` içerse bile) ve yalnızca yol uzunluğu 1 iken; veritabanı kapsamlı sonuçlarda mevcut "veritabanı yoksa boş" kuralı. `refreshNodeId`: Database → `srv/dbs`, Role → `srv/roles`, View → `…/sch:S/views`, MatView → `…/sch:S/matviews`, create → koleksiyonun kendisi. Görünüm/MV kolon düğümleri, `tablespaces`/`ts:N`, `db:5/roles`, `db:5/views`, `sch:1/views` (veritabanısız) **boş**.
- Üretir (`src-tauri/src/ddl.rs`): `ddl_defaults/ddl_load/ddl_lookup/ddl_preview/ddl_apply` `database_oid: Option<u32>`.

- [ ] **Adım 1: Başarısız testler.**
  - `target.rs`: yeni düğümler tablosu (`dbs`, `db:5`, `roles`, `role:10`, `db:5/sch:2200/views`, `db:5/sch:2200/view:30`, `db:5/sch:2200/matviews`, `db:5/sch:2200/mv:31`) → beklenen `object/create/databaseOid/refreshNodeId`; `db:5` için `databaseOid == None`; `other_nodes_are_empty` listesinden bu düğümler çıkarılır, yeni negatifler eklenir (`db:5/roles`, `db:5/role:10`, `db:5/views`, `db:5/view:30`, `sch:2200/views`, `db:5/sch:2200/tbl:1/views`); `refresh_node_is_the_collection` yeni satırlar; JSON (`databaseOid: null`).
  - `pgt-core` birim/Docker: `release_database` havuzdaki o hedefi ve ad önbelleğini siler; bağlı değilken panik yok.
  - `manager.rs` saf: `target_for` tablosu; `materialize` (gizlenmiş ifade gerçek değerle, diğerleri aynen; `PlanView` kopyası değişmez); `failure_payload_for` ipucu tablosu (PG ipucu varsa korunur); `identity_changed` yeni kollar.
  - `tests/manager.rs` (Docker; `PGT_TEST_URLS` yoksa atlanır):

```rust
async fn view_lifecycle_through_manager()          // create → Applied.created → load eşit; C-O-R alter; uyumsuz C-O-R → Failed + Karar 1 ipucu, değişiklik yok; recreate → DataLoss: confirmed=false → confirmation_required, üretim etiketinde production_confirmation_required, onaylarla Applied; drop Destructive onaysız red
async fn matview_refresh_needs_confirmation()      // refresh Normal → confirmation_required; onaylı Applied
async fn database_lifecycle_on_maintenance()       // database_oid = Some(başka db) verilse de CREATE bakım veritabanında; Applied.created (pg_database oid); load eşit;
                                                   // bu yöneticinin havuzu hedef db'ye bağlıyken (lookup Schemas çalıştırılmış) RENAME başarılı (release_database);
                                                   // ikinci bir ayrılmış bağlantı açıkken DROP → Failed 55006 + ipucu; PG13+ FORCE ile Applied ve öteki bağlantı kopmuş; drop DataLoss üretim kapısı
async fn maintenance_database_is_protected()       // bakım veritabanını rename/drop/set tablespace → Invalid(MAINTENANCE_DATABASE), bağlantı açılmadan; yorum/ayar değişimi serbest
async fn database_partial_apply_reports_committed() // rename + var olmayan owner: Failed{failed_index:1, committed:1}; ad değişmiş (transaction dışı)
async fn role_lifecycle_and_password()             // create (password=set) parolasız → role_password_missing; parola ile Applied; ConnectionManager::test(cfg{user: rol}, Some(parola)) başarılı, yanlış parola başarısız;
                                                   // password=keep planına parola → PASSWORD_NOT_USED; boş parola → invalid; alter clear → giriş başarısız; REVOKE → confirmation_required
async fn role_insufficient_privilege_is_reported()  // CREATEROLE'lü, süper kullanıcı olmayan test rolüyle (rastgele parola, ayrı ServerConfig) SUPERUSER rol oluştur → Failed 42501 + Karar 9 ipucu
async fn tampered_requests_are_refused()           // Role alter old.password=set → plan_changed; bayat old (başka oturum COMMENT) → plan_changed; Database drop def.name ≠ yüklenen → plan_changed;
                                                   // object.kind=Role + def Database → invalid_def; view hash oynanmış → plan_changed; View için database_oid=None → invalid (NEEDS_DATABASE)
async fn locked_server_refuses_all_new_kinds()     // dört tür için apply → locked; preview/load/lookup serbest
```

  - `manager.rs` iç Docker testi (`mod docker`, crate içi `apply_statements` kalıbı; yardımcı `apply_statements` gizlenmiş ifadeyi `materialize` ile somutlaştıracak şekilde `secret: Option<&Secret>` parametresi alır): `redacted_failure_hides_position` — `[password_redacted("pgt_yok_rol")]` + parola → var olmayan rol 42704 ile `Failed`; `position == None`, `error.message`/`detail`/`hint` parolayı içermez.

  - `tests/logging.rs`: mevcut teste ek gövde — rol `create` + `password = set` + parola `"gizli-parola-Q7"` ile uygulama, ardından aynı adla ikinci uygulama (42710 Failed); yakalanan günlükte, `PlanView` JSON'unda (`serde_json::to_string`), `ApplyOutcome` JSON'unda ve `format!("{:?}", opts)` çıktısında `"gizli-parola-Q7"` **yok**; ön izleme hash'i parola değerinden bağımsız (iki farklı parolayla aynı `expected_hash` kabul edilir).
  - `tests/json.rs`: `PlanRequest` create/alter/drop × view/materialized_view/database/role JSON şekilleri; `ApplyOptions.password` girişte dizge, çıkışta `"********"`; `LookupRequest` yeni varyantlar (`{"kind":"locales","provider":"icu"}`).
- [ ] **Adım 2: Uygula.** `src-tauri` komut imzaları; Tauri testinde mevcut çağrılar `Some(db)`.
- [ ] **Adım 3:** Docker'lı `cargo test -p pgt-ddl` (PG 18) + `scripts/test-matrix.ps1 -Versions 10,13,16,18 -SkipPerf`; `cargo test --workspace` (Docker'sız), `cargo test -p pg-tools`, `cargo clippy --workspace --all-targets -- -D warnings`; `bun run typecheck` (binding değişimleri — `DdlTarget` şekli aynı, `LookupRequest` genişledi) ve `bun run test` yeşil; ana `index-*.js` 233,088 B.
- [ ] **Adım 4: Commit** (`feat(ddl): sunucu kapsamlı türler, parola kapısı ve somutlaştırma, bakım veritabanı koruması, target_of yeni düğümler`).

---

### Task 9: Saldırgan yeniden inceleme, belgeler ve bütünleşik doğrulama

**Dosyalar:**
- Değiştir: `docs/superpowers/specs/2026-10-06-object-dialogs-design.md`, `docs/smoke-test.md`, gerekirse inceleme bulgularının düzeltildiği kaynak/test dosyaları

**Arayüzler:**
- Tüketir: Task 1–8. Üretir: belgeler + (varsa) düzeltmeler.

- [ ] **Adım 1: Saldırgan yeniden inceleme (final incelemeden ÖNCE, ayrı görev).** Odak: (a) view/MV `query` kapıları ve `CREATE OR REPLACE`/`CREATE MATERIALIZED VIEW … WITH DATA` bağlamında sarma; (b) GUC adı/değeri, `split_guc_list` (tırnak/virgül/boşluk köşe durumları), encoding/locale/valid-until değerleri; (c) parola yolunun tamamı (`Secret` → `materialize` → çalıştırma → hata yükü → günlük → `Debug`/`Serialize`; `grep -rn "expose()" crates/pgt-ddl/src` yalnızca `role::password_sql` ve `check_secret`); (d) sunucu kapsamlı hedef çözümünün atlatılamaması (Database isteğinin `Target::Database` ile çalıştırılamaması); (e) bakım veritabanı kapısı. Her bulgu için regresyon testi + düzeltme; bulgu yoksa gerekçeli "temiz" notu ledger'a.
- [ ] **Adım 2: Spec.** Durum satırı: "Plan 10–14 (View/MatView, Database, Role arka ucu dahil) uygulandı; Plan 15 (bu türlerin arayüzü) kalan". §3 tablosu: `DROP DATABASE` DataLoss satırına taşınır; "Düzenlemede yeniden kurma (view `recreate`, MV tanım değişikliği) DataLoss" satırı eklenir; `REVOKE` (rol üyeliği) Destructive. §1.3: "Rol üyeliklerinde PG 16+ seçenekleri yalnızca salt okunur gösterilir". §5 View satırına "kolon uyumsuzluğunda kullanıcı açıkça 'Yeniden oluştur' seçer (Karar 1)". "Sözleşme notları"na **"View/MatView/Database/Role arka ucu (Plan 14) sözleşme notları (arayüz Plan 15 için)"** paragrafı: aşağıdaki "Plan 15 için notlar"ın tamamı. Planlama notuna "Plan 15 (View/MatView, Database, Role arayüzü)".
- [ ] **Adım 3: `docs/smoke-test.md`.** "## Nesne diyalogları: View/MatView/Database/Role arka ucu (Plan 14)" bölümü: Docker doğrulama komutu (`scripts/test-matrix.ps1 -SkipPerf`), "GUI maddeleri Plan 15 bölümündedir", bilinen sınırlar (görünüm sorgusunda `\` içeren sıradan dize; MV indeksleri yeniden kurmada kaybolur; parola sunucu günlüğüne düşebilir).
- [ ] **Adım 4: Bütünleşik doğrulama.** `cargo test --workspace` (Docker'sız), Docker matrisi **tüm sürümler** `scripts/test-matrix.ps1 -SkipPerf` (10–18) yeşil; `cargo clippy --workspace --all-targets -- -D warnings`; `bun run typecheck`, `bun run test`, `bun run build` (ana `index-*.js` = 233,088 B; `index.html` önyüklemeleri yalnızca solid/web/Icon); `rg "tracing::" crates/pgt-ddl/src` çağrıları SQL/ad/değer/parola içermez; `rg "format!" crates/pgt-ddl/src/build/{view,matview,database,role,settings}.rs` her kullanım `quote_*` çıktısı, doğrulanmış GUC adı parçası, `wrap()` sonucu ya da sabit anahtar sözcük birleştirir (incelemede tek tek); `rg "expose\(\)" crates/pgt-ddl/src` yalnızca iki yer.
- [ ] **Adım 5: Commit** (`docs(ddl): Plan 14 sözleşme notları, spec risk tablosu ve smoke notları; saldırgan inceleme düzeltmeleri`).

---

## Kapanış kontrolü (Plan 14 bitişi)

- Docker matrisi PG 10–18 yeşil (view/MV ve küme gidiş-dönüşleri, yönetici kapıları, gizlilik testi); kapsayıcılar silinir; başka kapsayıcılara dokunulmamıştır.
- Ana paket bayt sayısı değişmedi (233,088 B); menüde yeni tür yok (Plan 15).
- Final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**; Plan 15 birleşmeden sonra başlar.

## Plan 15 için notlar (sözleşme)

- **Kapsam:** Database ve Role sunucu kapsamlıdır: `DdlTarget.databaseOid` bu türlerde `null`; arayüz `ddl_*` çağrılarında `databaseOid: null` gönderir (komutlar `Option<u32>`); Query Tool'da aç → bakım veritabanı. Veritabanı kapsamlı türlerde `null` → `invalid`.
- **Hedefler:** `dbs` → create Database; `db:N` → object Database; `roles` → create Role; `role:N` → object Role; `…/views`, `…/view:N`, `…/matviews`, `…/mv:N` aynı kalıp; `refreshNodeId` koleksiyon. Görünüm/MV kolon düğümleri menü almaz. Sistem rolleri (`pg_*`) arka uçta `invalid`; menü bunları göstermemeli (`flags.system`).
- **Parola:** `RoleDef.password` `"keep" | "set" | "clear"`; değer yalnızca `ApplyOptions.password` (`string | null`) — önizleme isteğine asla girmez; planda `redacted: true` ifade varken parola yoksa `role_password_missing` kodu; `redacted` ifade içeren plan Query Tool'a aktarılmamalı (metin bilerek çalıştırılamaz). Yükleme her zaman `"keep"`; eski tarafta `"keep"` dışı → `plan_changed`.
- **Drop:** `PlanRequest.drop.force` yalnızca Database (PG 13+); Database/Role için `cascade = true` alan hatası. `DROP DATABASE` DataLoss (üretimde ad yazma); `DROP ROLE` Destructive; görünüm/MV drop Destructive, CASCADE DataLoss.
- **FieldError yolları:** View `schema|name|owner|query|checkOption|securityBarrier|securityInvoker|comment|recreate|old.*`; MatView `schema|name|owner|query|tablespace|fillfactor|withData|refresh|comment`; Database `name|owner|template|encoding|localeProvider|lcCollate|lcCtype|providerLocale|tablespace|connectionLimit|isTemplate|allowConnections|settings|settings[i].name|settings[i].value|comment|force|cascade`; Role `name|login|…|connectionLimit|validUntil|password|memberOf|memberOf[i].role|settings|settings[i].name|settings[i].value|comment|cascade`.
- **Yükleme normalleştirmesi:** view/MV `query` `pg_get_viewdef` metnidir (biçim sunucunundur); `columns`/`unsupported`/`inheritOption`/`setOption` salt okunurdur (istemci değiştirmez); Database `template = null`, create-only alanlar dolu ve alter'da kilitli; `tablespace` her zaman dolu; Role `validUntil` UTC `…Z` biçimi, temizlenemez (`infinity`); ayarlar ada göre sıralı.
- **Alter-only bayraklar:** `ViewDef.recreate` (yalnızca sorgu değişmişken anlamlı; DataLoss), `MatViewDef.refresh` (`withData` tablosuna uyar; Destructive). Yüklemede `false`/`null`; uygulama sonrası yeniden yüklemede sıfırlanır.
- **Yeni lookup'lar:** `grantable_roles`, `encodings`, `template_databases`, `locales{provider}`, `settings` (hepsi `databaseOid: null` ile çalışır).

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §5 View (şema/ad/sahip/sorgu/seçenekler/yeniden adlandırma/C-O-R ve yükseltme → Task 2, 6, 8), MatView (WITH [NO] DATA, tablespace, REFRESH → Task 3, 6), Database (ad/sahip/template/encoding/locale + PG 15 ICU/tablespace/limit/is_template/allow_connections; rename uyarısı; FORCE PG 13+; tümü transaction dışı; Dashboard ipucu → Task 4, 7, 8), Role (öznitelikler, limit, VALID UNTIL, yalnız yazılır parola + maskeleme + günlük dışı, üyelikler, log_statement uyarısı → Task 5, 7, 8; arayüz uyarısı Plan 15); §3 risk modeli (Global Constraints + Karar 2–3); §6 (tek serbest SQL alanı, değer yerleştirmeleri, parola gizliliği → Task 2–5, 8, 9); §7 (birim + insta + PG 10–18 gidiş-dönüş + 42501 + parola günlük testi → Task 2–8).
- **Bilinçli kararlar ve spec çatışmaları:** Karar 1 (açık `recreate`), Karar 3 (`DROP DATABASE` DataLoss), Karar 4 (PG 16 seçenekleri salt okunur — spec'e uyuldu, görev metninden sapıldı), Karar 5 (rolde sahip yok), Karar 6 (parola tasarımı; `********` tırnaksız), Karar 10 (REFRESH kapsamda), Karar 12 (backlog). Bu kararlar Task 9'da spec'e işlenir.
- **Yer tutucu taraması:** Task 1'deki "henüz desteklenmiyor" kolları Task 2–5'te kaldırılır (açıkça belirtilmiş ara durum); "TBD/TODO" yok.
- **Tip tutarlılığı:** `DropOptions`/`drop_with` Task 1'de tanımlanır, Task 4–5 kullanır; `Secret` Task 1, `password_sql`/`check_secret` Task 5, `materialize` Task 8; `stmt_nt` Task 1, Task 4; `LookupRequest::server_wide` Task 7, Task 8 kapsam kuralı; `ObjectKind::server_scoped` Task 1, Task 8 `target_for`/`target_of`; ts bindingleri Task 1'de üretilir (`DdlErrorCode.ts` dahil), Task 7 yalnızca `LookupRequest.ts`'i genişletir.
- **Boyut:** 9 görev; ön yüz ayrı plan (Plan 15, 7 görev) — iki dosyanın başlıkları bağlantıyı belirtir.
