# Yedekleme / Geri Yükleme — Arka Uç (Plan 26) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Yeni `crates/pgt-backup` crate'i. Kapsadıkları:
- `pg_dump` / `pg_restore` / `psql` araçlarının bulunması ve sürüm seçimi,
- saf argüman/ortam üreticisi ve kural doğrulaması,
- format algılama ve arşiv özeti,
- süreç yürütücüsü (stderr akışı, ilerleme, atomik çıktı, iptal),
- Tauri komutları, ayar (`pgBinDir`) ve Docker testleri.

**Ön yüz bu planda değil** (Plan 27).

**Mimari:**
- Saf katman (`tools`, `args`, `detect`, `lines`) dış süreç ve dosya sistemine dokunmaz; tamamen birim testlidir.
- Yürütücü (`runner`) `tokio::process` ile çocuk süreci başlatır; ortamı açıkça kurar (miras `PG*` temizlenir, parola `PGPASSWORD`).
- Olaylar `EventSink` arayüzüyle iletilir; Tauri tarafında `Channel<BackupEvent>`.
- `JobRegistry` iş kimliğine göre iptal eder; `cancel_all` uygulama kapanışına ve sayfa yenilemeye bağlanır.
- Kapılar Tauri komutunda uygulanır:
  - kilit (geri yükleme),
  - onay ve üretim onayı (risk sınıfına göre),
  - parola bulunamazsa `password_required`.

**Teknoloji:** Rust 2024, tokio (+ `process`, `io-util`, `fs` özellikleri), ts-rs, Tauri 2 Channel.

**Spec:** `docs/superpowers/specs/2026-10-18-backup-restore-design.md` (§1 kararlar — bağlayıcı, §2 model, §4 test).

## Global Constraints

- **Gizlilik (en önemli):**
  - Parola yalnız çocuk süreç ortamına `PGPASSWORD` olarak girer; argümana, önizlemeye, günlüğe, hata iletisine, olaya ya da diske hiç yazılmaz.
  - Komut önizlemesi parolasızdır.
  - `tracing`'e yalnız iş türü, süre, çıkış kodu ve sonuç yazılır. Argüman, yol, nesne adı ve stderr metni yazılmaz.
  - Testler bunu yakalama ile doğrular.
- **Güvenli yürütme:**
  - Kabuk yok: `Command::new(exe).args([...])`.
  - Çalıştırılabilir dosya yalnız bulunan araç kümesinden (tam yol) seçilir; kullanıcı girdisi çalıştırılabilir ad olarak kullanılmaz.
  - Nesne adları ve yollar ayrı argüman olarak verilir: `-n`/`-t` değerleri tırnaklı biçimde (`"şema"."ad"`; `pg_dump` desen sözdizimi, `*`/`?`/`.` kaçışlı) üretilir.
  - Windows'ta `CREATE_NO_WINDOW` bayrağıyla konsol penceresi açılmaz.
- **Kurallar:** spec §1 aynen. Dışlamalar doğrulama hatası verir (alan yollarıyla):
  - yalnız şema ⟂ yalnız veri;
  - `--inserts` ⟂ `--column-inserts`;
  - `-j` yalnız directory (yedek) ve `-j` ⟂ `-1` (geri yükleme);
  - `-Z` tar'da yok;
  - plain'e özgü seçenekler diğer formatlarda yok;
  - psql'de temizle ve oluştur yok;
  - `-C` geri yüklemede hedef bakım veritabanıdır.
- **Atomik çıktı:** `<hedef>.pgt-partial` (dosya ya da klasör) → başarıda `rename`; başarısızlık ya da iptalde silinir. Hedef klasör (directory formatı) boş değilse doğrulama hatası verilir.
- **Sınırlar:**
  - Satır günlüğü en çok 2000 satır; satır başına 4 KiB, UTF-8 sınırında kesilir, sonuna `…` eklenir.
  - Taşma son satırla bildirilir.
  - `--version` zaman aşımı 5 sn.
- **Risk:** spec §1. Yedekleme Safe. Geri yükleme: `--clean`/`--create`/plain DataLoss, diğerleri Destructive. Geri yükleme kilitli sunucuda reddedilir.
- **Docker ve araç testleri:**
  - Testler `PGT_TEST_URLS` (mevcut) ve araç klasörü ister: `PGT_TEST_PGBIN` ya da otomatik bulma (`C:\Program Files\PostgreSQL\18\bin` bu makinede var). Araç yoksa test atlanır ve bunu yazar.
  - Matris yalnızca `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` ile çalıştırılır. Kapsayıcılar geçicidir ve silinir; başka kapsayıcılara dokunulmaz. URL/parola yazdırılmaz.
  - Yedek dosyaları test geçici klasörüne yazılır ve test sonunda silinir.
- **Ayar:** `AppSettings` `Copy` türetmesini kaybeder (`Option<String>` alanı). Kullanım yerleri `.clone()`/yapı güncellemesiyle uyarlanır; mevcut ayar testleri geçer. Eski `settings.json` (alan yok) varsayılanla okunur.
- **Genel:**
  - Bindingler commit edilir; satır sonu gürültüsü `git checkout -- src/ipc/bindings` ile atılır.
  - Ön yüze dokunulmaz (`commands.ts` dahil). Yalnız TS derleme uyumu gerekirse tembel dosyalarda düzeltilir.
  - `bun run bundle:check` geçer.
  - Yorumlar Türkçe. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - Crate genelinde `cargo fmt` çalıştırılmaz.

## Review Focus

- **Parola sızıntısı:**
  - `Debug`/`Serialize` türetmelerinde parola alanı yok; parola yalnız yürütücüye `Secret` benzeri bir sarmalayıcıyla geçer.
  - Hata yolları (spawn hatası, çıkış kodu) parola içermez.
  - Ortam önizlemesinde `PGPASSWORD=***`.
- **Miras ortam:** kullanıcının makinesindeki `PGPASSWORD`/`PGSERVICE`/`PGPASSFILE`/`PGOPTIONS` gibi değişkenler çocuğa sızmaz ve bağlantıyı değiştirmez.
- **İptal ve temizlik:**
  - İptal çocuğu sonlandırır, geçici çıktı silinir ve `Finished{cancelled}` gelir.
  - Uygulama kapanınca çocuk süreç kalmaz.
  - Çocuk kendiliğinden ölürse (çıkış kodu ≠ 0) geçici dosya da silinir.
  - Aynı hedefe iki iş yazmaz: aynı `outputPath` için çalışan iş varsa reddedilir.
- **Sürüm seçimi:**
  - Sunucu 16, araçlar 15 ve 18 → 18 seçilir.
  - Sunucu 18, araçlar yalnız 16 → net hata.
  - Aynı ana sürüm tercih edilir.
  - `PATH`'te ve Program Files'ta aynı klasör iki kez sayılmaz.
- **Format algılama ve uyumsuz istek:** custom dosyayı psql'e vermek ya da plain dosyayı pg_restore'a vermek mümkün değildir; algılama yolu belirler. Kullanıcı formatı zorlayamaz.

---

### Task 1: Crate iskeleti, model, araç bulma ve `pgBinDir` ayarı

**Dosyalar:**
- Oluştur: `crates/pgt-backup/{Cargo.toml, src/lib.rs, src/model.rs, src/tools.rs}`.
- Değiştir: kök `Cargo.toml` (members + workspace dep; tokio özellikleri `process`, `io-util`, `fs`), `crates/pgt-core/src/store.rs` (`AppSettings.pg_bin_dir`), `src-tauri/src/store_cmds.rs` (`settings_set_pg_bin_dir`), `src-tauri/src/main.rs` (kayıt), testler.

**Arayüzler — Üretir:**

```rust
pub struct ToolVersion { pub major: u32, pub version_num: u32, pub text: String }        // "18.1" → 180001, "17beta1" → 170000
pub fn parse_version(stdout: &str) -> Option<ToolVersion>;                                // "pg_dump (PostgreSQL) 16.4" vb.
pub struct ToolSet { pub dir: String, pub version: ToolVersion, pub pg_dump: bool, pub pg_restore: bool, pub psql: bool }   // TS: camelCase
pub struct ToolDetection { pub sets: Vec<ToolSet>, pub problem: Option<String> }          // TS
pub fn candidate_dirs(setting: Option<&str>, path_var: Option<&str>, program_files: &[String], list_dir: &dyn Fn(&str) -> Vec<String>) -> Vec<String>;  // saf; tekrar eler (büyük/küçük harf duyarsız, sondaki \ normalize)
pub async fn detect(setting: Option<&str>) -> ToolDetection;                               // gerçek fs + `--version` (5 sn)
pub enum ToolKind { PgDump, PgRestore, Psql }
pub fn choose(sets: &[ToolSet], kind: ToolKind, server_version_num: u32) -> Result<&ToolSet, String>;   // spec §1 seçim kuralı; hata metni sürümleri içerir
```
- **`AppSettings`:**
  - Yeni alan: `#[serde(default)] pub pg_bin_dir: Option<String>`.
  - `Copy` türetmesi kaldırılır ve kullanım yerleri uyarlanır.
  - Ayar komutu: `settings_set_pg_bin_dir(dir: Option<String>) -> AppSettings`. `Some` için klasör var mı ve `pg_dump.exe` içeriyor mu doğrulanır, değilse `Invalid` döner.
- **Adım 1: Başarısız testler.**
  - `parse_version` örnekleri: 10.23, 16.4, 18devel, 17beta1, 9.6.24 ve bozuk girdi.
  - `candidate_dirs`: sıra, tekrar eleme ve sahte `list_dir`.
  - `choose` tablosu (Review Focus).
  - Eski `settings.json` (alan yok) okunur.
  - `pg_bin_dir` gidiş-dönüşü.
  - Gerçek `detect` testi: bu makinede `C:\Program Files\PostgreSQL\18\bin` varsa 18 bulunur, yoksa test atlanır.
- **Adım 2–4:** uygula; `cargo test --workspace`, clippy, bindingler (`ToolSet`, `ToolDetection`, `AppSettings`), `bun run typecheck`.
- **Adım 5: Commit** (`feat(backup): pgt-backup crate'i, araç bulma ve pgBinDir ayarı`).

### Task 2: Saf üretici — argümanlar, ortam, kurallar, format algılama, satırlar

**Dosyalar:** Oluştur `crates/pgt-backup/src/{args.rs, detect.rs, lines.rs}` (+ testler, gerekirse insta).

**Arayüzler — Üretir:**

```rust
pub enum DumpFormat { Custom, Plain, Directory, Tar }                       // serde snake_case
pub enum BackupScope { Database, Schema { name: String }, Table { schema: String, name: String } }  // tag "kind"
pub struct BackupOptions { pub schema_only: bool, pub data_only: bool, pub no_owner: bool, pub no_privileges: bool, pub if_exists: bool, pub clean: bool, pub create: bool, pub inserts: bool, pub column_inserts: bool, pub compress: Option<u8>, pub jobs: Option<u8>, pub encoding: Option<String> }
pub struct BackupRequest { pub scope: BackupScope, pub format: DumpFormat, pub options: BackupOptions, pub output_path: String }
pub enum RestoreTarget { Existing, CreateFromArchive }                       // Existing: istek bağlamındaki veritabanı
pub struct RestoreOptions { pub clean: bool, pub if_exists: bool, pub create: bool, pub single_transaction: bool, pub jobs: Option<u8>, pub schema_only: bool, pub data_only: bool, pub no_owner: bool, pub no_privileges: bool, pub exit_on_error: bool }
pub struct RestoreRequest { pub path: String, pub target: RestoreTarget, pub options: RestoreOptions }
pub enum SourceFormat { Custom, Directory, Tar, Plain }
pub struct ArchiveInfo { pub archive_version: Option<String>, pub db_name: Option<String>, pub items: u32, pub counts: Vec<(String, u32)> }   // TS
pub struct Conn { pub host: String, pub port: u16, pub user: String, pub database: String, pub sslmode: &'static str, pub root_cert: Option<String>, pub client_cert: Option<String>, pub client_key: Option<String> }
pub struct Invocation { pub exe: String, pub args: Vec<String>, pub env: Vec<(String, String)> /* parolasız */, pub preview: String, pub risk: Risk, pub notes: Vec<String>, pub output: Option<OutputPlan> }
pub struct OutputPlan { pub final_path: String, pub partial_path: String, pub is_dir: bool }
pub fn backup_invocation(tool: &ToolSet, conn: &Conn, req: &BackupRequest) -> Result<Invocation, Vec<FieldError>>;
pub fn restore_invocation(tools: (&ToolSet /*restore*/, &ToolSet /*psql*/), conn: &Conn, maintenance_db: &str, req: &RestoreRequest, src: SourceFormat) -> Result<Invocation, Vec<FieldError>>;
pub fn sniff_format(head: &[u8], is_dir: bool, dir_has_toc: bool) -> SourceFormat;       // "PGDMP" → Custom; tar başlığı "toc.dat" → Tar; dizin+toc → Directory; aksi Plain
pub fn parse_list(output: &str) -> ArchiveInfo;                                           // `pg_restore -l` çıktısı
pub fn dump_pattern(ident: &str) -> String;                                               // pg_dump -n/-t desen kaçışı ("…", * ? . kaçışlı)
pub enum LineSeverity { Info, Warning, Error }
pub fn classify(line: &str) -> LineSeverity;                                              // "pg_dump: error:", "warning:", "pg_restore: error:", psql "ERROR:", "WARNING:"
pub fn progress_tick(line: &str, kind: JobKind) -> bool;                                  // ilerlemeye sayılır mı (spec §1)
pub fn failure_hint(lines: &[String]) -> Option<&'static str>;                            // spec §1 ipuçları
```
- `FieldError` ve `Risk` için `pgt-ddl`'deki tipler yeniden kullanılır (bağımlılık eklenir) ya da eşdeğerleri tanımlanır. Seçim rapora yazılır; TS'te aynı biçim olmalı.
- `env` `PGHOST`, `PGPORT`, `PGUSER`, `PGDATABASE`, `PGSSLMODE`, sertifika değişkenleri, `PGAPPNAME` ve `PGCONNECT_TIMEOUT` değişkenlerini içerir. Parola burada yoktur; yürütücü ekler.
- `preview` biçimi: tırnaklı tek satır komut + ortam özeti (`PGPASSWORD=***`).
- **Adım 1: Başarısız testler:**
  - Her format, seçenek ve kapsam için argüman ve ortam (insta).
  - Kural dışlamaları tablosu (alan yolları).
  - Sürüme bağlı farklar (ör. `--if-exists` `-c` gerektirir).
  - `dump_pattern`: düşmanca adlar (`"`, `*`, `.`, Unicode, boşluk).
  - `sniff_format` örnek baytları.
  - `parse_list` örneği (gerçek `pg_restore -l` çıktısından kesit).
  - `classify`, `progress_tick`, `failure_hint`.
  - Önizlemede parola yok.
  - Atomik çıktı yolları.
- **Adım 2–4:** uygula, `cargo test -p pgt-backup`, clippy, bindingler.
- **Adım 5: Commit** (`feat(backup): saf argüman/ortam üreticisi, format algılama ve satır sınıflandırma`).

### Task 3: Yürütücü ve iş kaydı

**Dosyalar:** Oluştur `crates/pgt-backup/src/{runner.rs, registry.rs, events.rs}` (+ testler).

**Arayüzler — Üretir:**

```rust
pub enum BackupEvent {                                                    // serde tag "event", camelCase; TS
    Started { preview: String },
    Line { severity: LineSeverity, text: String },
    Progress { done: u32, total: Option<u32>, bytes: Option<u64> },
    Finished { outcome: JobOutcome /* success|failed|cancelled */, exit_code: Option<i32>, elapsed_ms: u64, warnings: u32, errors: u32, output_path: Option<String>, hint: Option<String>, truncated_lines: u32 },
}
pub trait EventSink: Send { fn send(&mut self, e: BackupEvent) -> bool; }  // false → istemci gitti → iptal
pub struct Secret(String);                                                // Debug/Serialize parolayı yazmaz
pub async fn run(inv: Invocation, password: Option<Secret>, kind: JobKind, total: Option<u32>, sink: &mut dyn EventSink, cancel: CancelToken) -> JobSummary;
pub struct JobRegistry { … }  // register(job_id, output_path) -> Result<(CancelToken, Guard), "aynı hedefe çalışan iş">; cancel(job_id); cancel_all() -> usize
```
- **Çocuk süreç kurulumu:**
  - `env_clear` kullanılmaz: Windows'un `SystemRoot`/`PATH` gibi değişkenleri gerekir. Bunun yerine miras alınan tüm `PG*` anahtarları `env_remove` ile kaldırılır.
  - Ardından `inv.env` ve `PGPASSWORD` eklenir. Windows'ta `CREATE_NO_WINDOW`.
  - stdin null; stdout atılır (plain dahil, çünkü çıktı `-f` ile yazılır); stderr satır satır okunur.
- **İlerleme:**
  - `progress_tick` ile sayılır.
  - Bayt sayısı geçici çıktının boyutudur (dizin için dosyaların toplamı) ve 1 sn aralıkla `Progress` olayıyla gönderilir; bir sonraki olay `done` değişince ya da 1 sn dolunca gelir.
- **Bitiş:**
  - Çıkış kodu 0 ise geçici çıktı hedefe taşınır (hedef varsa önce silinir). Klasörde taşıma başarısız olursa kopyala+sil yapılmaz, hata verilir.
  - ≠ 0, iptal ya da sink false ise geçici çıktı silinir.
  - Son `Finished` olayında ipucu `failure_hint` ile hesaplanır.
- **İptal:** `CancelToken` tetiklenince `child.kill()` + `wait`.
- **Adım 1: Başarısız testler.** Sahte bir çalıştırılabilir kullanılır: test, Rust ile derlenmiş küçük bir yardımcı ikiliyi (`tests/bin/fake_tool.rs` gibi bir `[[bin]]` ya da `examples`) belirli stderr satırları yazdırıp verilen kodla çıkacak biçimde çalıştırır; gerçek PG gerekmez. Senaryolar:
  - satır akışı ve sınıflandırma;
  - 2500 satır → sınır + `truncated_lines`;
  - başarı → geçici çıktı hedefe taşınır;
  - hata → geçici çıktı silinir;
  - iptal (yavaş sahte araç) → `cancelled`, temizlik;
  - sink false → iptal;
  - miras `PGPASSWORD`/`PGSERVICE` çocuğa sızmaz (sahte araç ortamını yazdırır; test beklenen anahtarları denetler);
  - `PGPASSWORD` verildiğinde yalnız o değer var;
  - aynı hedefe ikinci iş reddedilir;
  - `cancel_all`.
- **Adım 2–4:** uygula; `cargo test -p pgt-backup`, clippy.
- **Adım 5: Commit** (`feat(backup): süreç yürütücüsü, ilerleme olayları, atomik çıktı ve iş kaydı`).

### Task 4: Tauri komutları, kapılar ve uygulama kancaları

**Dosyalar:** `src-tauri/src/backup.rs` (yeni), `src-tauri/src/state.rs` (`JobRegistry`), `src-tauri/src/main.rs` (komut kaydı, `cancel_all` kancaları), `src-tauri/Cargo.toml`; gerekirse `crates/pgt-catalog` (kapsam tablo sayısı sorgusu).

**Arayüzler — Üretir (Tauri):**

```rust
backup_tools(server_id: Uuid) -> ToolDetection                                // bağlıysa sunucu sürümüne göre seçilenleri de işaretler: ToolDetection.chosen{Dump,Restore,Psql}: Option<String>
backup_plan(server_id, database_oid, request: BackupRequest) -> JobPlan        // { preview, risk, notes, errors: FieldError[] }; çalıştırmaz
backup_start(server_id, database_oid, request, job_id: Uuid, on_event: Channel<BackupEvent>) -> JobSummary
restore_inspect(path: String) -> RestoreSource                                 // { format, archive?: ArchiveInfo, isDir }
restore_plan(server_id, database_oid: Option<u32>, request: RestoreRequest) -> JobPlan
restore_start(server_id, database_oid, request, job_id, confirmed: bool, confirmed_production: bool, on_event) -> JobSummary
backup_cancel(job_id: Uuid) -> ()
```
- **Bağlantı:**
  - `Conn`, `ServerConfig`'ten üretilir: SSL modu eşlemesi ve sertifika yolları.
  - Veritabanı adı `database_oid`'den çözülür (mevcut `db_name_locked`/katalog yardımcıları). `CreateFromArchive`'da veritabanı `session_maintenance_db` olur.
  - Sunucu sürümü `ConnectionManager::info` ile alınır. Sunucu bağlı değilse `not_connected`.
- **Parola:** `state.credentials.password(server_id)` ile alınır. `None` olur ve sunucu parola istiyorsa ilk deneme `password_required` değil, çocuk süreç "no password supplied" hatası verir. Bu yüzden komut başında parola yoksa ve sunucu kaydı parola kaydetmiyorsa (`save_password = false` ve oturum parolası yok) `password_required` hatası döner; ön yüz yeniden bağlanır.
- **Kapılar (`restore_start`):** sıra kilitli → plan doğrulaması (`errors` boş) → risk ≥ Destructive ise `confirmed` → DataLoss ve üretim sunucusu ise `confirmed_production`. `backup_start` yalnız plan doğrulamasını yapar.
- **Toplam:** yedeklemede kapsamdaki tablo sayısı (`relkind IN ('r','p','m')`, kapsam şeması/tablosu) katalog havuzundan sayılır; hata olursa `None` (belirsiz ilerleme). Geri yüklemede toplam `ArchiveInfo.items`'tır.
- **Kancalar:** `on_page_load` ve `RunEvent::Exit`'te `state.backup.cancel_all()`.
- **Günlük:** `tracing::info!(job = "backup"|"restore", outcome, exit_code, elapsed_ms)`; başka alan yazılmaz.
- **Adım 1: Başarısız testler.**
  - Birim:
    - SSL eşlemesi;
    - kapı sırası (sahte durumla, mümkünse saf yardımcı);
    - `password_required` koşulu;
    - kapsam sayım SQL'inin parametreli olması.
  - Docker (`crates/pgt-backup/tests/roundtrip.rs`; araç yoksa atlanır):
    - custom, plain, directory ve tar yedek → yeni veritabanına geri yükleme → tablo ve satır sayıları eşit;
    - şema ve tablo kapsamı;
    - `--clean` ile mevcut veritabanına yeniden yükleme;
    - plain + `psql -1` hatalı SQL'de geri alınır;
    - büyük tabloda iptal → geçici çıktı yok;
    - yanlış parola → ipucu;
    - sürüm uyumsuzluğu (araç sürümünden yeni sunucu varsa — bu makinede araç 18 olduğu için gerçek senaryo yok; saf testte kapsanır);
    - günlük yakalama (parola, yol, ad yok).
- **Adım 2–4:** uygula; `cargo test --workspace`, clippy, `cargo build -p pg-tools`, matris (`PGT_TEST_PGBIN` gerekirse matris betiğine geçirilir; betik değişikliği raporlanır). `bun run typecheck`.
- **Adım 5: Commit** (`feat(backup): Tauri komutları, kapılar ve iptal kancaları`).

### Task 5: Belgeler ve bütünleşik doğrulama

- Spec:
  - durum güncellenir;
  - **Plan 26 sözleşme notları (Plan 27 için)** eklenir: komut imzaları, olay biçimi, alan yolları, risk/kapı kuralları, ipuçları, araç seçimi metinleri.
- `docs/smoke-test.md`: "Yedekleme arka ucu (Plan 26)" bölümü (GUI maddeleri Plan 27'de).
- `docs/testing.md`: araç gerekliliği ve `PGT_TEST_PGBIN`.
- Doğrulama:
  - tam matris;
  - `cargo test --workspace`, clippy;
  - `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`.
- Kapanış taramaları:
  - `rg "PGPASSWORD" crates src-tauri` → yalnız yürütücü ve testler.
  - `rg "tracing::" crates/pgt-backup` → alanlarda yol, ad ya da metin yok.
- **Commit** (`docs(backup): Plan 26 sözleşme notları, smoke ve doğrulama`).

## Öz inceleme

- **Spec §1 kapsamı:**
  - Araç bulma/seçimi ve ayar → T1.
  - Yedekleme/geri yükleme argümanları, kurallar ve format algılama → T2.
  - Atomik çıktı, ilerleme, iptal ve sınırlar → T3.
  - Kimlik bilgisi, kapılar, toplamlar ve kancalar → T4.
  - Günlük → Global Constraints + T4 testleri.
  - §4 test → T1–T4.
- **Tip tutarlılığı:** `ToolSet`/`choose` (T1) → `Invocation` (T2) → `run` (T3) → komutlar (T4); `BackupEvent` (T3) → Tauri Channel (T4).
