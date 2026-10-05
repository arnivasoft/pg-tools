# Object Explorer — Plan 4: Kalan İşler (pgbouncer, TLS zincir testi, ertelenen minörler) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Plan 1–3'ten devreden işleri kapatmak: (1) pgbouncer gibi `options` başlangıç parametresini reddeden arkalarda bağlanabilmek, (2) `verify-ca` / `verify-full` / `require` TLS davranışını gerçek sertifika zinciriyle (Docker'da özel CA) entegrasyon testine bağlamak, (3) ledger'daki ertelenen minör bulguları toplu temizlemek.

**Architecture:** (1) `pgt-core::connection` ilk bağlantıyı `options` ile dener; sunucu `options` parametresini açıkça reddederse bir kez `options`'sız yeniden bağlanır ve aynı üç oturum ayarını `SET` ile uygular (salt-okunurluk sözleşmesi korunur). (2) `scripts/tls-test.ps1` Docker'da bir CA + sunucu sertifikası üretir, `ssl=on` bir PostgreSQL açar ve `crates/pgt-core/tests/tls.rs` entegrasyon testini çalıştırır; yeni cargo/npm bağımlılığı yoktur (sertifikalar kapsayıcıdaki `openssl` ile üretilir). (3) Minörler küçük, bağımsız commit'ler olarak tek görevde toplanır.

**Tech Stack:** Plan 1–3 ile aynı (Rust 2024, tokio-postgres + rustls/ring, Tauri 2.12, SolidJS, Vitest, Docker, PowerShell).

**Spec:** `docs/superpowers/specs/2026-10-05-object-explorer-design.md` (§4.3 TLS, §4.4 bağlantı modeli, §7.2, §1.3). **Önceki planlar:** P1, P2, P3 — hepsi `main`'de.

## Global Constraints

- Hedef OS: Windows 10/11, Server 2019/2022; PostgreSQL 10–18 vanilla. Her oturum salt okunur kalır: `default_transaction_read_only = on`, `statement_timeout = 30000`, `search_path = pg_catalog` (bu üçü her koşulda uygulanmış olmalı; `options` yolu olmasa bile).
- TLS: rustls + ring (`aws-lc-rs` açılmaz). `sslmode` anlamları spec §4.3 tablosundaki gibidir; `verify-ca` host adını doğrulamaz, `verify-full` doğrular.
- Parola/bağlantı dizgisi hiçbir log, rapor, commit veya hata metninde yer almaz. Test sertifika anahtarları yalnızca geçici dizinde yaşar, repoya yazılmaz.
- Yeni bağımlılık eklenmez. UI metinleri `src/app/strings.ts`'tedir.
- Katalog/bağlantı gelecekleri `Box::pin` ile kutulanır (release ana yığın).
- Docker yoksa TLS ve matris adımları `BLOCKED` raporlanır, uydurulmaz.

### Bu planın bilinçli sınırları

- Kapsam dışı: SSH tüneli, pgpass, yeni nesne türleri, Query Tool.
- Elle arayüz kontrolü (`docs/smoke-test.md` Plan 3 listesi) bu planın parçası değildir; kullanıcıyla birlikte yürütülür.
- Transaction-pooling modunda `SET` oturum ayarları havuz tarafından kaybedilebilir; bu sınır belgelenir (Task 1), çözülmez.

## Review Focus

1. **Yanlış pozitif yeniden bağlanma:** `options` yedek yolu yalnızca sunucu bu parametreyi açıkça reddettiğinde (SQLSTATE `08P01`/`0A000` benzeri ve mesajda `options`/`startup parameter`) devreye girmeli; parola hatası, ağ hatası, TLS hatası ASLA ikinci denemeyi tetiklememeli ve hata maskelenmemeli. Test: Task 1 `non_options_errors_do_not_retry`.
2. **Salt-okunurluk kaybı:** yedek yolda `SET` başarısız olursa bağlantı kurulmuş sayılmamalı (yazma yapabilen oturum sızmamalı). Test: Task 1 `fallback_apply_failure_is_connect_error`.
3. **TLS yanlış kabul:** güvenilmeyen CA ile `verify-ca`/`verify-full` bağlanmamalı; `verify-full` host adı uyuşmazlığında bağlanmamalı; `require` sertifikayı doğrulamadan bağlanmalı; `disable` ile `hostssl` sunucuya bağlanmamalı. Test: Task 2 tablosu.
4. **Minör temizlikte regresyon:** toast zamanlayıcıları temizlenirken çift kapama/sızıntı olmamalı; filtre açıkken fareyle aç/kapa `expanded` kümesini değiştirmemeli. Test: Task 3.

## Dosya Haritası

```
crates/pgt-core/src/connection.rs      options yedek yolu (Task 1)
crates/pgt-core/tests/connection.rs    yedek yol birim/entegrasyon testleri (Task 1)
scripts/tls-test.ps1                   Docker'lı TLS test ortamı (Task 2)
crates/pgt-core/tests/tls.rs           TLS zincir testi (Task 2)
docs/testing.md                        TLS ve pgbouncer notları (Task 1, 2)
src/app/{toast,Toaster}.ts(x), src/features/explorer/{TreeRow,TreeView}.tsx, scripts/{smoke-release,measure}.ps1, ...   (Task 3)
```

---

### Task 1: `options` başlangıç parametresi yedek yolu (pgbouncer)

**Files:**
- Modify: `crates/pgt-core/src/connection.rs`
- Test: `crates/pgt-core/src/connection.rs` (`#[cfg(test)]`), `crates/pgt-core/tests/connection.rs`
- Modify: `docs/testing.md`

**Interfaces:**
- Consumes: `SESSION_OPTIONS`, `ConnectionManager::connect/test`, `Error::Connect{kind: ..}`.
- Produces: saf `fn rejects_options(err: &tokio_postgres::Error) -> bool`; `const SESSION_SETS: [&str; 3]` (`SET default_transaction_read_only = on`, `SET statement_timeout = 30000`, `SET search_path = pg_catalog`); bağlantı kurucuda ilk deneme `options` ile, `rejects_options` doğruysa bir kez `options`'sız + `SESSION_SETS` ile.

- [ ] **Step 1: Başarısız testler.** `rejects_options` için tablo testi: pgbouncer'ın gerçek mesajı `unsupported startup parameter: options` (SQLSTATE `08P01`) → `true`; `password authentication failed` (`28P01`), `connection refused`, TLS hatası, `database "x" does not exist` (`3D000`) → `false`. `tokio_postgres::Error` doğrudan kurulamadığı için hata sınıflandırmasını `fn rejects_options_message(sqlstate: Option<&str>, message: &str) -> bool` çekirdeğine ayırın ve onu test edin (`Error` sarmalayıcısı çekirdeği çağırır). `SESSION_SETS` testi: üç ayar da `SESSION_OPTIONS` ile aynı değerleri taşır (iki sabit birbirinden sapmasın diye `-c k=v` ayrıştırıp `SET k = v` ile karşılaştırın).
- [ ] **Step 2: Çalıştır → başarısız.** `cargo test -p pgt-core connection::`
- [ ] **Step 3: Uygula.** Bağlantı kurucuyu (`connect_once(config, password, with_options: bool)`) ikiye bölün; `with_options=false` yolu `options` yerine bağlantı kurulduktan hemen sonra `SESSION_SETS`'i tek `batch_execute` ile çalıştırır; herhangi biri başarısız olursa istemci kapatılır ve `Error::Connect` döner (Review Focus 2). Ana yol yalnızca `rejects_options` doğruysa yedek yolu dener; ikinci denemenin hatası ilk hatanın yerine döner. Yedek yola düşüldüğünde `tracing::info!` ile (parolasız, host/port'suz) tek satır günlük yazılır.
- [ ] **Step 4: Entegrasyon testi.** Gerçek pgbouncer gerekmez: `session_settings_are_applied` testinin yedek yol eşleniğini ekleyin — `connect_once(.., with_options=false)` ile bağlanıp üç ayarın (`SHOW`) uygulandığını ve yazmanın (`CREATE TEMP TABLE`) reddedildiğini doğrulayın (PGT_TEST_URLS'li tüm sürümler).
- [ ] **Step 5: Belge + doğrulama + commit.** `docs/testing.md`'ye pgbouncer notu (yedek yol, transaction-pooling sınırı). `cargo test --workspace --all-features`, `bun run typecheck`.

```bash
git add crates/pgt-core docs/testing.md
git commit -m "feat(core): options parametresini reddeden sunucular (pgbouncer) için SET yedek yolu"
```

---

### Task 2: TLS zincir entegrasyon testi (verify-ca / verify-full / require / disable)

**Files:**
- Create: `scripts/tls-test.ps1`, `crates/pgt-core/tests/tls.rs`
- Modify: `docs/testing.md`

**Interfaces:**
- Consumes: `ServerConfig`/`SslConfig`/`SslMode`, `ConnectionManager::test(&config, Some(password))`, `Error::Connect{kind: Tls|Auth|Network}`.
- Produces: `scripts/tls-test.ps1 [-Version 16] [-KeepContainer]`: kapsayıcıda `openssl` ile CA + sunucu sertifikası (CN=`pgt-tls.test`, SAN yalnızca bu ad; **IP yok**) üretir, `ssl=on` + `hostssl` yalnızca-TLS `pg_hba` ile bir PostgreSQL açar, sertifikaları geçici dizine çıkarır ve `PGT_TLS_URL=host=127.0.0.1 port=… user=… (parola ortam değişkeninde)`, `PGT_TLS_CA=<ca.pem>`, `PGT_TLS_OTHER_CA=<ayrı bir CA'nın pem'i>` ortam değişkenleriyle `cargo test -p pgt-core --test tls` çalıştırır; sonunda kapsayıcıyı ve geçici dizini siler. Ortam değişkenleri yoksa test atlanır.

- [ ] **Step 1: Test tablosunu yaz** (`tls.rs`; her satır bir `#[tokio::test]`, ortak yardımcı yapılandırmayı kurar). Sunucuya `127.0.0.1` ile bağlanılır, sertifika yalnızca `pgt-tls.test` adını taşır:

| mod | root_cert | beklenen |
|---|---|---|
| `disable` | – | hata (sunucu yalnızca `hostssl` kabul eder) |
| `prefer` | – | başarılı (TLS kurulur) |
| `require` | – | başarılı (sertifika doğrulanmaz) |
| `verify-ca` | doğru CA | **başarılı** (host adı uyuşmazlığı yok sayılır) |
| `verify-ca` | başka CA (`PGT_TLS_OTHER_CA`) | hata, `kind = Tls` |
| `verify-ca` | yok (Windows deposu) | hata, `kind = Tls` (özel CA depoda yok) |
| `verify-full` | doğru CA | **hata** (IP/host adı SAN'da yok), `kind = Tls` |
| `verify-full` | doğru CA, `host=pgt-tls.test` (hosts girdisi gerekmeden: `hostaddr` desteği yoksa bu satır atlanır ve nedeni testte yorumlanır) | başarılı |

İstemci sertifikası senaryosu kapsam dışıdır (spec'te var ama TLS'li test sunucusu bu planda yalnızca sunucu doğrulamasını sınar).
- [ ] **Step 2: Betiği yaz.** Kapsayıcıda `openssl req -x509 -newkey rsa:2048 -nodes` ile CA, ardından sunucu anahtarı + CSR + `-extfile` ile SAN (`DNS:pgt-tls.test`) imzalama; ikinci bir bağımsız CA; `docker cp` ile geçici dizine al; `postgres -c ssl=on -c ssl_cert_file=… -c ssl_key_file=…` (anahtar `chmod 600`, sahibi `postgres`); `pg_hba` yalnızca `hostssl all all all scram-sha-256`. Parola rastgele üretilir ve yalnızca ortam değişkeninde yaşar. Hata durumunda kapsayıcı yine silinir (`finally`).
- [ ] **Step 3: Testi çalıştır.** `powershell -NoProfile -File scripts/tls-test.ps1` → tablo satırlarının hepsi PASS. Bir satır beklenenden farklıysa **önce** tablonun varsayımını (ör. rustls'in SAN'sız IP'ye davranışı) doğrulayın; kod hatasıysa `tls.rs` (kaynak: `crates/pgt-core/src/tls.rs`) içinde kök nedenden düzeltin ve düzeltmeyi raporlayın.
- [ ] **Step 4: Belge + commit.** `docs/testing.md`'ye `tls-test.ps1` kullanımı ve tablo.

```bash
git add scripts crates/pgt-core/tests/tls.rs docs/testing.md
git commit -m "test(core): Docker'lı TLS zincir testi (verify-ca/verify-full/require/disable)"
```

---

### Task 3: Ertelenen minörlerin toplu temizliği

**Files:** aşağıdaki kalemlerin dokunduğu dosyalar (her kalem ayrı commit).
**Interfaces:** yok (yalnızca davranış düzeltmeleri; kamuya açık imza değişmez).

Kalemler (her biri küçük, bağımsız; bir kalem kapsam dışı çıkarsa `DONE_WITH_CONCERNS` ile atlanıp nedeni raporlanır):

- [ ] **A. Toast:** `toast.ts` zamanlayıcıları `Map<id, timer>` ile tutulup elle kapatmada/tahliyede temizlenir; `Toaster.tsx` kalıcı `aria-live="polite"` bölgesi içinde render edilir; aynı metin+tür art arda gelirse tekilleştirilir (sayaç yenilenir). Test: `toast-model.test.ts` (tekilleştirme, tahliye).
- [ ] **B. Filtre + fare:** süzülmüş görünümde `TreeRow` çift tıklama/ok tıklaması `controller.toggle`'ı çağırmaz; `TreeView` `aria-activedescendant` yalnızca görünür satıra işaret eder. Test: tree-model/controller testi (`filter` etkinken `toggle` `expanded`'ı değiştirmez).
- [ ] **C. Rozet odağı:** `ErrorBadge` tıklaması sonrası odak ağaçta kalır (P3 düzeltmesi var; kalan: kart içindeki düğme Tab ile erişilebilir olsun — `HoverCard` içeriğine `tabIndex` gerektirmeden Enter ile aynı eylem yeterliyse bu kalemi atlayıp gerekçesini yaz).
- [ ] **D. `reloadAll` testleri:** "parents first" testi derin düğümün sığdan önce yüklendiği bir durumla; "disconnected" testi `disconnect` yerine alt listesi hâlâ duran bağlantısız bir durumla (state'i doğrudan kurarak); "continues after failing load" try/catch'siz kodda kırılacak biçimde sertleştirilir.
- [ ] **E. İçe aktarma:** `ImportDialog` uyarıları yalnızca `title` ile değil görünür liste olarak (genişletilebilir satır) gösterir; `import.rs` `apply` aynı dosya içindeki yinelenenleri (host+port+kullanıcı+ad) ikinci kez eklemez; `passwords_in_the_file_are_never_read` testi `ServerConfig`'i de döker; UTF-16 dosya için anlaşılır hata mesajı. Testler: `import.rs`.
- [ ] **F. Betikler:** `smoke-release.ps1` süreç ağacını ana süreç ölmüş olsa bile (ad+üst süreç kimliğiyle) öldürür ve geçici dizin temizliği `finally` içinde yapılır; `measure.ps1` UTF-8 BOM ile kaydedilir (PS 5.1 Türkçe çıktısı); `docs/performance.md` MB/MiB tutarlı yazılır.
- [ ] **G. Komut/çekirdek:** `delete_registered` salt-okunur testi belirli hata türünü doğrular (`Error::Config`); `set_show_system` için küçük birim testi (`Catalog` bayrağı); P3 son incelemesinin artakalanı: `creds.remember` ile kayıt-silme arasındaki dar yarış için `remember_after_connect` kaydı kilit altında yeniden doğrular.
- [ ] **Doğrulama:** `cargo test --workspace --all-features` (PGT_TEST_URLS ile), `bun run test`, `bun run typecheck`, `bun run build`; `scripts/test-matrix.ps1 -SkipPerf` bir kez.

```bash
git commit -m "chore: ertelenen minörlerin temizliği (<kalem>)"   # kalem başına ayrı commit
```
