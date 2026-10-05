# Query Tool — Plan 5/6: Çalıştırma Motoru, Oturumlar ve Korumalar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Query Tool'un **arka ucunu** kurmak: sunucu rengi/etiketi/salt okunur kilit alanları, havuzdan ayrı sorgu oturumları, SQL ifade ayırıcı + sınıflandırıcı + tehlikeli ifade tespiti, imleçle parça parça sonuç çeken çalıştırıcı (iptal, satır sınırı, NOTICE toplama, hata konumu, autocommit/manuel transaction), şema/tablo/kolon tamamlama sorguları, CSV akıtma ve bunların Tauri komutları/olay akışı. Arayüz **Plan 6**'dadır; bu plan sonunda arka uç `qt_*` komutlarıyla uçtan uca (Rust entegrasyon testleri ve Tauri komutları) çalışır.

**Architecture:** Yeni `pgt-query` crate'i saf mantığı (sözcük çözümleyici, ifade ayırıcı, sınıflandırıcı) ve çalıştırıcıyı barındırır; çalıştırıcı sonuçları Tauri'den bağımsız bir `EventSink` arayüzüne akıtır (Tauri komutları bu arayüzü `Channel`'a bağlar). `pgt-core` yalnızca iki şey kazanır: `ServerConfig` yeni alanları ve **oturum profiline göre** (katalog: salt okunur + 30 sn; sorgu: kullanıcı ayarlı) açılan ayrı bağlantı (`ConnectionManager::open_dedicated`). Sorgu bağlantısı `connection` nesnesini kendisi sürer (NOTICE iletilerini yakalamak için); transaction durumu tokio-postgres'te dışarı verilmediğinden (`ReadyForQuery` atılır — kaynak doğrulandı) **sınıflandırma tabanlı durum makinesi + hedefli yeniden eşitleme sorgusuyla** izlenir.

**Tech Stack:** Plan 1–4 ile aynı (Rust 2024, tokio-postgres 0.7.18 + rustls/ring, Tauri 2.12, ts-rs, insta; Docker matrisi). **Yeni bağımlılık:** yalnızca `futures-util` (`tokio-postgres`'in zaten çektiği; `Connection::poll_message` akışı için). Başka bağımlılık eklenmez (CSV elle yazılır).

**Spec:** `docs/superpowers/specs/2026-10-06-query-tool-design.md` (§3 oturumlar, §4 motor, §6 korumalar, §7 hata, §8 güvenlik, §9 test)

**Önceki planlar:** P1–P4 (Object Explorer) — `main`'de.

## Plan 4 sonrası kod gerçekleri (bu planın dayandığı yapı)

- `crates/pgt-core/src/connection.rs`: `ConnectionManager` (`connect`, `disconnect`, `run(server, Target, f)` ile **havuz** bağlantıları), `PgConn { client, info, last_used }`, `connect_once(config, dbname, password, with_options)` (özel) ve `open`/`open_inner` (options yedek yolu); `SESSION_OPTIONS` (`-c default_transaction_read_only=on -c statement_timeout=30000 -c search_path=pg_catalog`) ve `SESSION_SETS`; `session_settings_ok`, `apply_session_sets`. Bağlantı sürücüsü `tokio::spawn(async move { connection.await })` ile arka plana atılır (NOTICE iletileri **atılır**).
- Oturum yapısı: `Session { config, password, conns: HashMap<Target, Arc<PgConn>>, db_names: HashMap<u32, String> }`; `Target::{Maintenance, Database(oid)}`. Sunucuya bağlanmak = bakım veritabanı bağlantısı; `db_names` OID→ad eşlemesi veritabanı bağlantısı açılırken doldurulur.
- Hata modeli `crates/pgt-core/src/error.rs`: `Error::{Config, Invalid, Credential, PasswordRequired, Connect{kind,sqlstate,message}, Query{sqlstate,message,detail,hint}, NotFound, UnknownServer, NotConnected, ConnectionLost}`; `ErrorPayload{code,message,detail,hint}`; `Error::from_query/from_connect`. **tokio-postgres hatasında `position` bilgisi `Error::Query`'de yoktur**; bu plan hata konumunu `pgt-query` içinde ayrı bir `QueryFailure` ile taşır (çekirdek hata modeli değişmez).
- `ServerConfig` (`crates/pgt-core/src/config.rs`): `id, name, group, host, port, maintenance_db, username, save_password, ssl`; `#[serde(rename_all = "camelCase")]`, ts-rs `#[ts(export)]`; `ServerConfig::new`, `.normalized()`, `.validate()`. pgAdmin içe aktarma `crates/pgt-core/src/import.rs` (`parse_entry`).
- Ön yüz `ServerConfig` tipi `src/ipc/bindings/ServerConfig.ts` (üretilir); `src/features/servers/server-form.ts` form modelini kurar (yeni zorunlu alanlar burada varsayılanlanır).
- Tauri: `src-tauri/src/{commands.rs,main.rs,state.rs}`; **katalog/bağlantı `await`'leri `Box::pin` ile kutulanır** (release ana yığın taşması). `AppState { registry, credentials, manager, catalog, startup_notice }`.
- Test altyapısı: `PGT_TEST_URLS` (virgüllü) ile entegrasyon; **kalıcı test sunucusu yoktur**: Docker ile geçici PG (`scripts/test-matrix.ps1` kalıbı; tek sürüm için `docker run -d --name … -e POSTGRES_PASSWORD=<rastgele> -p 127.0.0.1:<port>:5432 postgres:<sürüm>`, `pg_isready` + 3 sn bekle, `PGT_TEST_URLS` yalnızca test komutunun ortamında, **işi bitince `docker rm -f`**). Parola/URL hiçbir yere yazılmaz. `scripts/test-matrix.ps1 [-SkipPerf]` PG 10–18'i çalıştırır.
- bun `~/.bun/bin/bun`; ts-rs bağlamaları `src/ipc/bindings` altına `cargo test` ile üretilir (satır sonu farkı olanları `git checkout -- src/ipc/bindings` ile geri alın; yeni tiplerin dosyalarını commit edin).

## Global Constraints

- Hedef: Windows 10/11, Server 2019/2022; PostgreSQL 10–18 vanilla. Hiçbir görev PG sürümüne özel katalog SQL'i gerektirmemeli; gerekiyorsa `VersionedSql` kalıbı ve matris testi.
- **Object Explorer'ın katalog katmanı salt okunur kalır** (havuz bağlantıları, `default_transaction_read_only = on`). Query Tool, "arayüze rastgele SQL açılmaz" ilkesinin **bilinçli ve tek istisnasıdır**; ayrı crate (`pgt-query`), ayrı bağlantılar, ayrı komutlar (`qt_*`). Katalog kodu `pgt-query`'ye bağımlı olmaz.
- Sorgu oturumunun `search_path`'ine dokunulmaz (kullanıcının yolu geçerli); ancak **tamamlama sorguları** kendi katalog adlarını `pg_catalog.` ile niteler ve oturum yolundan bağımsızdır.
- **Bellek sözleşmesi:** arka uç hiçbir koşulda sınırsız satır biriktirmez/göndermez: parça başına en çok 1.000 satır, oturum başına toplam satır sınırı (varsayılan 10.000; `limit` parametresi), doğrudan (imleçsiz) yol tüm satırları alsa da **sınırda keser** ve kesildiğini bildirir.
- **Parola, bağlantı dizgisi ve SQL metni hiçbir günlüğe/hata metnine yazılmaz** (günlük: yalnızca ifade türü sınıfı, süre, sonuç/hata kodu). Test kimlik bilgileri (`PGT_TEST_URLS`, Docker parolaları) rapora/commit'e/çıktıya yazılmaz.
- Katalog ve sorgu gelecekleri Tauri komutlarında `Box::pin` ile kutulanır.
- Rutin sorgu hataları modal diyalog açmaz (arayüz Plan 6'da); arka uç hataları `ErrorPayload` + `QueryFailure.position` olarak döner.
- Yeni bağımlılık yalnızca `futures-util` (workspace bağımlılığı olarak, `default-features = false`, gerekli özellikle). Başka bağımlılık yok.
- TLS kripto sağlayıcısı ring; `aws-lc-rs` açılmaz. UI metinleri bu planda yoktur (Plan 6).

### Bu planın bilinçli sınırları

- Arayüz (sekmeler, editör, ızgara, geçmiş, sunucu diyaloğu alanları, bağlam menüsü "Query Tool'da aç") **Plan 6**'dadır. Bu planda arayüze yalnızca tip uyumu için gereken en küçük değişiklikler yapılır (`server-form.ts` varsayılanları).
- Kapsam dışı: `COPY FROM/TO STDIN`, `psql` meta-komutları, hücre düzenleme, `EXPLAIN` görselleştirme, `LISTEN/NOTIFY`, large object.
- Bilinen sınırlar (belgelenir): doğrudan yoldaki dev `INSERT … RETURNING` tüm satırları geçici olarak bellekte alır (kesilmeden önce); tehlikeli ifade tespiti sözlüksel ve tutucudur (`WHERE true` yakalanmaz); transaction durumu sınıflandırmadan türetilir (fonksiyon içi dinamik `COMMIT` yakalanmaz).

## Review Focus

1. **Ayırıcı yanlış bölme:** dolar tırnaklı gövde (`$$ … ; … $$`, `$fn$ … $fn$`), `E'\''` kaçışı, `''` çifte tırnak, çift tırnaklı tanımlayıcıdaki `;`, iç içe `/* /* */ */`, `--` yorumundaki `;`, `BEGIN ATOMIC … END` (PG 14+ gövdeli fonksiyon; içinde `CASE … END`), parantez içi `;` (çok eylemli `CREATE RULE`), tanımlayıcı içindeki `$` (`a$b$`), `$1` parametresi; **kapanmamış** tırnak/yorum (hata sunucuya bırakılır, ayırıcı çökmemeli). Test: Task 3 tabloları + yeniden-birleştirme özelliği.
2. **Yazma/DDL'in kilitli sunucuda çalışması:** istemci reddi atlatılsa bile (sınıflandırıcı yanılsa: `WITH x AS (…) INSERT`, `DO $$…$$`, `CALL`, fonksiyon çağrısı ile yazma) oturum `default_transaction_read_only = on` açıldığından sunucu reddetmeli. Test: Task 6 `locked_session_rejects_writes_even_if_classifier_misses`.
3. **Tehlikeli ifade onayı atlatma:** `UPDATE t SET … ` (WHERE yok) onaysız çalışmamalı (arka uç `confirmation_required` döner — arayüz atlansa bile); `UPDATE … WHERE` alt sorgudaki WHERE sayılmamalı; `Üretim` etiketli sunucuda her yazma onaylı; yorum/dize içindeki "WHERE" kelimesi yanıltmamalı. Test: Task 4 + Task 6.
4. **İmleç ömrü:** autocommit'te açık imleç `idle in transaction` bırakır; yeni ifade çalışınca/sekme kapanınca/zaman aşımında (5 dk) kapanmalı; manuel modda kullanıcının transaction'ı imleç yüzünden commit/rollback edilmemeli; iptal sonrası oturum kullanılabilir kalmalı. Test: Task 5/6.
5. **Bellek sınırı:** `generate_series(1, 1000000)` çalıştırıldığında arka uç ve kanal yalnızca `limit` kadar satır taşımalı; "daha fazla" yalnızca istenince. Test: Task 5 `row_limit_bounds_memory`, Task 9 sayaç testi.
6. **Hata konumu ve çok baytlı metin:** PG `position` alanı **karakter** konumudur (bayt değil); Türkçe/emoji içeren sorgularda doğru UTF-16/karakter eşlemesi gerekir (arayüz Plan 6'da kullanır; arka uç ham `position`'ı ve ifade başlangıcının karakter ofsetini verir). Test: Task 5 `error_position_is_relative_to_statement_chars`.

## Dosya Haritası (bu plan)

```
Cargo.toml                                workspace: pgt-query üyesi, futures-util bağımlılığı
crates/pgt-core/src/config.rs             ServerConfig: color, label, read_only_lock (Task 1)
crates/pgt-core/src/import.rs             pgAdmin BGColor → renk (Task 1)
crates/pgt-core/src/connection.rs         SessionProfile, ConnectionManager::open_dedicated (Task 2)
crates/pgt-query/                         yeni crate
  src/lib.rs, lex.rs, split.rs, classify.rs     saf mantık (Task 3, 4)
  src/events.rs, exec.rs, session.rs            çalıştırıcı, oturum yöneticisi (Task 5, 6)
  src/complete.rs, export.rs                    tamamlama sorguları, CSV (Task 7)
  tests/…                                       entegrasyon testleri (Task 5–9)
src-tauri/src/query.rs                    qt_* Tauri komutları, Channel köprüsü (Task 8)
src/features/servers/server-form.ts       yeni alan varsayılanları (Task 1)
docs/testing.md, docs/performance.md, scripts/…   Task 9
```

---

### Task 1: `ServerConfig` — renk, etiket, salt okunur kilit; pgAdmin `BGColor`

**Files:**
- Modify: `crates/pgt-core/src/config.rs`, `crates/pgt-core/src/import.rs`, `src/features/servers/server-form.ts`
- Test: `crates/pgt-core/src/config.rs` (`#[cfg(test)]`), `crates/pgt-core/src/import.rs` (`#[cfg(test)]`), `crates/pgt-core/src/registry.rs` testleri, `src/features/servers/server-form.test.ts`
- Regenerated: `src/ipc/bindings/ServerConfig.ts`

**Interfaces:**
- Produces:
  - `pub const COLORS: [&str; 8] = ["red", "orange", "amber", "green", "teal", "blue", "purple", "gray"]`.
  - `ServerConfig { …, #[serde(default)] pub color: Option<String>, #[serde(default)] pub label: Option<String>, #[serde(default)] pub read_only_lock: bool }`; `validate()`: `color` varsa `COLORS` içinde olmalı; `label` ≤ 32 karakter, kontrol karakteri yok; `normalized()` boş `label`/`color`'ı `None` yapar ve kırpar.
  - `pub fn is_production(&self) -> bool` — `label` büyük/küçük harf duyarsız `"üretim"`, `"uretim"`, `"production"` ya da `"prod"` ise `true`.
  - `import.rs`: pgAdmin `BGColor` (`"#rrggbb"`) → `COLORS` içinden RGB mesafesi en yakın renk (`nearest_color(hex) -> Option<&'static str>`); geçersiz/boş → `None`.
  - Ön yüz: `server-form.ts` yeni üç alanı (`color: null, label: null, readOnlyLock: false`) taşır.

- [ ] **Step 1: Başarısız testler (`config.rs`).**
```rust
#[test]
fn new_fields_default_for_old_servers_json() {
    // Alanları olmayan eski kayıt okunabilmeli (serde default).
    let json = r#"{"id":"00000000-0000-0000-0000-000000000001","name":"p","group":null,"host":"h","port":5432,"maintenanceDb":"postgres","username":"u","savePassword":false}"#;
    let c: ServerConfig = serde_json::from_str(json).unwrap();
    assert_eq!((c.color.clone(), c.label.clone(), c.read_only_lock), (None, None, false));
}
#[test]
fn validate_checks_color_and_label() {
    let mut c = ServerConfig::new("p", "h", 5432, "u");
    c.color = Some("red".into());
    assert!(c.validate().is_ok());
    c.color = Some("magenta".into());
    assert!(matches!(c.validate(), Err(Error::Invalid(m)) if m.contains("renk")));
    c.color = None;
    c.label = Some("x".repeat(33));
    assert!(matches!(c.validate(), Err(Error::Invalid(m)) if m.contains("etiket")));
    c.label = Some("a\nb".into());
    assert!(c.validate().is_err());
}
#[test]
fn normalized_drops_blank_label_and_color() {
    let mut c = ServerConfig::new("p", "h", 5432, "u");
    c.label = Some("  ".into());
    c.color = Some("".into());
    let n = c.normalized();
    assert_eq!((n.label, n.color), (None, None));
}
#[test]
fn production_label_detection() {
    for (label, expected) in [("Üretim", true), ("uretim", true), ("PRODUCTION", true), ("prod", true), ("Test", false), ("preprod", false)] {
        let mut c = ServerConfig::new("p", "h", 5432, "u");
        c.label = Some(label.into());
        assert_eq!(c.is_production(), expected, "{label}");
    }
}
```
`import.rs`: `nearest_color("#ff0000") == Some("red")`, `"#0000ff" → "blue"`, `"#808080" → "gray"`, `"red" / "" / "#12" → None`; `BGColor` olan pgAdmin kaydı `config.color`'ı doldurur; `registry` döngü testi (kaydet→yükle) yeni alanları korur.

- [ ] **Step 2: Çalıştır → başarısız.** `cargo test -p pgt-core config:: import::`
- [ ] **Step 3: Uygula.** Alanlar + `COLORS` + `is_production` + doğrulama (Türkçe mesajlar: "Geçersiz renk.", "Etiket en fazla 32 karakter olabilir ve satır sonu içeremez."); `nearest_color`: `#rrggbb` ayrıştır, her `COLORS` girdisi için referans RGB (`red #dc2626, orange #ea580c, amber #d97706, green #16a34a, teal #0d9488, blue #2563eb, purple #9333ea, gray #6b7280`) ile öklid mesafesi en küçüğü. `server-form.ts` form modeli ve `toConfig`/`fromConfig` yeni alanları taşır (`server-form.test.ts`: dönüşüm gidiş-dönüş testi güncellenir).
- [ ] **Step 4: Doğrula ve commit.** `cargo test --workspace --all-features` (Docker'lı geçici PG ile; gerekmiyorsa atlanır), `bun run typecheck`, `bun run test`; `ServerConfig.ts` bağlamasını commit'e ekleyin.

```bash
git add crates/pgt-core src/features/servers src/ipc/bindings/ServerConfig.ts
git commit -m "feat(core): sunucu rengi, etiketi ve salt okunur kilit alanları; pgAdmin BGColor eşlemesi"
```

---

### Task 2: Ayrı (havuzsuz) sorgu bağlantısı — `SessionProfile` ve `open_dedicated`

**Files:**
- Modify: `crates/pgt-core/src/connection.rs`, `Cargo.toml` (workspace `futures-util`), `crates/pgt-core/Cargo.toml`
- Test: `crates/pgt-core/src/connection.rs` (`#[cfg(test)]`), `crates/pgt-core/tests/connection.rs`

**Interfaces:**
- Consumes: `connect_once`, `open_inner`, `SESSION_SETS`, `tls::make_connector`, `Session.config/password/db_names`.
- Produces:
  - `pub enum SessionProfile { Catalog, Query { read_only: bool, statement_timeout_ms: u32 } }`.
  - `pub struct DedicatedConn { pub client: tokio_postgres::Client, pub info: ServerInfo, pub notices: tokio::sync::mpsc::UnboundedReceiver<Notice>, pub cancel: tokio_postgres::CancelToken }` ve `pub struct Notice { pub severity: String, pub code: String, pub message: String, pub detail: Option<String>, pub hint: Option<String> }`. Bağlantı sürücüsü `futures_util::stream::poll_fn(|cx| connection.poll_message(cx))` ile sürülür; `AsyncMessage::Notice` → `Notice` kanala; `Notification` yok sayılır (kapsam dışı); bağlantı kapanınca görev biter.
  - `impl ConnectionManager { pub async fn open_dedicated(&self, server: Uuid, target: Target, profile: SessionProfile) -> Result<DedicatedConn> }` — sunucu **bağlı olmalı** (`Error::NotConnected` aksi halde); yapılandırma ve parola oturumdan alınır; `Target::Database(oid)` adı `db_names`'ten çözülür (yoksa bakım bağlantısı üzerinden `SELECT datname FROM pg_database WHERE oid = $1` ile bulunur ve önbelleğe yazılır; bulunamazsa `Error::NotFound`).
  - Profil davranışı: `Catalog` = bugünkü `SESSION_OPTIONS`/`SESSION_SETS`; `Query{read_only, statement_timeout_ms}` = `application_name = 'pg-tools query'`, `-c default_transaction_read_only=<on|off> -c statement_timeout=<ms>` (**`search_path` verilmez**); `options`'ı reddeden/yok sayan sunucularda aynı yedek yol (SET): `SET default_transaction_read_only = …; SET statement_timeout = …`; sürüm sorgusu üç ayarın etkinliğini doğrular (Plan 4 `session_settings_ok` kalıbı, profile göre genelleştirilir) ve eksikse SET uygular, hata → `Error::Connect{kind: Other}` ve istemci düşürülür.

- [ ] **Step 1: Başarısız testler.**
  - Birim: `profile_session_options(Query{read_only:true,statement_timeout_ms:0})` → `"-c default_transaction_read_only=on -c statement_timeout=0"`; `read_only:false, 300000` → `…=off -c statement_timeout=300000`; `Catalog` → mevcut `SESSION_OPTIONS`; **hiçbir profilde `search_path` yok** (Query için). `settings_ok_for(profile, read_only, timeout, search_path)` tablosu (Catalog: üçü zorunlu; Query: yalnızca read_only ve timeout; `statement_timeout` `"0"`, `"30s"`, `"5min"` biçimleri `timeout_ms` ile).
  - Entegrasyon (`tests/connection.rs`, `PGT_TEST_URLS` kapıları): `open_dedicated(Query{read_only:false,0})` ile bağlanıp `CREATE TEMP TABLE t(i int)` **başarılı**; `read_only:true` ile **reddedilir** (25006); `SHOW statement_timeout` beklenen; `SHOW search_path` kullanıcı yolunu verir (`"$user", public`); `RAISE NOTICE` çalıştıran `DO $$ BEGIN RAISE NOTICE 'merhaba'; END $$` sonrası `notices` kanalında `Notice{message:"merhaba"}`; `cancel` ile `SELECT pg_sleep(30)` **< 2 sn'de** `57014` ile biter (iptal belirteci TLS'siz yolda); bağlı olmayan sunucuda `Error::NotConnected`; sorgu oturumu havuz sayısını değiştirmez (`manager` havuz bağlantısı sayısı artmaz — mevcut `run` yolu etkilenmez).
- [ ] **Step 2: Çalıştır → başarısız.**
- [ ] **Step 3: Uygula.** `connect_once`'ı profile parametreli iç fonksiyona (`connect_with(config, dbname, password, profile, with_options) -> Result<(Client, Connection<…>, ServerInfo)>`) bölün; `Catalog` profili mevcut davranışı **birebir** korur (Plan 4 testleri değişmeden geçmeli). `PgConn` yolu `connection.await` ile sürmeye devam eder; `DedicatedConn` yolu `poll_message` ile sürer. `open_dedicated` içinde `Box::pin` gerekmez (komut katmanında kutulanır).
- [ ] **Step 4: Doğrula ve commit.** `cargo test --workspace --all-features` (Docker'lı PG; mümkünse iki sürüm), `scripts/test-matrix.ps1 -SkipPerf` bir kez.

```bash
git add Cargo.toml crates/pgt-core
git commit -m "feat(core): havuzsuz sorgu bağlantısı (SessionProfile, open_dedicated, NOTICE ve iptal)"
```

---

### Task 3: `pgt-query` crate'i — sözcük çözümleyici ve ifade ayırıcı

**Files:**
- Create: `crates/pgt-query/Cargo.toml`, `crates/pgt-query/src/{lib.rs,lex.rs,split.rs}`
- Modify: `Cargo.toml` (workspace üyesi, `pgt-query` bağımlılık girdisi)
- Test: `crates/pgt-query/src/lex.rs`, `crates/pgt-query/src/split.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `lex::{Kind, Token}`: `enum Kind { Word, Ident, Str, Dollar, Comment, Punct }`, `struct Token { kind: Kind, start: usize, end: usize, depth: u32 }` (`depth` = tokenden **önceki** parantez derinliği; bayt ofsetleri), `fn lex(sql: &str) -> Vec<Token>`, `impl Token { fn text<'a>(&self, sql: &'a str) -> &'a str; fn is_word(&self, sql: &str, w: &str) -> bool /* büyük/küçük harf duyarsız */ }`.
  - `split::{Stmt, split, stmt_at}`: `struct Stmt { start: usize, end: usize, region_start: usize, region_end: usize }` — `start..end` yorumsuz ilk/son anlamlı belirteç arası (sondaki `;` hariç) bayt aralığı; `region_*` ifadenin "imleç alanı" (bir önceki `;`'nin hemen sonrasından kendi `;`'si dahil); `fn split(sql: &str) -> Vec<Stmt>` (yalnızca yorum/boşluk olanlar atlanır); `fn stmt_at(sql: &str, byte_offset: usize) -> Option<usize>` — imleç hangi ifadenin alanındaysa onun indeksi; alanlar arasındaki boşlukta/ sondaki `;`'den sonraki boşlukta **bir önceki** ifade; hiç ifade yoksa `None`.

`Cargo.toml`: `tokio`, `tokio-postgres`, `pgt-core`, `serde`, `serde_json`, `uuid`, `tracing`, `ts-rs`, `futures-util` (workspace); dev: `tokio` (`test-util`), `tempfile`.

- [ ] **Step 1: `lex.rs` — sözcük çözümleyiciyi yaz ve test et.** Kurallar (kopyalayıp uygulayın):
```rust
pub fn lex(sql: &str) -> Vec<Token> {
    let b = sql.as_bytes();
    let (mut i, mut depth, mut out) = (0usize, 0u32, Vec::<Token>::new());
    let ident_start = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c >= 0x80;
    let ident_part = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() { i += 1; continue; }
        let start = i;
        let kind = if c == b'-' && b.get(i + 1) == Some(&b'-') {
            while i < b.len() && b[i] != b'\n' { i += 1; }
            Kind::Comment
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut nest = 1; i += 2;
            while i < b.len() && nest > 0 {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') { nest += 1; i += 2; }
                else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') { nest -= 1; i += 2; }
                else { i += 1; }
            }
            Kind::Comment
        } else if c == b'\'' {
            // E'..' : hemen önceki bitişik tek harfli E/e Word'ü bu dizeye katılır ve ters bölü kaçış sayılır.
            let esc = matches!(out.last(), Some(t) if t.kind == Kind::Word && t.end == i && t.end - t.start == 1 && matches!(b[t.start], b'e' | b'E'));
            let start = if esc { out.pop().unwrap().start } else { start };
            i += 1;
            while i < b.len() {
                if esc && b[i] == b'\\' { i += 2; continue; }
                if b[i] == b'\'' { if b.get(i + 1) == Some(&b'\'') { i += 2; continue; } i += 1; break; }
                i += 1;
            }
            out.push(Token { kind: Kind::Str, start, end: i.min(b.len()), depth }); continue;
        } else if c == b'"' {
            i += 1;
            while i < b.len() {
                if b[i] == b'"' { if b.get(i + 1) == Some(&b'"') { i += 2; continue; } i += 1; break; }
                i += 1;
            }
            Kind::Ident
        } else if c == b'$' && dollar_tag_end(b, i).is_some() {
            let tag_end = dollar_tag_end(b, i).unwrap(); // tag'ın son '$' indeksi
            let tag = &sql[i..=tag_end];
            i = match sql[tag_end + 1..].find(tag) { Some(p) => tag_end + 1 + p + tag.len(), None => b.len() };
            Kind::Dollar
        } else if ident_start(c) || c.is_ascii_digit() {
            i += 1; while i < b.len() && ident_part(b[i]) { i += 1; }
            Kind::Word
        } else {
            i += 1;
            if c == b'(' { out.push(Token { kind: Kind::Punct, start, end: i, depth }); depth += 1; continue; }
            if c == b')' { depth = depth.saturating_sub(1); }
            Kind::Punct
        };
        out.push(Token { kind, start, end: i, depth });
    }
    out
}

/// `$tag$` açılışı: `$` + (boş | [A-Za-z_\x80+][alfanümerik _ \x80+]*) + `$`; son `$` indeksini döner.
/// `$1` (konum parametresi) açılış değildir.
fn dollar_tag_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if j < b.len() && (b[j].is_ascii_alphabetic() || b[j] == b'_' || b[j] >= 0x80) {
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] >= 0x80) { j += 1; }
    }
    (b.get(j) == Some(&b'$')).then_some(j)
}
```
Not: `Kind::Punct` `;`, `,`, `.`, `=` gibi tek karakterlerdir; `)` belirteci `depth` azaltıldıktan **sonraki** derinlikle işaretlenir (yani `(` belirteci dış derinlikte, `)` belirteci dış derinlikte). Birim testleri (tablo): `select 'a;b'` → 2 belirteç (Word, Str); `select $$a;b$$` → Word+Dollar; `select $a$x$$y$a$` → tek Dollar; `select a$b$ from t` → `a$b$` tek Word; `select $1, $2` → Word + Punct(`$`) … (`$1`: `$` Punct, `1` Word — kabul); `E'\''` tek Str, `'it''s'` tek Str; `"a;b"` tek Ident; `/* a /* b */ c */ x` Comment+Word; `-- ;\nx` Comment+Word; kapanmamış `'abc` ve `/* abc` ve `$$abc` sonuna kadar tek belirteç ve **panik yok**; çok baytlı (`select 'çğ;ü' ; select 2`) bayt ofsetleri geçerli UTF-8 sınırlarında (`sql[t.start..t.end]` panik etmemeli); derinlik: `f(a,(b))` içindeki belirteçlerin `depth` değerleri.

- [ ] **Step 2: `split.rs` — ayırıcıyı yaz.** Algoritma: `lex` ile belirteçleri al; `;` Punct belirteci **`depth == 0` ve `atomic == 0`** ise ifadeyi bitirir. `BEGIN ATOMIC` izleme: `Word("ATOMIC")` belirteci, bir önceki **Word/yorum dışı** belirteç `Word("BEGIN")` ise `atomic = 1`; `atomic > 0` iken `Word("CASE")` `atomic += 1`, `Word("END")` `atomic -= 1` (0'a düşünce çıkılır; `END` sonrası gelen `;` ifadeyi bitirir). `start/end`: ifadenin ilk ve son **Comment olmayan** belirteci; hiç yoksa ifade atlanır. `region_start`: bir önceki ifadenin `;` belirtecinin `end`'i (ilk ifade için 0); `region_end`: kendi `;` belirtecinin `end`'i, yoksa `sql.len()`. `stmt_at`: `region_start <= off < region_end` olan ifade; `off` hiçbir alanda değilse (ör. yalnız boşluk/yorum sonrası) son ifadeden önceki en yakın ifade.
Testler (tablo odaklı, her satır beklenen ifade metinleri):
```rust
#[test]
fn splits_the_hard_cases() {
    let cases: &[(&str, &[&str])] = &[
        ("select 1; select 2", &["select 1", "select 2"]),
        ("select 1;;; select 2;", &["select 1", "select 2"]),
        ("select 'a;b'; select 2", &["select 'a;b'", "select 2"]),
        ("select E'\\'; x'; select 2", &["select E'\\'; x'", "select 2"]),
        ("select 'it''s;'; select 2", &["select 'it''s;'", "select 2"]),
        ("select \"a;b\" from t; select 2", &["select \"a;b\" from t", "select 2"]),
        ("create function f() returns int as $$ begin return 1; end $$ language plpgsql; select 2", &["create function f() returns int as $$ begin return 1; end $$ language plpgsql", "select 2"]),
        ("do $fn$ begin perform 1; end $fn$; select 2", &["do $fn$ begin perform 1; end $fn$", "select 2"]),
        ("select 1 /* ; /* ; */ ; */ ; select 2", &["select 1 /* ; /* ; */ ; */", "select 2"]),
        ("select 1 -- ; \n ; select 2", &["select 1 -- ; \n", "select 2"]),
        ("create rule r as on insert to t do also (insert into a values (1); insert into b values (2)); select 3", &["create rule r as on insert to t do also (insert into a values (1); insert into b values (2))", "select 3"]),
        ("create function f(a int) returns int language sql begin atomic select case when a > 0 then 1 else 2 end; select a; end; select 9", &["create function f(a int) returns int language sql begin atomic select case when a > 0 then 1 else 2 end; select a; end", "select 9"]),
        ("begin; select 1; commit", &["begin", "select 1", "commit"]),
        ("select a$b$ from t; select $1", &["select a$b$ from t", "select $1"]),
        ("select 'çğ;ü'; select 'x'", &["select 'çğ;ü'", "select 'x'"]),
        ("-- yalnızca yorum\n/* ve bu */", &[]),
        ("", &[]),
        ("select 'açık", &["select 'açık"]),
        ("select $$açık", &["select $$açık"]),
    ];
    for (sql, want) in cases {
        let got: Vec<&str> = split(sql).iter().map(|s| &sql[s.start..s.end]).collect();
        assert_eq!(&got, want, "{sql:?}");
    }
}
#[test]
fn stmt_at_picks_the_statement_under_the_cursor() {
    let sql = "select 1;\nselect 2;   \nselect 3";
    assert_eq!(stmt_at(sql, 3), Some(0));
    assert_eq!(stmt_at(sql, sql.find("select 2").unwrap() + 2), Some(1));
    assert_eq!(stmt_at(sql, sql.find("2;").unwrap() + 4), Some(1)); // `;` sonrası boşluk: önceki ifade
    assert_eq!(stmt_at(sql, sql.len()), Some(2));
    assert_eq!(stmt_at("", 0), None);
}
#[test]
fn rejoining_statements_reproduces_input_modulo_separators() {
    // Rastgele (sabit tohumlu) parçalardan SQL kur; ayırıcının ifade sayısı kurulan sayıya eşit olmalı.
    let frags = ["select 1", "select 'a;b'", "select $$x;y$$", "select \"q;\"", "select /* ; */ 2", "select 3 -- ;\n"];
    let mut seed = 0x2545F491u64;
    for _ in 0..200 {
        let n = 1 + (next(&mut seed) % 6) as usize;
        let picked: Vec<&str> = (0..n).map(|_| frags[(next(&mut seed) % frags.len() as u64) as usize]).collect();
        let sql = picked.join(";");
        assert_eq!(split(&sql).len(), n, "{sql:?}");
    }
}
fn next(s: &mut u64) -> u64 { *s ^= *s << 13; *s ^= *s >> 7; *s ^= *s << 17; *s }
```
- [ ] **Step 3: Çalıştır → başarısız; uygula; geçir.** `cargo test -p pgt-query`
- [ ] **Step 4: Commit.**

```bash
git add Cargo.toml crates/pgt-query
git commit -m "feat(query): sözcük çözümleyici ve SQL ifade ayırıcı (dolar tırnağı, BEGIN ATOMIC, iç içe yorum)"
```

---

### Task 4: İfade sınıflandırıcı, tehlikeli ifade ve hassas ifade tespiti

**Files:**
- Create: `crates/pgt-query/src/classify.rs`
- Modify: `crates/pgt-query/src/lib.rs`
- Test: `crates/pgt-query/src/classify.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `lex::{lex, Kind, Token}`, `split::split`.
- Produces:
  - `#[derive(Serialize, TS)] enum StmtKind { Select, Write, Ddl, Tx, Other }` (`#[serde(rename_all = "camelCase")]`, `#[ts(export)]`).
  - `#[derive(Serialize, TS)] enum Danger { UpdateWithoutWhere, DeleteWithoutWhere, Truncate, Drop, DropColumn }` (aynı serde/ts).
  - `struct Analysis { kind: StmtKind, danger: Option<Danger>, sensitive: bool, returns_rows_hint: bool }`; `fn analyze(stmt_sql: &str) -> Analysis` (tek ifade metni).
  - `fn is_tx_begin/ is_tx_end`: `TxOp { Begin, Commit, Rollback, Savepoint, Release, RollbackTo, Other }`, `fn tx_op(stmt_sql: &str) -> Option<TxOp>`.

Kurallar (spec §4.2, §6): ilk **anlamlı** belirteç (yorum ve baştaki `(` atlanır) büyük/küçük harf duyarsız:
- `SELECT|VALUES|TABLE|SHOW` → `Select` (`SELECT … INTO` yeni tablo yaratır → `Ddl`; belirteç taraması: üst düzey `INTO` ve kalıp `SELECT … INTO` → `Ddl`, `FOR UPDATE` kilitleri `Select` kalır).
- `WITH` → belirteçlerde (herhangi derinlikte) `INSERT|UPDATE|DELETE|MERGE` Word'ü varsa `Write`, yoksa `Select`.
- `EXPLAIN` → ikinci anlamlı belirteç `ANALYZE` (veya `(`…`ANALYZE`…`)`) **ve** belirteçlerde `INSERT|UPDATE|DELETE|MERGE` varsa `Write`; yoksa `Select`.
- `INSERT|UPDATE|DELETE|MERGE|COPY` → `Write` (`COPY … TO` `Select` sayılır: ikinci derece; `COPY … FROM` `Write`).
- `CREATE|ALTER|DROP|TRUNCATE|COMMENT|GRANT|REVOKE|REINDEX|CLUSTER|REFRESH|SECURITY|IMPORT|CHECKPOINT|ANALYZE|VACUUM|REASSIGN` → `Ddl`.
- `BEGIN|START|COMMIT|END|ROLLBACK|ABORT|SAVEPOINT|RELEASE|PREPARE|SET TRANSACTION` → `Tx` (`PREPARE TRANSACTION`, `COMMIT PREPARED` dahil; `PREPARE name AS …` `Other`).
- diğer her şey (`SET`, `RESET`, `DO`, `CALL`, `LOCK`, `LISTEN`, `DISCARD`, `EXECUTE`, …) → `Other`.
Tehlike (yalnızca `Write`/`Ddl` ve `Select`-olmayan): `UPDATE`/`DELETE` ifadesinde (CTE içi dahil: `WITH … DELETE …` ve `WITH … UPDATE …`) **`depth == 0` Word `WHERE`** yoksa `UpdateWithoutWhere`/`DeleteWithoutWhere` (hangi eylem ana ifadedir: CTE'de son üst düzey `INSERT|UPDATE|DELETE` Word'ü); `TRUNCATE` → `Truncate`; `DROP` (ilk belirteç) → `Drop`; `ALTER` ifadesinde `depth == 0` `DROP` Word'ü → `DropColumn` (ad kötü ama kapsar: `DROP COLUMN/CONSTRAINT`). Yorumlar ve dize/dolar/tırnaklı tanımlayıcı **içindeki** "where" tehlikeyi gizlemez (`lex` bunları ayrı belirteç yapar; yalnızca `Kind::Word` bakılır). `sensitive`: `CREATE|ALTER` ifadesinde `PASSWORD` veya `CONNECTION` Word'ü (geçmişe yazılmaz). `returns_rows_hint`: `Select`, `Write` içinde `RETURNING` Word'ü, `EXPLAIN`, `SHOW` (çalıştırıcıya ipucu; kesin karar `prepare` sütunlarındandır).

- [ ] **Step 1: Başarısız testler.**
```rust
#[test]
fn classifies_statement_kinds() {
    use StmtKind::*;
    for (sql, want) in [
        ("select 1", Select), ("  /* c */ (select 1) union (select 2)", Select), ("values (1)", Select), ("table t", Select), ("show all", Select),
        ("select * into t2 from t", Ddl),
        ("with a as (select 1) select * from a", Select),
        ("with a as (delete from t returning *) select * from a", Write),
        ("explain select 1", Select), ("explain analyze select 1", Select), ("explain analyze delete from t", Write),
        ("insert into t values (1)", Write), ("update t set a=1", Write), ("delete from t", Write), ("merge into t using s on true when matched then delete", Write),
        ("copy t from stdin", Write), ("copy t to stdout", Select),
        ("create table t(i int)", Ddl), ("drop table t", Ddl), ("truncate t", Ddl), ("grant select on t to u", Ddl), ("vacuum t", Ddl),
        ("begin", Tx), ("start transaction", Tx), ("commit", Tx), ("end", Tx), ("rollback to sp", Tx), ("savepoint sp", Tx), ("prepare transaction 'x'", Tx),
        ("set search_path = x", Other), ("do $$ begin end $$", Other), ("call p()", Other), ("prepare q as select 1", Other),
    ] {
        assert_eq!(analyze(sql).kind, want, "{sql}");
    }
}
#[test]
fn detects_dangerous_statements() {
    use Danger::*;
    for (sql, want) in [
        ("update t set a=1", Some(UpdateWithoutWhere)),
        ("update t set a=1 where id=1", None),
        ("update t set a=(select max(b) from u where u.x=t.x)", Some(UpdateWithoutWhere)), // alt sorgudaki WHERE sayılmaz
        ("delete from t", Some(DeleteWithoutWhere)),
        ("delete from t where true", None), // bilinen sınır: hileler yakalanmaz
        ("with d as (delete from t returning *) select * from d", Some(DeleteWithoutWhere)),
        ("with d as (select 1) delete from t using d where t.id = d.x", None),
        ("delete from t -- where id=1", Some(DeleteWithoutWhere)),
        ("delete from t /* where */", Some(DeleteWithoutWhere)),
        ("update t set a='where'", Some(UpdateWithoutWhere)),
        ("update \"where\" set a=1", Some(UpdateWithoutWhere)),
        ("truncate t", Some(Truncate)), ("drop table t", Some(Drop)), ("drop index i", Some(Drop)),
        ("alter table t drop column c", Some(DropColumn)), ("alter table t add column c int", None),
        ("select 1", None), ("insert into t values (1)", None), ("create table t(i int)", None),
    ] {
        assert_eq!(analyze(sql).danger, want, "{sql}");
    }
}
#[test]
fn flags_sensitive_statements() {
    for (sql, want) in [
        ("alter role app password 'x'", true), ("create user u with password 'x'", true),
        ("create subscription s connection 'host=h password=p' publication x", true),
        ("select 'password'", false), ("create table password (i int)", true), // tutucu: ad olarak da geçse işaretlenir
    ] {
        assert_eq!(analyze(sql).sensitive, want, "{sql}");
    }
}
#[test]
fn recognises_transaction_operations() {
    for (sql, want) in [("begin", Some(TxOp::Begin)), ("start transaction read only", Some(TxOp::Begin)), ("commit", Some(TxOp::Commit)), ("end", Some(TxOp::Commit)),
        ("rollback", Some(TxOp::Rollback)), ("abort", Some(TxOp::Rollback)), ("rollback to savepoint a", Some(TxOp::RollbackTo)), ("savepoint a", Some(TxOp::Savepoint)),
        ("release a", Some(TxOp::Release)), ("select 1", None), ("rollback prepared 'x'", Some(TxOp::Other)), ("commit prepared 'x'", Some(TxOp::Other))] {
        assert_eq!(tx_op(sql), want, "{sql}");
    }
}
```
(`create table password` testi tutuculuğu belgeler; kabul edilen yanlış pozitif.)
- [ ] **Step 2: Çalıştır → başarısız; uygula; geçir.** `cargo test -p pgt-query classify::`
- [ ] **Step 3: Commit.**

```bash
git add crates/pgt-query
git commit -m "feat(query): ifade sınıflandırıcı, tehlikeli/hassas ifade ve transaction işlemi tespiti"
```

---

### Task 5: Çalıştırıcı — hazırlama, imleç, doğrudan yol, NOTICE, hata konumu, iptal

**Files:**
- Create: `crates/pgt-query/src/{events.rs,exec.rs}`, `crates/pgt-query/tests/exec.rs`
- Modify: `crates/pgt-query/src/lib.rs`
- Test: `crates/pgt-query/tests/exec.rs` (Docker'lı PG; `PGT_TEST_URLS` kapılı)

**Interfaces:**
- Consumes: `pgt_core::connection::{DedicatedConn, Notice, SessionProfile, ConnectionManager}`, `split::split`, `classify::analyze`.
- Produces (`events.rs`, hepsi `Serialize + TS`, `#[serde(rename_all = "camelCase")]`, `#[ts(export)]`):
```rust
pub struct Column { pub name: String, pub type_name: String, pub type_oid: u32 }
pub enum QueryEvent {
    Started { stmt_index: u32, kind: StmtKind, columns: Vec<Column>, char_offset: u32 },
    Rows { stmt_index: u32, rows: Vec<Vec<Option<String>>>, more: bool },
    Done { stmt_index: u32, tag: String, rows_affected: Option<u64>, rows_sent: u64, truncated: bool, elapsed_ms: u64, notices: Vec<NoticeInfo> },
    Failed { stmt_index: u32, error: ErrorPayload, position: Option<u32>, char_offset: u32 },
    ScriptDone { stmts_run: u32, stopped_on_error: bool },
}
pub struct NoticeInfo { pub severity: String, pub code: String, pub message: String, pub detail: Option<String>, pub hint: Option<String> }
pub trait EventSink: Send { fn send(&mut self, ev: QueryEvent) -> bool; /* false = alıcı gitti, çalıştırma durur */ }
```
  `char_offset`: ifadenin metindeki **karakter** (Unicode scalar) ofseti; `position`: PG'nin ham 1 tabanlı **karakter** konumu **ifade metnine** göre (ifade içi; arayüz `char_offset + position - 1` kullanır).
  - `exec.rs`:
```rust
pub struct ExecConfig { pub limit: u32 /* toplam satır sınırı */, pub chunk: u32 /* 1000 */, pub autocommit: bool }
pub struct Executor { conn: DedicatedConn, … }   // oturum başına bir tane (Task 6 sarar)
impl Executor {
    pub async fn run_statement(&mut self, idx: u32, sql: &str, char_offset: u32, cfg: &ExecConfig, sink: &mut dyn EventSink) -> StmtOutcome; // Ok/Err
    pub async fn more(&mut self, idx: u32, sink: &mut dyn EventSink) -> Result<()>;   // açık imleçten sonraki parça
    pub async fn close_cursor(&mut self) -> Result<()>;
    pub fn cancel_token(&self) -> tokio_postgres::CancelToken;
}
pub enum StmtOutcome { Completed, Failed }
```

Çalıştırma kuralları (spec §4.3):
1. `client.prepare(sql)` → `Statement::columns()` (ad, `Type::name()`, `Type::oid()`); prepare hatası → `Failed` (konum `DbError::position()`: `Position::Original(n)` → `Some(n)`, `Internal` → `None`).
2. **Sütun varsa (satır döndüren):** autocommit ve açık transaction yoksa `BEGIN`; `DECLARE pgt_c NO SCROLL CURSOR FOR <sql>` (**imleç adı sabit; aynı anda tek açık imleç**; yeni ifade başlamadan önce `close_cursor`); `DECLARE` `42P11` (cannot open … query as cursor) veya sözdizimi nedenli başarısızsa **doğrudan yol**: `simple_query(sql)` ile tüm satırlar toplanır, `limit`'te kesilir (`truncated = true`). İmleç yolu: `FETCH FORWARD <chunk> FROM pgt_c` `simple_query` ile; her parça `Rows{…, more}`; toplam `limit`'e ulaşınca durur (`more = true` ise imleç açık kalır, `more()` ile `chunk` kadar daha; `limit` aşılmaz — `more()` her çağrıda en çok `chunk` ekler, satır sınırı **tek bir ifadenin toplam sınırı değil**, ilk yüklemenin sınırıdır; "daha fazla" kullanıcı onayıdır).
3. **Sütun yoksa:** `simple_query(sql)`; `CommandComplete(n)` → `rows_affected`; `tag` komut etiketi (tokio-postgres `simple_query` etiketi vermiyorsa `tag` = sınıflandırıcıdan türetilen anahtar kelime, örn. `INSERT`, `UPDATE`, `CREATE TABLE`… — `analyze` ilk 1–2 belirteç).
4. **Notice'lar:** ifade boyunca `DedicatedConn.notices` boşaltılıp `Done.notices`'e eklenir.
5. **Süre:** `Instant` ile gönderimden tamamlanmaya `elapsed_ms`.
6. **Hata:** `Failed{error: Error::from_query(e).into(), position}`; ifade imleç yolundaysa imleç/transaction kapatılır (autocommit'teyse `ROLLBACK`; manuel modda kullanıcının transaction'ı `aborted` kalır — **dokunulmaz**).
7. **İptal:** `cancel_token().cancel_query(NoTls veya aynı TLS bağlayıcı)` — `DedicatedConn.cancel` TLS'i kendisi taşır (Task 2); iptal edilen ifade `57014` ile `Failed` döner ve oturum kullanılabilir kalır (autocommit imleç transaction'ı geri alınır).
8. **Satır sınırı:** `limit = 0` geçersiz (`Error::Invalid`); üst sınır `1_000_000` (aşarsa kısılır).

- [ ] **Step 1: Başarısız entegrasyon testleri** (`tests/exec.rs`; `PGT_TEST_URLS` yoksa atlanır; yardımcı: `connect_dedicated(server_url, profile)` — mevcut `pgt_core::testing` yardımcılarıyla ManagerFixture kur). Test sink'i olayları `Vec`'e toplar.
  - `select_streams_columns_then_rows_then_done` (`select i, 'x'::text t, null::int n from generate_series(1,3) i`): `Started{columns:[i int4, t text, n int4]}`, `Rows` toplam 3, `NULL` = `None`, boş dize ≠ `None` (`select ''`), `Done{rows_sent:3, truncated:false}`.
  - `row_limit_bounds_memory` (`select * from generate_series(1,100000)` `limit=2500, chunk=1000`): toplam gönderilen satır **tam 2500**, son `Rows.more == true`, `Done.truncated == true`; `more()` ile 1000 satır daha gelir; `Executor` içinde biriken satır sayısı sayacı (test erişimi için `#[cfg(test)]` alan) `limit`'i aşmaz.
  - `insert_returning_uses_direct_path` (`create temp table`; `insert … returning id` 5 satır): çıktı doğru; doğrudan yol kullanıldığı (`Executor::last_path() == Path::Direct`) ve `limit=2` ile `truncated == true`.
  - `explain_and_show_work` (`explain select 1`, `show server_version`): satır döner.
  - `non_row_statement_reports_tag_and_count` (`create temp table`, `insert … values (1),(2)`, `update … `, `delete …`): `rows_affected` 2/…; `tag` anahtar kelime.
  - `text_formats_cover_exotic_types` (`select '{1,2}'::int[], '[1,5)'::int4range, '{"a":1}'::jsonb, '\xDEAD'::bytea, 1.50::numeric, '2026-01-02 03:04:05+00'::timestamptz, 'a b'::name`): her değer PG metin gösterimiyle birebir (`{1,2}`, `[1,5)`, `{"a": 1}`, `\xdead`, `1.50`, …).
  - `notices_are_collected` (`do $$ begin raise notice 'merhaba'; raise warning 'uyarı'; end $$`): `Done.notices` iki öğe, ciddiyetler `NOTICE`/`WARNING`.
  - `error_has_position_and_stops_script` (`select 1 + 'x'::text` ve `selectt 1`): `Failed.position == Some(n)`; `error.code == "query"`, SQLSTATE metni ayrıntıda; ardından oturum hâlâ çalışır (`select 1` başarılı).
  - `error_position_is_relative_to_statement_chars` (`select 'çğü💥' || x` — tanımsız sütun): PG'nin `position` değeri **karakter** tabanlıdır ve `char_offset + position - 1` hata belirtecini (`x`) gösterir; `char_offset` ifade metnin başında olmayan bir betikte (ikinci ifade) doğru.
  - `cancel_stops_a_long_query` (`select pg_sleep(30)` başlatılır, 300 ms sonra iptal): **< 2 sn**'de `Failed{57014}`; ardından `select 1` çalışır.
  - `cursor_is_closed_before_next_statement` (autocommit, `limit=10` ile 1000 satırlık sorgu `more=true`; sonra `select 1`): ikinci ifade başarılı, sunucuda `select count(*) from pg_stat_activity where state='idle in transaction' and pid = pg_backend_pid()` **0**; oturum `idle in transaction` kalmaz.
  - `manual_mode_cursor_does_not_end_user_transaction` (kullanıcı `begin` çalıştırır, `select` imleçli, sonra `rollback`): `begin`/`rollback` yalnızca kullanıcının ifadeleridir; çalıştırıcı kendi `BEGIN`/`COMMIT`'ini **eklemez** (`ExecConfig.autocommit=false`).
  - `statement_timeout_error_is_friendly` (`set statement_timeout=100; select pg_sleep(1)`): `error.message` Türkçe "Sorgu zaman aşımına uğradı…" (mevcut `friendly_sqlstate`).
  - Sink `false` döndürürse (alıcı gitti) çalıştırma biter ve imleç kapatılır (`sink_disconnect_stops_execution`).
- [ ] **Step 2: Çalıştır → başarısız.** (Docker'lı geçici PG; bkz. bağlam.)
- [ ] **Step 3: Uygula** (yukarıdaki kurallar). Önemli ayrıntılar: simple_query yanıtı `SimpleQueryMessage::{Row, CommandComplete, RowDescription}` ayrıştırılır (`Row::get(i) -> Option<&str>`); `FETCH` sonucu satır sayısı `chunk`'tan azsa imleç tükenmiştir (`more=false`, ardından `CLOSE` + gerekirse `COMMIT`); `BEGIN` yalnızca `autocommit && !in_tx` iken eklenir ve **tek `batch_execute`** ile (`BEGIN; DECLARE …` ayrı round trip değil: `DECLARE` hata verirse `ROLLBACK`); `COMMIT/ROLLBACK` imleç kapanışında. Dosya başına yorum: SQL metni günlüğe **yazılmaz** (`tracing` alanlarında yalnızca `kind`, `elapsed_ms`, `rows_sent`, `code`).
- [ ] **Step 4: Doğrula ve commit.** İki PG sürümünde (ör. 10 ve 18) çalıştırın; sonra Task 9'daki matris.

```bash
git add crates/pgt-query
git commit -m "feat(query): çalıştırıcı — imleçle parça parça sonuç, doğrudan yol, NOTICE, hata konumu, iptal"
```

---

### Task 6: `QueryManager` — oturumlar, transaction durumu, korumalar

**Files:**
- Create: `crates/pgt-query/src/session.rs`, `crates/pgt-query/tests/session.rs`
- Modify: `crates/pgt-query/src/lib.rs`
- Test: `crates/pgt-query/src/session.rs` (`#[cfg(test)]`: durum makinesi), `crates/pgt-query/tests/session.rs` (Docker'lı PG)

**Interfaces:**
- Consumes: Task 1 `ServerConfig::{read_only_lock, is_production}`, Task 2 `open_dedicated`, Task 3–5.
- Produces:
```rust
#[derive(Serialize, TS)] enum TxState { Idle, InTransaction, Failed }          // camelCase, ts(export)
#[derive(Serialize, Deserialize, TS)] enum TxMode { Autocommit, Manual }
#[derive(Serialize, Deserialize, TS)] struct SessionOptions { mode: TxMode, statement_timeout_ms: u32 /* 0 = sınırsız */ }
#[derive(Serialize, TS)] struct StmtPlan { index: u32, kind: StmtKind, danger: Option<Danger>, production_write: bool, locked_reject: bool, sensitive: bool, char_offset: u32 }
pub struct QueryManager { … }   // Arc ile paylaşılır
impl QueryManager {
    pub fn new(mgr: Arc<ConnectionManager>, registry: Arc<RwLock<Registry>>) -> Self;
    pub async fn open(&self, server: Uuid, db: Target, opts: SessionOptions) -> Result<SessionId /* Uuid */>;
    pub fn analyze(&self, session: SessionId, sql: &str, scope: Scope) -> Result<Vec<StmtPlan>>;   // Scope::{All, Selection{start,end}, AtCursor{offset}} (karakter ofsetleri)
    pub async fn run(&self, session: SessionId, sql: &str, scope: Scope, limit: u32, confirmed: bool, sink: &mut dyn EventSink) -> Result<()>;
    pub async fn more(&self, session: SessionId, stmt_index: u32, sink: &mut dyn EventSink) -> Result<()>;
    pub fn cancel(&self, session: SessionId);                 // kilitsiz: iptal belirteci ayrı saklanır
    pub async fn tx(&self, session: SessionId, op: TxAction /* Commit | Rollback */) -> Result<TxState>;
    pub fn tx_state(&self, session: SessionId) -> Result<TxState>;
    pub async fn set_options(&self, session: SessionId, opts: SessionOptions) -> Result<()>; // statement_timeout/mod; modu değiştirmek açık transaction varken reddedilir
    pub async fn close(&self, session: SessionId, tx_policy: ClosePolicy /* Commit | Rollback | Refuse */) -> Result<()>;
    pub fn reap_idle(&self, max_idle: Duration);              // 30 dk boşta: bağlantıyı kapat, oturum "uyku" (yeniden bağlanır); açık transaction varsa dokunma
}
```
Kurallar:
- **Kilit/salt okunurluk:** oturum açılırken sunucunun `read_only_lock`'u okunur ve `SessionProfile::Query{read_only: lock, …}` kullanılır. `run`: `locked_reject = lock && kind ∈ {Write, Ddl}` ise **hiçbir ifade çalışmadan** `Err(Error::Invalid("Bu sunucu salt okunur kilitli: yazma ve şema değiştiren ifadeler çalıştırılamaz."))` (betikte herhangi bir ifade reddedilirse tüm betik başlamaz).
- **Onay kapısı:** `run(…, confirmed=false)`: planda `danger.is_some()` **veya** `production_write` (`is_production() && kind ∈ {Write, Ddl}`) varsa `Err(Error::Invalid("confirmation_required"))` — **arka uç, arayüz atlanırsa bile** çalıştırmaz (kod: `ErrorPayload.code == "invalid"`, `message == "confirmation_required"`; Plan 6 bunu onay diyaloğuna çevirir). `confirmed=true` ise çalışır.
- **Transaction durumu (sınıflandırma tabanlı durum makinesi):** `Idle` + `tx_op=Begin` → `InTransaction`; `InTransaction` + `Commit|Rollback` → `Idle`; ifade hatası: oturum `InTransaction` ise → `Failed` (PG aborted), `Idle` ise `Idle`; `Failed` iken `Rollback|RollbackTo` → `Idle`/`InTransaction` (`RollbackTo` sonrası `InTransaction`); **`Failed` iken başka ifade çalıştırılmaz** (`Err(Error::Invalid("Transaction hata durumunda; önce ROLLBACK yapın."))`). Manuel mod: ilk ifadede (Tx sınıfı olmayan) `Idle`'dan **otomatik `BEGIN`** (`ExecConfig.autocommit=false`; `tx_state` → `InTransaction`). **Yeniden eşitleme:** her `Tx` sınıfı ifade ve her hata sonrası `SELECT statement_timestamp() <> transaction_timestamp()` (bir round trip) çalıştırılır: `true` → `InTransaction`; `false` → `Idle`; sorgu `25P02` ile hata verirse → `Failed`. Çelişki (makine ≠ eşitleme) durumunda **eşitleme kazanır** ve `tracing::warn!` yazılır (SQL metni yok).
- **İmleç yaşam döngüsü:** yeni `run`/`close`/`tx`/`set_options` önce açık imleci kapatır; imleç 5 dk `more()` almazsa arka plan görevi kapatır (autocommit'te ilgili `COMMIT`/`ROLLBACK` ile).
- **Betik:** ifadeleri sırayla çalıştırır; ilk `Failed`'da durur (`ScriptDone{stopped_on_error:true}`); `Started`/`Done` olayları ifade indeksiyle gelir; `stmt_index` betik içindeki **tüm ifadelere göre** (kapsam seçimi yalnızca çalıştırılacakları seçer, indeksler `Scope` içi sıradır).
- **Çalıştırma sırası ve eşzamanlılık:** oturum başına tek çalıştırma (ikincisi `Err(Error::Invalid("Bu sekmede bir sorgu zaten çalışıyor."))`); `cancel` çalışırken çağrılabilir.
- **Kapanış politikası:** `close(Refuse)` açık transaction varsa `Err(Error::Invalid("Açık transaction var."))`.
- Oturum haritası `StdMutex<HashMap<Uuid, Arc<SessionSlot>>>`; slot içinde `tokio::sync::Mutex<Inner>` (executor, durum, seçenekler) + kilitsiz `CancelToken`.

- [ ] **Step 1: Başarısız testler.**
  - Birim (durum makinesi, saf): tablo `(durum, olay) → durum` — `Idle+Begin→InTransaction`, `InTransaction+Commit→Idle`, `InTransaction+StmtError→Failed`, `Idle+StmtError→Idle`, `Failed+Rollback→Idle`, `Failed+RollbackTo→InTransaction`, `Failed+Select→reddedilir`, `InTransaction+Savepoint→InTransaction`.
  - Plan/koruma birimleri (`analyze` + kurallar): kilitli sunucuda `insert` → `locked_reject`; `Üretim` etiketi + `update … where id=1` → `production_write`; etiketsiz + `update` (WHERE yok) → `danger`; `confirmed=false` ile `Err("confirmation_required")`, `confirmed=true` ile geçer; kilitli sunucuda `select` serbest; betikte tek bir yazma ifadesi tüm betiği engeller.
  - Entegrasyon: `locked_session_rejects_writes_even_if_classifier_misses` — kilitli oturumda sınıflandırıcıyı atlatan ifadeler (`with x as (select 1) insert into t select * from x` sınıflandırıcıda `Write`'tır; atlatıcı örnekler: `do $$ begin insert into t values (1); end $$` (`Other`), `select f_that_writes()`) **sunucu tarafından** `25006` ile reddedilir (arka uç `Failed` olayı); `tx_state_follows_user_begin_commit` (kullanıcı `begin; insert; commit` yazar → `InTransaction` → `Idle`); `failed_transaction_blocks_until_rollback` (`begin; select 1/0` → `Failed`; `select 1` → `Err(…ROLLBACK…)`; `rollback` → `Idle`); `manual_mode_auto_begins` (Manual modda `insert` → `InTransaction`; `tx(Rollback)` → `Idle`, eklenen satır yok; `tx(Commit)` ile kalıcı); `close_refuses_open_transaction`; `second_run_is_rejected_while_running`; `cancel_while_running`; `set_options_changes_statement_timeout` (`set_options` sonrası `show statement_timeout`); `mode_change_rejected_in_transaction`; `reap_idle_sleeps_and_reopens` (zaman `start_paused` ile; uyku sonrası `run` sessizce yeniden bağlanır; açık transaction varsa dokunulmaz); `sensitive_statements_are_flagged_in_plan`; `idle_cursor_closes_after_timeout` (zaman ileri sarılır).
- [ ] **Step 2: Çalıştır → başarısız; uygula; geçir.** Docker'lı geçici PG (iki sürüm).
- [ ] **Step 3: Commit.**

```bash
git add crates/pgt-query
git commit -m "feat(query): QueryManager — oturumlar, transaction durum makinesi, kilit ve onay kapısı"
```

---

### Task 7: Tamamlama sorguları ve CSV akıtma

**Files:**
- Create: `crates/pgt-query/src/{complete.rs,export.rs}`, `crates/pgt-query/tests/{complete.rs,export.rs}`
- Modify: `crates/pgt-query/src/lib.rs`, `crates/pgt-query/src/session.rs` (`complete`, `export_csv` metotları)
- Test: yukarıdaki dosyalar (Docker'lı PG)

**Interfaces:**
- Produces:
  - `#[derive(Serialize, TS)] struct CompletionItem { label: String, kind: CompletionKind /* Schema|Table|View|MatView|Column|Function */, detail: Option<String> }`.
  - `QueryManager::complete(&self, session, req: CompletionRequest) -> Result<Vec<CompletionItem>>` — `CompletionRequest { kind: Schemas | Relations{schema: Option<String>} | Columns{schema: Option<String>, relation: String} }`; **ayrı kısa ömürlü sorgu bağlantısı kullanılmaz**; oturumun kendi bağlantısı **meşgul değilse** kullanılır, meşgulse (sorgu çalışıyor) boş liste döner (tamamlama sorgu çalıştırmayı **asla bekletmez**). Oturumda açık transaction `Failed` ise boş liste.
  - Sorgular (sabit SQL, `$n` parametreli, **hepsi `pg_catalog.` nitelikli**, `search_path`'ten bağımsız, `quote_ident` sunucuda): şemalar (`pg_namespace`, `pg_%`/`information_schema` hariç — kullanıcı `pg_catalog.` yazdığında sistem şemaları için ayrı istek `include_system`), ilişkiler (`pg_class` relkind `r,p,v,m,f` `relnamespace` filtreli; `schema` yoksa `current_schemas(false)` yolundakiler), kolonlar (`pg_attribute` `attnum > 0 AND NOT attisdropped`, tür `pg_catalog.format_type`). `label` ham adı, `insert_text` yerine `quoted: String` (`quote_ident(name)`) alanı döner (arayüz tırnaklı adı kullanır). Sonuç sayısı sınırı 500.
  - Önbellek: oturum başına `(istek anahtarı) → (zaman, sonuç)` 60 sn tazelik; `invalidate_completion_cache(session)` DDL ifadesi (Ddl sınıfı) başarıyla bitince çağrılır.
  - `QueryManager::export_csv(&self, session, sql: &str, path: &Path, opts: CsvOptions, progress: &mut dyn FnMut(u64) -> bool /* false = iptal */) -> Result<ExportSummary>`: **oturumun kendi bağlantısında ikinci imleçle** (`pgt_x`) akıtır; açık transaction'ı (manuel mod) **bozmaz** (autocommit'te geçici `BEGIN`/`COMMIT`); parça 5.000 satır; `BufWriter`; CSV kuralları: ayırıcı `,` (seçenek: `;`, `\t`), `"` ile çevrelenir **yalnızca** ayırıcı/`"`/CR/LF içeren alanlarda, `"` → `""`, `NULL` = boş **kaydedilmez** ayrımı: `NULL` boş alan, boş dize `""`; ilk satır başlık (seçenek); UTF-8 **BOM isteğe bağlı** (Excel için varsayılan açık); satır sonu `\r\n`; dosya `path.tmp` olarak yazılıp bittiğinde `rename` (iptal/hata → tmp silinir; mevcut dosyaya yalnızca başarıda dokunulur). `ExportSummary { rows: u64, bytes: u64, elapsed_ms: u64 }`. Yalnızca satır döndüren ifade (prepare sütunları) desteklenir, aksi halde `Error::Invalid`.

- [ ] **Step 1: Başarısız testler.**
  - Tamamlama (PG matrisi; fixture `tests/common` benzeri veritabanı oluştur): şemalar kullanıcı şemalarını verir, `pg_catalog`/`information_schema`/`pg_toast` yok; `Relations{schema:"app"}` tablo/görünüm/matview ayrı `kind`; tırnak gerektiren ad (`"Order Items"`) `quoted` alanında `"Order Items"`; kolonlar `ordinal` sırasında ve `detail` tür adı; **oturum yolu bozuk** (`set search_path = nowhere`) iken de doğru sonuç; sorgu çalışırken `complete` boş liste ve **bloke olmaz** (< 100 ms); DDL sonrası önbellek geçersiz (yeni tablo görünür); 500 sınırı.
  - CSV: `csv_quoting_rules` (saf birim: `,` `"` `\n` `\r` içeren alanlar, `NULL` vs `''`, BOM, ayırıcı seçenekleri, Unicode); entegrasyon: `export_streams_without_buffering` (`generate_series(1,200000)` → dosya satır sayısı 200001, **ilerleme geri çağrısı artan**, arka uç bellek sayacı `parça` boyutunu aşmaz), `export_cancel_leaves_no_file` (`progress` `false` döndürür → hedef dosya yok/önceki içerik duruyor, `.tmp` silindi), `export_inside_manual_transaction_keeps_it` (manuel modda export sonrası `tx_state == InTransaction` ve kullanıcı satırları duruyor), `export_rejects_non_row_statements`, hedef dizin yoksa/yazılamıyorsa `Error::Config` (yol mesajda, parola yok).
- [ ] **Step 2: Çalıştır → başarısız; uygula; geçir.**
- [ ] **Step 3: Commit.**

```bash
git add crates/pgt-query
git commit -m "feat(query): katalog tamamlama sorguları ve CSV akıtma"
```

---

### Task 8: Tauri komutları ve olay akışı (`qt_*`)

**Files:**
- Create: `src-tauri/src/query.rs`
- Modify: `src-tauri/src/{main.rs,state.rs}`, `src-tauri/Cargo.toml` (`pgt-query`), `src/ipc/commands.ts` (tipli sarmalayıcılar), `src-tauri/capabilities/default.json` (gerekirse)
- Test: `src-tauri/src/query.rs` (`#[cfg(test)]`), `src/ipc/commands.ts` tip denetimi (`bun run typecheck`)

**Interfaces:**
- Consumes: Task 6–7 `QueryManager`.
- Produces (Tauri komutları; hepsi `async`, **`Box::pin(...).await`** ile kutulu):
  - `qt_open(server_id: Uuid, database_oid: Option<u32>, options: SessionOptions) -> Uuid`
  - `qt_analyze(session: Uuid, sql: String, scope: Scope) -> Vec<StmtPlan>` (saf; ağa gitmez)
  - `qt_run(session: Uuid, sql: String, scope: Scope, limit: u32, confirmed: bool, on_event: tauri::ipc::Channel<QueryEvent>) -> ()` — `EventSink` uygulaması `Channel::send`'e köprüler (`Err` → `false`)
  - `qt_more(session: Uuid, stmt_index: u32, on_event: Channel<QueryEvent>)`, `qt_cancel(session)`, `qt_tx(session, action) -> TxState`, `qt_tx_state(session) -> TxState`, `qt_set_options(session, options)`, `qt_close(session, policy)`
  - `qt_complete(session, request) -> Vec<CompletionItem>`
  - `qt_export_csv(session, sql, path, options, on_progress: Channel<u64>) -> ExportSummary` (iptal: `qt_cancel` aynı oturumda dışa aktarmayı da keser; ilerleme kanalı kapanırsa iptal)
  - `AppState`'e `query: Arc<QueryManager>`; `main.rs`: boşta oturum süpürme görevi (60 sn'de bir `reap_idle(30 dk)`); uygulama kapanışında açık oturumlar kapatılır (açık transaction'lar **ROLLBACK** — onay arayüz katmanında, Plan 6).
  - `commands.ts`: `qtOpen/qtAnalyze/qtRun(…, onEvent: (ev: QueryEvent) => void)` (`Channel` sarmalayıcı) vb. tipli (`QueryEvent`, `StmtPlan`, `TxState`, … bağlamaları `cargo test` ile üretilir).
- Sunucu silme (`delete_server`), sunucu bağlantısını kesme (`disconnect_server`) veya `save_server` ile hedef değişimi: ilgili sunucunun **tüm sorgu oturumları kapatılır** (açık transaction'lar geri alınır) ve olay kanallarına `Failed{error: connection_lost}` ya da kapanış bilgisi gider; kilit alanı değişirse (`save_server` ile `read_only_lock` değişimi) o sunucunun oturumları kapatılır (Plan 6 yeniden bağlanmayı sunar) — spec §3.

- [ ] **Step 1: Başarısız testler (`query.rs`).** Köprü birimleri (Channel'sız): `ChannelSink` yerine test sink'iyle `qt_run` çekirdek fonksiyonunun (`run_with_sink`) hata eşlemesi: `Error::Invalid("confirmation_required")` → `ErrorPayload{code:"invalid"}`; `delete_server_closes_query_sessions` (kayıt silinince `QueryManager` oturum sayısı 0 — `delete_registered` yanında `close_sessions_of(server)` çağrılır; birim: sahte yönetici sayacı veya gerçek yönetici + Docker'sız yolla `NotConnected` hatasında); `commands_registered` (generate_handler listesinde hepsi) derleme testi; **yayın yığın güvenliği:** `qt_run`/`qt_complete`/`qt_export_csv` gelecekleri `Box::pin` ile (kod incelemesi + Task 9'daki duman testi).
- [ ] **Step 2: Uygula.** `commands.rs`'teki `delete_server` ve `disconnect_server` ve `save_server` akışlarına `state.query.close_sessions_of(server_id)` çağrısını ekleyin (`save_server`'da yalnızca hedef/kilit değiştiyse). `Scope`, `StmtPlan`, `TxState`, `SessionOptions`, `CompletionRequest`/`CompletionItem`, `CsvOptions`, `ExportSummary`, `QueryEvent` hepsi `#[ts(export)]` (bağlamalar commit'e).
- [ ] **Step 3: Doğrula ve commit.** `cargo test --workspace --all-features` (Docker'lı PG), `bun run typecheck`.

```bash
git add src-tauri src/ipc crates/pgt-query
git commit -m "feat(tauri): qt_* sorgu komutları, Channel olay akışı ve oturum yaşam döngüsü köprüleri"
```

---

### Task 9: Uçtan uca doğrulama, bellek sözleşmesi ve belgeler

**Files:**
- Create: `crates/pgt-query/tests/memory.rs`
- Modify: `scripts/smoke-release.ps1` (Query komutları kayıtlı mı: yalnızca açılış duman testi değişmez; ek kontrol gerekmiyorsa dokunma), `docs/testing.md`, `docs/performance.md`, `docs/superpowers/specs/2026-10-06-query-tool-design.md` (yalnızca "Durum" satırı: "Arka uç uygulandı (Plan 5); arayüz Plan 6")
- Test: `crates/pgt-query/tests/memory.rs`

**Interfaces:** yok (doğrulama görevi).

- [ ] **Step 1: Bellek sözleşmesi testi.** `row_budget_holds_for_one_million_rows`: `select * from generate_series(1, 1000000)` `limit=10000, chunk=1000` ile çalıştırılır; test sink'i **gelen satır sayısını ve her `Rows` olayının boyutunu** sayar: toplam **tam 10.000**, hiçbir `Rows` olayı 1.000'den büyük değil; `more()` 3 kez çağrılır → toplam 13.000 (her seferinde ≤ 1.000); ifade bitmeden `qt_cancel`/`close` imleci kapatır ve oturum `idle in transaction` kalmaz; `export_csv` aynı sorguyu 1M satır akıtır (`#[ignore]`siz ama `-- --skip` ile hızlı koşulabilir: `PGT_QUERY_BIG=1` ortam değişkeniyle kapılı) ve `progress` monoton artar. Süre bütçesi sert değil (flaky olmasın): yalnızca satır/bellek sayaçları assert edilir; ölçülen süreler `docs/performance.md`'ye yazılır.
- [ ] **Step 2: PG 10–18 matrisi.** `scripts/test-matrix.ps1 -SkipPerf` (9 sürüm) — yeni `pgt-query` entegrasyon testleri `PGT_TEST_URLS`'i kullanır; hepsi geçmeli. Sürüme özgü sapma çıkarsa (ör. `generate_series`, `DECLARE … CURSOR` mesajları, `42P11` kodu, `pg_current_xact_id` yok) **kök nedenini** sürüm kapılamasıyla düzeltin ve raporlayın.
- [ ] **Step 3: Yayın duman testi.** `bun run build`, `cargo build --release -p pg-tools`, `scripts/smoke-release.ps1` → 0 (yeni komutlar ve `futures-util` yayın yığınında çökme yaratmamalı). `scripts/measure.ps1 -Runs 3` boşta toplam özel çalışma kümesi < 150 MB (Query Tool arayüzü henüz yok; hedef korunuyor mu doğrulaması), Rust süreci < 30 MB. Aşılırsa `docs/performance.md`'deki karar kuralını uygulayın; kuralda olmayan bir iyileştirme gerekiyorsa görevi `BLOCKED` raporlayın.
- [ ] **Step 4: Belgeler.** `docs/testing.md`: `pgt-query` testleri (Docker'lı PG ile çalıştırma), `PGT_QUERY_BIG`, transaction durum makinesinin sınırları, kilit/onay kapısının arka uçta zorunlu olduğu. `docs/performance.md`: Plan 5 ölçümleri (1M satır akış sayaçları, CSV hızı, tamamlama gecikmesi, iptal süresi), `futures-util` ve `pgt-query` eklenmesinin exe boyutuna etkisi (öncesi/sonrası).
- [ ] **Step 5: Tam doğrulama ve commit.** `cargo test --workspace --all-features` (Docker'lı PG), `bun run test`, `bun run typecheck`, `bun run build`; `git checkout -- src/ipc/bindings` (yalnızca satır sonu farkları).

```bash
git add crates docs scripts
git commit -m "test(query): bellek sözleşmesi, PG 10–18 doğrulaması ve Plan 5 belgeleri"
```
