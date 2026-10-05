# Object Explorer — Plan 3/3: pgAdmin İçe Aktarma, Filtre, Menü, Hata Gösterimi ve Kapanış Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Object Explorer'ı günlük kullanıma hazır hale getirmek: pgAdmin `servers.json` içe aktarma (§4.5), yüklü düğümlerde filtre (§6.2), sağ tık menüsü + kopyalama eylemleri + sunucu silme/bağlantıyı kesme (§6.2), toast bildirimleri ve düğüm hata rozeti/"Tekrar dene"/`NotFound` akışı (§7.2), sistem nesnelerini göster ayarı, durum çubuğu iyileştirmeleri; ardından açılış/bellek/paket hedeflerini yayın (release) exe üzerinde yeniden doğrulamak ve PG 13/15'i sürüm matrisine eklemek.

**Architecture:** Arka uçta `pgt-core` içine saf bir `import` modülü (JSON ayrıştırma + yinelenen tespiti, dosya G/Ç'sinden ayrı) ve üç küçük Tauri komutu (`delete_server`, `set_show_system`, `quote_identifiers`) eklenir; katalog sorgu mimarisi değişmez. Ön yüzde yeni mantık **saf, test edilebilir modüllerde** yaşar (`tree-model.ts`'e `filterRows`, `actions.ts`, `toast-model.ts`, `import-model.ts`); Solid bileşenleri bu modülleri ince bir kabukla bağlar. Filtre, `TreeState.filter` alanı üzerinden `flatten()`'e girer; böylece klavye gezintisi ve sanal liste ek kod olmadan süzülmüş satırlarla çalışır.

**Tech Stack:** Plan 1/2 ile aynı (Rust 2024, Tauri 2.12, tokio-postgres, ts-rs, insta; bun, SolidJS, Kobalte 0.13, Tailwind 4, Vitest). Yeni bağımlılık eklenmez (Kobalte `context-menu` ve `hover-card` zaten `@kobalte/core` içindedir; `@tauri-apps/plugin-dialog` zaten var).

**Spec:** `docs/superpowers/specs/2026-10-05-object-explorer-design.md` (§3.3 komutlar, §4.5 içe aktarma, §6.1–6.3 arayüz, §7.2 gösterim, §1.3 başarı ölçütleri)

**Önceki planlar:** `docs/superpowers/plans/2026-10-05-object-explorer-p1-cekirdek.md`, `…-p2-nesne-turleri.md` — ikisi de tamamlandı ve `main`'e birleşti.

## Plan 2 sonrası kod gerçekleri (bu planın dayandığı yapı)

- Tauri komutları `src-tauri/src/commands.rs` içindedir ve `main.rs` `generate_handler!` listesine eklenir. Katalog gelecekleri büyüktür; **katalog/bağlantı `await`'leri `Box::pin` ile kutulanır** (release exe ana yığın taşması, Plan 2 Task 10). Yeni komutlarda da aynı kural: `Box::pin(...).await`.
- `AppState { registry: Arc<RwLock<Registry>>, credentials: Credentials, manager: Arc<ConnectionManager>, catalog: Catalog, startup_notice }` (`src-tauri/src/state.rs`). `Catalog::set_show_system(bool)` zaten vardır (`crates/pgt-catalog/src/lib.rs`); henüz komut olarak açılmamıştır. `show_system` yalnızca şema, rol ve tablespace listelerini etkiler.
- `Registry` (`crates/pgt-core/src/registry.rs`): `servers()`, `get(id)`, `upsert(cfg)` (normalize + doğrula + atomik kaydet), `remove(id) -> Result<Option<ServerConfig>>`. Dosya okunamadıysa salt-okunurdur (`is_read_only()`), yazma `Error::Config` döner. `Credentials::forget(id)` kayıtlı ve oturum parolasını siler. **`delete_server` komutu henüz yoktur** (spec §3.3 listeler).
- `ServerConfig { id, name, group, host, port, maintenance_db, username, save_password, ssl: SslConfig{mode: SslMode, root_cert, client_cert, client_key} }`; `SslMode` kebab-case (`disable|prefer|require|verify-ca|verify-full`). `ServerConfig::new(name, host, port, username)`, `.normalized()`, `.validate()`.
- `ConnectionManager::run(server, Target, |c| …)` ile sorgu çalışır (`Target::Maintenance | Target::Database(oid)`); katalog tarafında `crates/pgt-catalog/src/db.rs` içindeki `query(&cat.mgr, server, target, &VersionedSql, p![…])` yardımcısı kullanılır; `VersionedSql(&[(min_ana_sürüm, sql)])`.
- Ön yüz ağaç durumu: `src/features/explorer/tree-model.ts` (`TreeState { nodes, parent, children, errors, loading, expanded, selected }`, saf fonksiyonlar `setChildren/setError/patchNode/expand/collapse/select/invalidate/flatten/keyIntent`, `serverOf`, `rowLabel`, `ROOT = ""`). `controller.ts` (`ExplorerController`: `load`, `ensureConnected`, `expand`, `collapse`, `toggle`, `select`, `refresh`, `reloadServers`, `handleKey`, `sessionLost`; `isSessionGone`). `explorer-store.ts` `createExplorer(deps)` → `{ state, rows, selectedNode, controller }`. Test kalıbı için `controller.test.ts` ve `tree-model.test.ts`'e bakın (sahte `api` ile).
- `TreeRow.tsx` hata rozetini şimdilik yalnızca `title` ile gösterir; `StatusBar.tsx` `notice` (başlangıç bildirimi) hiç temizlenmez; `ServerDialog`/`PasswordPrompt` açılışta yüklenir (tembel değil); durum çubuğunda veritabanı adı yoktur. Toolbar `Toolbar.tsx` + `App.tsx` içinde birleşir. `src/ui/{Button,Dialog,Field,Tabs}.tsx` ortak kabuklardır (`Dialog` Kobalte tabanlı).
- Hata kodları (`ErrorPayload.code`): `not_connected`, `connection_lost`, `password_required`, `auth`, `config`, `invalid`, `not_found`, `query`, … (`crates/pgt-core/src/error.rs`). `NotFound` ön yüze `not_found` koduyla gelir.
- Pano: `navigator.clipboard.writeText` (`Inspector.tsx`). Ortak yardımcıya çıkarılır (Task 6).
- `localStorage` yalnızca `src/app/storage.ts` (`readStorage`/`writeStorage`) üzerinden kullanılır.
- Test altyapısı: `PGT_TEST_URLS` (virgüllü; yoksa entegrasyon testleri atlanır), `crates/pgt-catalog/tests/common/mod.rs` (`fixtures()` → `Fixture`), Vitest `src/**/*.test.ts` (node ortamı: DOM yok → bileşen mantığı saf modüllerde test edilir). `scripts/test-matrix.ps1`, `scripts/measure.ps1`. ts-rs bağlamaları `src/ipc/bindings` altına `cargo test` ile üretilir (`.cargo/config.toml` `TS_RS_EXPORT_DIR`); yeni `#[ts(export)]` tipleri için üretilen dosya commit'e eklenir. Derleme/test sonrası yalnızca satır sonu farkıyla değişen bağlama dosyaları `git checkout -- src/ipc/bindings` ile geri alınır.

## Global Constraints

- Hedef istemci OS: Windows 10/11, Windows Server 2019/2022. PostgreSQL 10–18, yalnızca vanilla. Hiçbir görev PG sürümüne özel SQL gerektirmez; `quote_identifiers` tüm sürümlerde geçerlidir.
- Her PostgreSQL oturumu salt okunur (`default_transaction_read_only = on`); sorgular yalnızca sabit SQL'dir, değerler `$n` parametresiyle bağlanır; arayüze rastgele SQL açılmaz. Plan 3 de salt okunurdur: **hiçbir görev PostgreSQL'e yazmaz**; "sil" yalnızca pg-tools'un sunucu kaydını siler.
- **Tırnaklama sunucuda yapılır** (`quote_ident`); ön yüzde ve Rust'ta rezerve kelime listesi/tırnaklama kuralı yazılmaz. Nitelikli ad kopyalama bu yüzden `quote_identifiers` komutunu kullanır.
- Parola hiçbir dosyaya, log'a, toast'a veya hata metnine yazılmaz; pgAdmin `servers.json` dosyasındaki hiçbir parola alanı okunmaz/saklanmaz. Test kimlik bilgileri (`PGT_TEST_URLS`) hiçbir rapora, commit'e veya log'a yazılmaz.
- Arayüz metinleri `src/app/strings.ts` içindedir (Türkçe komutlar, İngilizce PostgreSQL terimleri); bileşenlerde sabit Türkçe metin bırakılmaz.
- Rutin hatalar için **modal diyalog açılmaz** (§7.2): hata rozeti/toast/panel içi gösterim. Onay diyalogları yalnızca yıkıcı eylem içindir (sunucu silme).
- Katalog/bağlantı gelecekleri `Box::pin` ile kutulanır (yukarıya bakın).
- Paket yöneticisi bun (`~/.bun/bin/bun`, PATH'te olmayabilir). TLS kripto sağlayıcısı ring; `aws-lc-rs` açılmaz. Yeni npm/cargo bağımlılığı eklenmez.
- Başarı ölçütleri (§1.3) bu plan sonunda yayın exe'sinde yeniden ölçülür: soğuk açılış < 1 sn, boşta toplam özel çalışma kümesi < 150 MB (Rust süreci < 30 MB), 10.000 yüklü düğümde filtre < 50 ms.

### Bu planın bilinçli sınırları

- Kapsam dışı (sonraki spec'ler): nesne oluşturma/düzenleme/silme (PostgreSQL tarafında), Query Tool, veri ızgarası, Dashboard.
- Kapsam dışı (Plan 1'den devreden, ayrı iş): `verify-ca` için gerçek sertifika zinciriyle TLS testi (TLS'li test sunucusu gerektirir), pgbouncer için `options` başlangıç parametresi, kurulum paketinin dağıtımı (yalnızca boyut ölçülür, Task 10).
- Bilinen sınır: filtre yalnızca yüklenmiş düğümlerde çalışır (§6.2); kapalı/yüklenmemiş koleksiyonlar taranmaz.

## Review Focus

1. **Filtre ve Türkçe harfler/özel karakterler:** `İ`/`ı`, `Ç`, boşluk, `.*(`, `%`, `\` içeren sorgular düz alt dize olarak eşleşmeli (regex değil), büyük/küçük harf duyarsızlığı `toLowerCase()` ile tutarlı olmalı; boş/yalnız boşluk sorgusu süzmemeli. Test: Task 5 `treats the query as a plain substring`, `blank query does not filter`.
2. **İçe aktarma dosyası bozuk/sıra dışı:** BOM'lu UTF-8, JSON olmayan dosya, `Servers` yok, port sayı/metin/eksik/aralık dışı, `Host`/`Username` eksik, `ConnectionParameters` ile düz alanların birlikte bulunması, 10'dan fazla kayıt (anahtar sırası `"10"` < `"2"` olmamalı), çok büyük dosya. Test: Task 2 tabloları.
3. **Silinen sunucu bağlıyken/seçiliyken/genişletilmişken:** bağlantı kapanmalı, kayıtlı parola silinmeli, ağaç ve seçim temizlenmeli, Properties paneli boşalmalı; kayıt defteri salt-okunursa hata toast'u çıkmalı ve kayıt silinmemeli. Test: Task 1 `delete_server_*`, Task 6 `removed server clears the selection`.
4. **Tırnak gerektiren ve fonksiyon adları:** `"Mixed View"`, `user` (rezerve), içinde `"` olan ad, `fmt(integer)` imzası için nitelikli ad doğru üretilmeli; şema bilgisi güvenilmeyen düğümlerde (partition, constraint/trigger/policy) yanlış ad yerine eylem gizlenmeli. Test: Task 1 `quote_identifiers_*`, Task 6 `qualifiedParts` tabloları.
5. **Eskimiş kimlik:** menü açıkken veya eylem sürerken düğüm yenilemeyle kaybolursa eylem sessizce yanlış nesneye uygulanmamalı ("nesne artık mevcut değil" toast'u). Test: Task 6 `action targets are re-read at execution time`.
6. **Sistem nesnesi anahtarı ve bağlantısız sunucu:** anahtar değişince yalnızca bağlı sunucuların yüklenmiş listeleri yenilenmeli; bağlantısız sunucu için hata rozeti çıkmamalı; seçim kaybolmamalı. Test: Task 8 `reloadAll skips disconnected servers`.

## Dosya Haritası (bu plan)

```
crates/pgt-core/src/import.rs            pgAdmin servers.json ayrıştırma/önizleme/uygulama (Task 2)
crates/pgt-catalog/src/quote.rs          Catalog::quote_identifiers (Task 1)
src-tauri/src/commands.rs                delete_server, set_show_system, quote_identifiers (Task 1);
                                         preview_pgadmin_import, apply_pgadmin_import (Task 2)
src/app/toast-model.ts, toast.ts, Toaster.tsx   toast kuyruğu ve görünümü (Task 3)
src/app/clipboard.ts                     pano yardımcısı (Task 6)
src/features/servers/import-model.ts, ImportDialog.tsx   içe aktarma arayüzü (Task 4)
src/features/explorer/tree-model.ts      TreeState.filter + filterRows + ancestorOfKind (Task 5)
src/features/explorer/actions.ts, ExplorerMenu.tsx, src/ui/ConfirmDialog.tsx   menü ve eylemler (Task 6)
src/features/explorer/ErrorBadge.tsx     rozet + hover kartı + Tekrar dene (Task 7)
src/app/Toolbar.tsx, App.tsx, StatusBar.tsx   filtre kutusu, içe aktar, sistem anahtarı, durum çubuğu (Task 4,5,8,9)
scripts/smoke-release.ps1, test-matrix.ps1, docs/performance.md, docs/testing.md   (Task 10)
```

---

### Task 1: Arka uç komutları — `delete_server`, `set_show_system`, `quote_identifiers`

**Files:**
- Create: `crates/pgt-catalog/src/quote.rs`
- Modify: `crates/pgt-catalog/src/lib.rs` (`mod quote;`), `src-tauri/src/commands.rs`, `src-tauri/src/main.rs`, `src/ipc/commands.ts`
- Test: `crates/pgt-catalog/tests/quote.rs`, `src-tauri/src/commands.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `Catalog.mgr` (`ConnectionManager::run`), `Registry::remove`, `Credentials::forget`, `ConnectionManager::disconnect`, `Error::UnknownServer(Uuid)`.
- Produces:
  - `impl Catalog { pub async fn quote_identifiers(&self, server: Uuid, names: Vec<String>) -> Result<Vec<String>> }` — sonuç girdiyle aynı sıra ve uzunlukta.
  - Tauri: `delete_server(server_id: Uuid) -> ()`, `set_show_system(value: bool) -> ()`, `quote_identifiers(server_id: Uuid, names: Vec<String>) -> Vec<String>`.
  - `commands.ts`: `deleteServer(serverId)`, `setShowSystem(value)`, `quoteIdentifiers(serverId, names)`.

- [ ] **Step 1: `quote_identifiers` için başarısız entegrasyon testi yaz**

`crates/pgt-catalog/tests/quote.rs` — `tests/common/mod.rs`'in `fixtures()` kalıbını `tests/listing.rs`'teki gibi kullanın (test sunucusu yoksa test atlanır). Fixture'dan sunucu kimliğini alın (mevcut testlerdeki gibi `fx.server`):

```rust
mod common;

#[tokio::test]
async fn quote_identifiers_uses_server_side_quote_ident() {
    for fx in common::fixtures().await {
        let input = ["orders", "Mixed View", "user", "a\"b", "çok", "select", "ok_1"];
        let got = fx.catalog.quote_identifiers(fx.server, input.iter().map(|s| s.to_string()).collect()).await.unwrap();
        assert_eq!(got, vec!["orders", "\"Mixed View\"", "\"user\"", "\"a\"\"b\"", "çok", "\"select\"", "ok_1"], "PG {}", fx.major());
    }
}

#[tokio::test]
async fn quote_identifiers_empty_input_is_empty_output() {
    for fx in common::fixtures().await {
        assert!(fx.catalog.quote_identifiers(fx.server, vec![]).await.unwrap().is_empty());
    }
}
```

`çok`: PostgreSQL `quote_ident` ASCII olmayan harfleri tırnaksız bırakır; test fiilî sunucu davranışını sabitler. Fixture alan adları (`catalog`, `server`) farklıysa mevcut testlerdeki gerçek adları kullanın.

- [ ] **Step 2: Testin derlenmediğini/başarısız olduğunu doğrula**

Run: `cargo test -p pgt-catalog --test quote` (`PGT_TEST_URLS` ayarlı)
Expected: FAIL (`quote_identifiers` yok)

- [ ] **Step 3: `quote.rs`'i yaz**

```rust
use pgt_core::Result;
use pgt_core::connection::Target;
use uuid::Uuid;

use crate::Catalog;

const SQL: &str = "SELECT quote_ident(u.n) AS q FROM unnest($1::text[]) WITH ORDINALITY AS u(n, i) ORDER BY u.i";

impl Catalog {
    /// Adları sunucuda `quote_ident` ile tırnaklar (Rust'ta tırnaklama kuralı yoktur). Sıra korunur.
    pub async fn quote_identifiers(&self, server: Uuid, names: Vec<String>) -> Result<Vec<String>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let rows = self
            .mgr
            .run(server, Target::Maintenance, move |c| {
                let names = names.clone();
                async move { c.client.query(SQL, &[&names]).await }
            })
            .await?;
        Ok(rows.iter().map(|r| r.get::<_, String>("q")).collect())
    }
}
```

`ConnectionManager::run`'ın kapanış imzası farklıysa (`F: Fn(Arc<PgConn>) -> Fut` vb.) `db.rs::query` içindeki kullanıma uyarlayın. `lib.rs`'e `mod quote;` ekleyin.

- [ ] **Step 4: Testi çalıştır, geçtiğini doğrula**

Run: `cargo test -p pgt-catalog --test quote`
Expected: PASS (tüm `PGT_TEST_URLS` sürümlerinde)

- [ ] **Step 5: `delete_server` için başarısız birim testleri yaz (`commands.rs` test modülü)**

`delete_server` mantığını test edilebilir bir yardımcıya çıkarın: `fn delete_registered(registry: &RwLock<Registry>, manager: &ConnectionManager, creds: &Credentials, id: Uuid) -> Result<(), Error>`. Mevcut `registry_with`, `cfg`, `MemorySecretStore` test yardımcılarını kullanın:

```rust
#[test]
fn delete_server_removes_record_and_stored_password() {
    let dir = tempfile::tempdir().unwrap();
    let config = cfg(Uuid::new_v4());
    let registry = registry_with(dir.path(), &config);
    let store = Arc::new(MemorySecretStore::default());
    let c = Credentials::new(store.clone());
    c.remember(config.id, "pw", true).unwrap();
    let manager = ConnectionManager::new();
    delete_registered(&registry, &manager, &c, config.id).unwrap();
    assert!(registry.read().unwrap().get(config.id).is_none());
    assert_eq!(c.password(config.id).unwrap(), None);
}

#[test]
fn delete_unknown_server_is_unknown_server_error() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry_with(dir.path(), &cfg(Uuid::new_v4()));
    let err = delete_registered(&registry, &ConnectionManager::new(), &creds(), Uuid::new_v4()).unwrap_err();
    assert!(matches!(err, Error::UnknownServer(_)));
}

#[test]
fn delete_server_on_read_only_registry_keeps_everything() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let registry = RwLock::new(Registry::unavailable(dir.path().join("servers.json")));
    let c = creds();
    c.remember(id, "pw", true).unwrap();
    assert!(delete_registered(&registry, &ConnectionManager::new(), &c, id).is_err());
    assert_eq!(c.password(id).unwrap().as_deref(), Some("pw"), "kayıt silinemediyse parola da silinmemeli");
}
```

Önemli sıra: önce `registry.remove` (başarısızsa hiçbir şeye dokunulmaz), sonra `manager.disconnect`, sonra `creds.forget`. Kayıt yoksa `Error::UnknownServer(id)`.

- [ ] **Step 6: Çalıştır → başarısız; yardımcıyı ve komutları yaz**

```rust
fn delete_registered(registry: &RwLock<Registry>, manager: &ConnectionManager, creds: &Credentials, id: Uuid) -> Result<(), Error> {
    let removed = registry.write().unwrap().remove(id)?;
    if removed.is_none() {
        return Err(Error::UnknownServer(id));
    }
    manager.disconnect(id);
    creds.forget(id)
}

#[tauri::command]
pub fn delete_server(state: State<'_, AppState>, server_id: Uuid) -> CmdResult<()> {
    Ok(delete_registered(&state.registry, &state.manager, &state.credentials, server_id)?)
}

#[tauri::command]
pub fn set_show_system(state: State<'_, AppState>, value: bool) {
    state.catalog.set_show_system(value);
}

#[tauri::command]
pub async fn quote_identifiers(state: State<'_, AppState>, server_id: Uuid, names: Vec<String>) -> CmdResult<Vec<String>> {
    Ok(Box::pin(state.catalog.quote_identifiers(server_id, names)).await?)
}
```

`main.rs` `generate_handler!`'a üçünü ekleyin. `commands.ts`'e ekleyin:

```ts
  deleteServer: (serverId: string) => call<null>("delete_server", { serverId }),
  setShowSystem: (value: boolean) => call<null>("set_show_system", { value }),
  quoteIdentifiers: (serverId: string, names: string[]) => call<string[]>("quote_identifiers", { serverId, names }),
```

- [ ] **Step 7: Doğrula ve commit**

Run: `cargo test --workspace --all-features` ve `bun run typecheck`
Expected: PASS

```bash
git add crates/pgt-catalog src-tauri src/ipc/commands.ts
git commit -m "feat: delete_server, set_show_system ve quote_identifiers komutları"
```

---

### Task 2: pgAdmin içe aktarma — arka uç (`pgt-core::import`)

**Files:**
- Create: `crates/pgt-core/src/import.rs`
- Modify: `crates/pgt-core/src/lib.rs` (`pub mod import;`), `src-tauri/src/commands.rs`, `src-tauri/src/main.rs`, `src/ipc/commands.ts`
- Test: `crates/pgt-core/src/import.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `ServerConfig`, `SslMode`, `Registry::{servers, upsert}`, `Error::{Invalid, Config}`.
- Produces:
  - `pub struct ImportItem { index: u32, name: String, group: Option<String>, host: String, port: u16, username: String, duplicate: bool, warnings: Vec<String>, error: Option<String> }` (`Serialize + TS`, camelCase, `#[ts(export)]`).
  - `pub fn parse(text: &str, existing: &[ServerConfig]) -> Result<Vec<Parsed>>`, `pub struct Parsed { pub item: ImportItem, pub config: Option<ServerConfig> }`.
  - `pub fn preview(text: &str, existing: &[ServerConfig]) -> Result<Vec<ImportItem>>`; `pub fn apply(registry: &mut Registry, text: &str, indices: &[u32]) -> Result<u32>`.
  - Tauri: `preview_pgadmin_import(path: String) -> Vec<ImportItem>`, `apply_pgadmin_import(path: String, indices: Vec<u32>) -> u32`. `commands.ts`: `previewPgadminImport(path)`, `applyPgadminImport(path, indices)`.

pgAdmin biçimi (özet): kök `{"Servers": {"1": {…}, "2": {…}}}`; kayıt alanları `Name, Group, Host, Port, MaintenanceDB, Username, SSLMode, SSLRootCert, SSLCert, SSLKey, PassFile, Service, UseSSHTunnel, ConnectionParameters{sslmode, sslrootcert, sslcert, sslkey, passfile}`. pgAdmin 7+ SSL alanlarını `ConnectionParameters` içine koyar; eski biçim düz alanlardır. **Önce `ConnectionParameters`, yoksa düz alan** kullanılır.

- [ ] **Step 1: Başarısız testleri yaz (tablo odaklı)**

`import.rs` test modülü; JSON'lar satır içi sabit. Her test bir Review Focus 2 sınıfını sabitler:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = r#"{"Servers":{"1":{"Name":"prod","Group":"Üretim","Host":"db1","Port":5433,"MaintenanceDB":"app","Username":"u","SSLMode":"verify-full","SSLRootCert":"C:\\ca.pem"}}}"#;
    const NEW: &str = r#"{"Servers":{"1":{"Name":"n","Group":"G","Host":"h","Port":"5432","Username":"u","ConnectionParameters":{"sslmode":"require","sslcert":"c.pem","sslkey":"k.pem"}}}}"#;

    fn one(json: &str) -> Parsed {
        parse(json, &[]).unwrap().remove(0)
    }

    #[test]
    fn old_flat_format_maps_every_field() {
        let c = one(OLD).config.unwrap();
        assert_eq!((c.name.as_str(), c.group.as_deref(), c.host.as_str(), c.port), ("prod", Some("Üretim"), "db1", 5433));
        assert_eq!((c.maintenance_db.as_str(), c.username.as_str()), ("app", "u"));
        assert_eq!(c.ssl.mode, SslMode::VerifyFull);
        assert_eq!(c.ssl.root_cert.as_deref(), Some("C:\\ca.pem"));
        assert!(!c.save_password);
    }

    #[test]
    fn connection_parameters_format_and_string_port() {
        let p = one(NEW);
        let c = p.config.unwrap();
        assert_eq!((c.port, c.ssl.mode), (5432, SslMode::Require));
        assert_eq!((c.ssl.client_cert.as_deref(), c.ssl.client_key.as_deref()), (Some("c.pem"), Some("k.pem")));
        assert_eq!(c.maintenance_db, "postgres");
    }

    #[test]
    fn connection_parameters_win_over_flat_fields() {
        let j = r#"{"Servers":{"1":{"Name":"n","Host":"h","Username":"u","SSLMode":"disable","ConnectionParameters":{"sslmode":"verify-ca"}}}}"#;
        assert_eq!(one(j).config.unwrap().ssl.mode, SslMode::VerifyCa);
    }

    #[test]
    fn utf8_bom_is_accepted_and_numeric_keys_sort_numerically() {
        let j = format!("\u{feff}{}", r#"{"Servers":{"10":{"Name":"ten","Host":"h","Username":"u"},"2":{"Name":"two","Host":"h","Username":"u"},"1":{"Name":"one","Host":"h","Username":"u"}}}"#);
        let names: Vec<_> = parse(&j, &[]).unwrap().into_iter().map(|p| p.item.name).collect();
        assert_eq!(names, ["one", "two", "ten"]);
    }

    #[test]
    fn indices_are_stable_positions() {
        let items = preview(OLD, &[]).unwrap();
        assert_eq!(items[0].index, 0);
    }

    #[test]
    fn not_pgadmin_files_are_rejected_with_a_clear_message() {
        for bad in ["", "not json", "[]", r#"{"servers":{}}"#, r#"{"Servers":[1]}"#] {
            let err = parse(bad, &[]).unwrap_err();
            assert!(matches!(&err, Error::Invalid(m) if m.contains("pgAdmin")), "{bad:?} -> {err:?}");
        }
    }

    #[test]
    fn missing_or_bad_fields_become_item_errors_not_failures() {
        let j = r#"{"Servers":{"1":{"Name":"a","Username":"u"},"2":{"Name":"b","Host":"h"},"3":{"Name":"c","Host":"h","Username":"u","Port":70000},"4":{"Name":"d","Host":"h","Username":"u","Port":"x"}}}"#;
        let items = preview(j, &[]).unwrap();
        assert_eq!(items.len(), 4);
        assert!(items.iter().all(|i| i.error.is_some()), "{items:?}");
        assert!(items[0].error.as_deref().unwrap().contains("Host"));
        assert!(items[1].error.as_deref().unwrap().contains("Kullanıcı"));
        assert!(items[2].error.as_deref().unwrap().contains("port"));
    }

    #[test]
    fn missing_port_defaults_to_5432_and_missing_name_falls_back_to_host_port() {
        let c = one(r#"{"Servers":{"1":{"Host":"h","Username":"u"}}}"#).config.unwrap();
        assert_eq!((c.port, c.name.as_str()), (5432, "h:5432"));
    }

    #[test]
    fn unsupported_fields_produce_warnings() {
        let j = r#"{"Servers":{"1":{"Name":"n","Host":"h","Username":"u","UseSSHTunnel":1,"TunnelHost":"x","PassFile":"/p","Service":"s","SSLMode":"allow"}}}"#;
        let p = one(j);
        let w = p.item.warnings.join("|");
        assert!(w.contains("SSH"), "{w}");
        assert!(w.contains("passfile"), "{w}");
        assert!(w.contains("service"), "{w}");
        assert!(w.contains("allow"), "{w}");
        assert_eq!(p.config.unwrap().ssl.mode, SslMode::Prefer);
    }

    #[test]
    fn client_cert_without_key_is_dropped_with_a_warning() {
        let j = r#"{"Servers":{"1":{"Name":"n","Host":"h","Username":"u","SSLCert":"c.pem"}}}"#;
        let p = one(j);
        assert!(p.item.warnings.iter().any(|w| w.contains("birlikte")));
        let c = p.config.unwrap();
        assert!(c.ssl.client_cert.is_none() && c.ssl.client_key.is_none());
    }

    #[test]
    fn duplicates_match_on_host_port_user_and_name() {
        let existing = [ServerConfig::new("prod", "DB1", 5433, "u")];
        let items = preview(OLD, &existing).unwrap();
        assert!(items[0].duplicate, "host karşılaştırması büyük/küçük harf duyarsız");
        let other = [ServerConfig::new("prod2", "db1", 5433, "u")];
        assert!(!preview(OLD, &other).unwrap()[0].duplicate);
    }

    #[test]
    fn passwords_in_the_file_are_never_read() {
        let j = r#"{"Servers":{"1":{"Name":"n","Host":"h","Username":"u","Password":"secret","password":"secret"}}}"#;
        let dump = format!("{:?}", parse(j, &[]).unwrap().remove(0).item);
        assert!(!dump.contains("secret"));
    }

    #[test]
    fn apply_imports_only_selected_valid_items_with_fresh_ids() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = crate::registry::Registry::load(dir.path().join("servers.json")).unwrap().registry;
        let j = r#"{"Servers":{"1":{"Name":"a","Host":"h","Username":"u"},"2":{"Name":"b","Host":"h"},"3":{"Name":"c","Host":"h","Username":"u"}}}"#;
        let n = apply(&mut reg, j, &[0, 1]).unwrap();
        assert_eq!(n, 1, "hatalı kayıt (index 1) atlanır, seçilmeyen (2) alınmaz");
        assert_eq!(reg.servers().len(), 1);
        assert_eq!(reg.servers()[0].name, "a");
        assert_eq!(parse(j, &[]).unwrap()[0].config.as_ref().unwrap().id == reg.servers()[0].id, false);
    }
}
```

- [ ] **Step 2: Çalıştır → derleme hatası/başarısız**

Run: `cargo test -p pgt-core import::`
Expected: FAIL

- [ ] **Step 3: Uygula**

```rust
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::config::{ServerConfig, SslMode};
use crate::registry::Registry;
use crate::{Error, Result};

const NOT_PGADMIN: &str = "Dosya pgAdmin servers.json biçiminde değil.";
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportItem {
    pub index: u32,
    pub name: String,
    pub group: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub duplicate: bool,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

pub struct Parsed {
    pub item: ImportItem,
    pub config: Option<ServerConfig>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn number(v: &Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn truthy(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().is_some_and(|n| n != 0),
        Some(Value::String(s)) => matches!(s.trim(), "1" | "true" | "True"),
        _ => false,
    }
}

/// `ConnectionParameters.<param>` önce, yoksa düz `<flat>` alanı.
fn pick(v: &Value, flat: &str, param: &str) -> Option<String> {
    v.get("ConnectionParameters").and_then(|p| text(p, param)).or_else(|| text(v, flat))
}

fn ssl_mode(raw: Option<String>, warnings: &mut Vec<String>) -> SslMode {
    let Some(raw) = raw else { return SslMode::Prefer };
    match raw.to_ascii_lowercase().as_str() {
        "disable" => SslMode::Disable,
        "prefer" => SslMode::Prefer,
        "require" => SslMode::Require,
        "verify-ca" => SslMode::VerifyCa,
        "verify-full" => SslMode::VerifyFull,
        "allow" => {
            warnings.push("SSL modu “allow” desteklenmiyor; “prefer” olarak aktarıldı.".into());
            SslMode::Prefer
        }
        other => {
            warnings.push(format!("Bilinmeyen SSL modu “{other}”; “prefer” olarak aktarıldı."));
            SslMode::Prefer
        }
    }
}

fn parse_entry(index: u32, v: &Value, existing: &[ServerConfig]) -> Parsed {
    let mut warnings = Vec::new();
    let host = text(v, "Host").unwrap_or_default();
    let username = text(v, "Username").unwrap_or_default();
    let group = text(v, "Group");
    let mut error = None;
    let port = match v.get("Port") {
        None | Some(Value::Null) => 5432,
        Some(_) => match number(v, "Port") {
            Some(p) if (1..=65535).contains(&p) => p as u16,
            _ => {
                error = Some("Geçersiz port.".to_string());
                0
            }
        },
    };
    if error.is_none() && host.is_empty() {
        error = Some("Host alanı yok.".into());
    }
    if error.is_none() && username.is_empty() {
        error = Some("Kullanıcı adı yok.".into());
    }
    let name = text(v, "Name").unwrap_or_else(|| format!("{host}:{port}"));
    if truthy(v, "UseSSHTunnel") {
        warnings.push("SSH tüneli ayarları yok sayıldı.".into());
    }
    if pick(v, "PassFile", "passfile").is_some() {
        warnings.push("passfile yok sayıldı (pgpass desteklenmiyor).".into());
    }
    if text(v, "Service").is_some() {
        warnings.push("service tanımı yok sayıldı.".into());
    }
    let mode = ssl_mode(pick(v, "SSLMode", "sslmode"), &mut warnings);
    let (mut cert, mut key) = (pick(v, "SSLCert", "sslcert"), pick(v, "SSLKey", "sslkey"));
    if cert.is_some() != key.is_some() {
        warnings.push("İstemci sertifikası ve anahtarı birlikte verilmediği için ikisi de yok sayıldı.".into());
        (cert, key) = (None, None);
    }
    let duplicate = error.is_none()
        && existing.iter().any(|s| s.host.eq_ignore_ascii_case(&host) && s.port == port && s.username == username && s.name == name);

    let mut config = None;
    if error.is_none() {
        let mut c = ServerConfig::new(name.clone(), host.clone(), port, username.clone());
        c.group = group.clone();
        c.maintenance_db = text(v, "MaintenanceDB").unwrap_or_else(|| "postgres".into());
        c.ssl.mode = mode;
        c.ssl.root_cert = pick(v, "SSLRootCert", "sslrootcert");
        c.ssl.client_cert = cert;
        c.ssl.client_key = key;
        let c = c.normalized();
        match c.validate() {
            Ok(()) => config = Some(c),
            Err(e) => error = Some(e.to_string()),
        }
    }
    Parsed { item: ImportItem { index, name, group, host, port, username, duplicate, warnings, error }, config }
}

pub fn parse(text: &str, existing: &[ServerConfig]) -> Result<Vec<Parsed>> {
    let invalid = || Error::Invalid(NOT_PGADMIN.into());
    let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| invalid())?;
    let servers: &Map<String, Value> = root.get("Servers").and_then(Value::as_object).ok_or_else(invalid)?;
    let mut entries: Vec<(&String, &Value)> = servers.iter().collect();
    entries.sort_by_key(|(k, _)| (k.parse::<u64>().unwrap_or(u64::MAX), k.to_string()));
    Ok(entries.into_iter().enumerate().map(|(i, (_, v))| parse_entry(i as u32, v, existing)).collect())
}

pub fn preview(text: &str, existing: &[ServerConfig]) -> Result<Vec<ImportItem>> {
    Ok(parse(text, existing)?.into_iter().map(|p| p.item).collect())
}

/// Seçilen (ve geçerli) kayıtları yeni UUID'lerle ekler; eklenen sayıyı döner.
pub fn apply(registry: &mut Registry, text: &str, indices: &[u32]) -> Result<u32> {
    let existing = registry.servers().to_vec();
    let mut count = 0;
    for p in parse(text, &existing)? {
        if let (true, Some(config)) = (indices.contains(&p.item.index), p.config) {
            registry.upsert(config)?;
            count += 1;
        }
    }
    Ok(count)
}

/// Dosyayı sınırlı boyutta okur.
pub fn read_file(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(Error::Invalid("Dosya çok büyük (en fazla 5 MB).".into()));
    }
    std::fs::read_to_string(path).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
}
```

`Parsed` içindeki `config` `ServerConfig::new` ile her çağrıda yeni UUID alır; `apply` yalnızca seçilenleri ekler. `Error::Invalid` Türkçe mesajı olduğu gibi taşır (mevcut davranış).

- [ ] **Step 4: Çalıştır → geçmeli (düzelt)**

Run: `cargo test -p pgt-core import::`
Expected: PASS. `apply_imports_only…` testindeki son (UUID farklı) iddia gürültülüyse sadeleştirin; dosya okuma için ayrıca bir test ekleyin: `read_file` boyut sınırı (`MAX_FILE_BYTES + 1` bayt geçici dosya → `Error::Invalid`) ve olmayan dosya → `Error::Config`.

- [ ] **Step 5: Tauri komutları**

```rust
#[tauri::command]
pub fn preview_pgadmin_import(state: State<'_, AppState>, path: String) -> CmdResult<Vec<pgt_core::import::ImportItem>> {
    let text = pgt_core::import::read_file(std::path::Path::new(&path))?;
    let existing = state.registry.read().unwrap().servers().to_vec();
    Ok(pgt_core::import::preview(&text, &existing)?)
}

#[tauri::command]
pub fn apply_pgadmin_import(state: State<'_, AppState>, path: String, indices: Vec<u32>) -> CmdResult<u32> {
    let text = pgt_core::import::read_file(std::path::Path::new(&path))?;
    Ok(pgt_core::import::apply(&mut state.registry.write().unwrap(), &text, &indices)?)
}
```

`main.rs`'e ekleyin; `commands.ts`: `previewPgadminImport: (path: string) => call<ImportItem[]>("preview_pgadmin_import", { path })`, `applyPgadminImport: (path: string, indices: number[]) => call<number>("apply_pgadmin_import", { path, indices })` (`ImportItem` bağlaması `cargo test` ile üretilir; commit'e ekleyin). Tauri `dialog:allow-open` izni zaten vardır; dosyayı arka uç yoldan okuduğu için ek `fs` izni gerekmez.

- [ ] **Step 6: Doğrula ve commit**

Run: `cargo test --workspace --all-features`, `bun run typecheck`
Expected: PASS

```bash
git add crates/pgt-core src-tauri src/ipc
git commit -m "feat(core): pgAdmin servers.json içe aktarma (önizleme + uygulama)"
```

---

### Task 3: Toast altyapısı ve başlangıç bildiriminin temizlenmesi

**Files:**
- Create: `src/app/toast-model.ts`, `src/app/toast.ts`, `src/app/Toaster.tsx`
- Modify: `src/app/App.tsx` (notice → toast), `src/app/StatusBar.tsx` (notice kaldırılır), `src/app/strings.ts`
- Test: `src/app/toast-model.test.ts`

**Interfaces:**
- Produces: `toast.show(text: string, kind?: "info" | "success" | "error")`, `toast.dismiss(id)`, `<Toaster />`; saf `pushToast`, `dismissToast`, `durationFor`, `MAX_TOASTS = 4`.

- [ ] **Step 1: Başarısız testleri yaz**

```ts
import { describe, expect, it } from "vitest";
import { MAX_TOASTS, dismissToast, durationFor, pushToast, type Toast } from "./toast-model";

const mk = (id: number, kind: Toast["kind"] = "info"): Toast => ({ id, kind, text: `t${id}` });

describe("toast model", () => {
  it("appends and keeps only the newest MAX_TOASTS", () => {
    let list: Toast[] = [];
    for (let i = 1; i <= MAX_TOASTS + 2; i++) list = pushToast(list, mk(i));
    expect(list.map((t) => t.id)).toEqual([3, 4, 5, 6]);
  });
  it("dismisses by id and ignores unknown ids", () => {
    const list = [mk(1), mk(2)];
    expect(dismissToast(list, 1).map((t) => t.id)).toEqual([2]);
    expect(dismissToast(list, 9)).toEqual(list);
  });
  it("errors stay longer than info/success", () => {
    expect(durationFor("error")).toBeGreaterThan(durationFor("info"));
    expect(durationFor("success")).toBe(durationFor("info"));
  });
});
```

- [ ] **Step 2: Çalıştır → başarısız**

Run: `bun run test -- toast-model`
Expected: FAIL

- [ ] **Step 3: Uygula**

`toast-model.ts`:

```ts
export type ToastKind = "info" | "success" | "error";
export interface Toast { id: number; kind: ToastKind; text: string }
export const MAX_TOASTS = 4;

export function pushToast(list: Toast[], toast: Toast): Toast[] {
  const next = [...list, toast];
  return next.length > MAX_TOASTS ? next.slice(next.length - MAX_TOASTS) : next;
}
export function dismissToast(list: Toast[], id: number): Toast[] {
  return list.some((t) => t.id === id) ? list.filter((t) => t.id !== id) : list;
}
export function durationFor(kind: ToastKind): number {
  return kind === "error" ? 8000 : 3500;
}
```

`toast.ts`: modül düzeyinde `createRoot` içinde `createSignal<Toast[]>([])`; `show(text, kind = "info")` artan sayaçla `pushToast`, `setTimeout(() => dismiss(id), durationFor(kind))`; `toast = { list, show, dismiss }` dışa aktarılır. `Toaster.tsx`: sağ altta `fixed bottom-8 right-3 z-[60] flex flex-col gap-2`, her toast `role="status"` (`error` için `role="alert"`), renk token'ları (`bg-panel border-border`, `error` için `text-danger`), kapatma düğmesi (`aria-label={t.toast.dismiss}`). Rutin hatalar için modal yok (§7.2).

`strings.ts`'e `toast: { dismiss: "Kapat" }` ekleyin. `App.tsx`: `notice` sinyalini ve `StatusBar`'ın `notice` prop'unu kaldırın; `onMount` sonunda `const n = await commands.startupNotice().catch(() => null); if (n) toast.show(n, "error");`. `<Toaster />` `App` kökünün sonuna eklenir. Mevcut `catch (e) { setNotice(toPayload(e).message) }` (openDialog) → `toast.show(toPayload(e).message, "error")`.

- [ ] **Step 4: Doğrula ve commit**

Run: `bun run test && bun run typecheck`
Expected: PASS

```bash
git add src
git commit -m "feat(ui): toast bildirimleri; başlangıç bildirimi artık kendiliğinden kapanır"
```

### Task 4: İçe aktarma arayüzü

**Files:**
- Create: `src/features/servers/import-model.ts`, `src/features/servers/ImportDialog.tsx`
- Modify: `src/app/Toolbar.tsx` / `src/app/App.tsx` (İçe aktar düğmesi), `src/app/strings.ts`
- Test: `src/features/servers/import-model.test.ts`

**Interfaces:**
- Consumes: `commands.previewPgadminImport/applyPgadminImport` (Task 2), `toast` (Task 3), `ImportItem` bağlaması, `@tauri-apps/plugin-dialog` `open`.
- Produces: saf `defaultSelection(items): Set<number>`, `canImport(items, selected): boolean`, `summarize(items): { total, importable, duplicates, errors }`, `groupLabel(item): string`; `<ImportDialog open onClose onImported />`.

- [ ] **Step 1: Başarısız testleri yaz**

```ts
import { describe, expect, it } from "vitest";
import type { ImportItem } from "../../ipc/bindings/ImportItem";
import { canImport, defaultSelection, groupLabel, summarize } from "./import-model";

const item = (index: number, over: Partial<ImportItem> = {}): ImportItem => ({
  index, name: `s${index}`, group: null, host: "h", port: 5432, username: "u", duplicate: false, warnings: [], error: null, ...over,
});

describe("import model", () => {
  it("selects valid non-duplicate items by default", () => {
    const items = [item(0), item(1, { duplicate: true }), item(2, { error: "Host alanı yok." }), item(3)];
    expect([...defaultSelection(items)]).toEqual([0, 3]);
  });
  it("cannot import when nothing importable is selected", () => {
    const items = [item(0, { error: "x" }), item(1)];
    expect(canImport(items, new Set())).toBe(false);
    expect(canImport(items, new Set([0]))).toBe(false); // hatalı kayıt seçilemez
    expect(canImport(items, new Set([1]))).toBe(true);
  });
  it("summarizes counts", () => {
    const items = [item(0), item(1, { duplicate: true }), item(2, { error: "x" })];
    expect(summarize(items)).toEqual({ total: 3, importable: 2, duplicates: 1, errors: 1 });
  });
  it("labels ungrouped items", () => {
    expect(groupLabel(item(0))).toBe("(grupsuz)");
    expect(groupLabel(item(0, { group: "Üretim" }))).toBe("Üretim");
  });
});
```

(`"(grupsuz)"` metni `strings.ts`'ten gelir; testte modelin sabiti kullanılıyorsa aynı değeri `import-model.ts` içinde `UNGROUPED` olarak dışa aktarın.)

- [ ] **Step 2: Çalıştır → başarısız; modeli yaz**

```ts
import type { ImportItem } from "../../ipc/bindings/ImportItem";

export const UNGROUPED = "(grupsuz)";

const importable = (i: ImportItem) => i.error === null;

export function defaultSelection(items: ImportItem[]): Set<number> {
  return new Set(items.filter((i) => importable(i) && !i.duplicate).map((i) => i.index));
}
export function canImport(items: ImportItem[], selected: Set<number>): boolean {
  return items.some((i) => importable(i) && selected.has(i.index));
}
export function summarize(items: ImportItem[]) {
  return {
    total: items.length,
    importable: items.filter(importable).length,
    duplicates: items.filter((i) => i.duplicate).length,
    errors: items.filter((i) => !importable(i)).length,
  };
}
export function groupLabel(item: ImportItem): string {
  return item.group ?? UNGROUPED;
}
```

Run: `bun run test -- import-model` → PASS.

- [ ] **Step 3: `ImportDialog.tsx`**

Akış (spec §6.3): "Dosya seç…" düğmesi → `openFile({ multiple: false, filters: [{ name: "pgAdmin servers.json", extensions: ["json"] }] })` → `previewPgadminImport(path)`; yükleme sırasında "Okunuyor…"; hata varsa diyalog içinde (modal içinde, ikinci modal değil) kırmızı metin. Önizleme tablosu: onay kutusu, Ad, Grup, `host:port`, Kullanıcı, durum sütunu ("zaten var" / hata metni / uyarılar `title` ile ve `⚠ n` rozetiyle). Hatalı kayıtlar için onay kutusu `disabled`. Üstte özet satırı (`summarize`) ve "Tümünü seç / Hiçbirini seçme". Alt düğmeler: "Vazgeç", "İçe aktar (N)" (`disabled={!canImport(...)}`; uygulama sürerken de devre dışı). Başarıda: `toast.show(t.import.done(count), "success")`, `onImported()`, `onClose()`. Başarısızlıkta diyalogda hata metni kalır (parola/yol dışında hassas veri yok; yol gösterilebilir). `apply` çağrısı önizlemedeki **aynı `path`** ile yapılır (dosya değişmişse indeksler kayabilir: uygulamadan önce kayıt sayısı değiştiyse arka uç zaten aynı sıralamayı üretir; sınırı `docs/testing.md`'ye "dosya önizleme ile içe aktarma arasında değiştirilirse sonuç değişebilir" olarak yazın).

Diyalog tembel yüklenir (Task 9'da `lazy` ile bağlanır; şimdilik normal import edilebilir). `strings.ts`'e `import: { button: "İçe aktar", title: "pgAdmin sunucularını içe aktar", choose: "Dosya seç…", reading: "Okunuyor…", selectAll: "Tümünü seç", selectNone: "Hiçbirini seçme", duplicate: "zaten var", doImport: (n: number) => \`İçe aktar (${n})\`, done: (n: number) => \`${n} sunucu içe aktarıldı. Parolalar ilk bağlantıda sorulacak.\`, summary: (s) => … , warnings: "Uyarılar" }` ekleyin (`summary` için `${total} kayıt · ${importable} aktarılabilir · ${duplicates} zaten var · ${errors} hatalı`).

- [ ] **Step 4: Toolbar/App bağlantısı**

`App.tsx`'te `importOpen` sinyali; Toolbar'da "Sunucu ekle"nin yanında `FileInput` (lucide) simgeli "İçe aktar" düğmesi. `onImported` → `explorer.controller.reloadServers().catch(() => {})`.

- [ ] **Step 5: Doğrula ve commit**

Run: `bun run test && bun run typecheck`; elle: `bun run tauri dev` ile örnek bir `servers.json` (en az bir yinelenen, bir hatalı, bir SSH tünelli kayıt) içe aktarın; yinelenen varsayılan seçili değil, hatalı kayıt seçilemiyor, sonuç ağaçta gruplarıyla görünüyor.

```bash
git add src
git commit -m "feat(ui): pgAdmin içe aktarma diyaloğu"
```

---

### Task 5: Filtre (yüklenmiş düğümlerde)

**Files:**
- Modify: `src/features/explorer/tree-model.ts`, `src/features/explorer/controller.ts`, `src/features/explorer/TreeView.tsx`, `src/app/Toolbar.tsx`, `src/app/App.tsx`, `src/app/strings.ts`
- Test: `src/features/explorer/tree-model.test.ts` (ekleme), `src/features/explorer/controller.test.ts` (ekleme)

**Interfaces:**
- Produces: `TreeState.filter: string` (`emptyTree()` içinde `""`; `clone()` korur); `filterRows(s: TreeState, query: string): VisibleRow[]`; `flatten(s)` `s.filter.trim() !== ""` ise `filterRows(s, s.filter)` döner; `ancestorOfKind(s, id, kinds: NodeKind[]): TreeNode | null` (Task 6/9 kullanır); `ExplorerController.setFilter(query: string)`.

Kurallar (spec §6.2): yalnızca yüklenmiş düğümler; büyük/küçük harf duyarsız **düz alt dize** (`label.toLowerCase().includes(q.trim().toLowerCase())`; regex yok); eşleşen düğüm üst düğümleriyle gösterilir; üst düğüm kendi alt ağacında eşleşme varsa `expanded=true` satırıyla çizilir; eşleşen bir düğümün eşleşmeyen çocukları gösterilmez; `expanded` kümesi filtre sırasında yok sayılır (değiştirilmez); sunucuya sorgu gitmez. Seçim filtre değişince korunur (filtre dışında kalsa bile). Koleksiyon sayısı etiketi (`Tables (1.243)`) süzülmüş görünümde **yüklenen toplam** çocuk sayısını gösterir (`childCount` değişmez).

- [ ] **Step 1: Başarısız testler (`tree-model.test.ts`)**

Mevcut dosyadaki durum kurma yardımcılarını (`setChildren` ile ağaç kurma) kullanın; yeni yardımcı `build(spec)` yoksa şu kalıpla yazın: ROOT → grup `g` → sunucu `s1`(connected) → `databaseCollection` → `db1` → `schemaCollection` → `public` → `tableCollection` → tablolar.

```ts
describe("filterRows", () => {
  it("shows matching nodes with their ancestors, expanded, and hides non-matching siblings", () => {
    const s = withFilter(tree(["orders", "customers", "order_items"]), "ord");
    const rows = flatten(s);
    expect(rows.map((r) => r.node.label)).toEqual(["Üretim", "pg-prod", "Databases", "app", "Schemas", "public", "Tables", "orders", "order_items"]);
    expect(rows.slice(0, -2).every((r) => r.expanded)).toBe(true);
    expect(rows.at(-1)!.posInSet).toBe(2);
    expect(rows.at(-1)!.setSize).toBe(2);
  });
  it("is case-insensitive", () => {
    expect(flatten(withFilter(tree(["Orders"]), "oRDERS")).map((r) => r.node.label)).toContain("Orders");
  });
  it("treats the query as a plain substring", () => {
    const s = tree(["a.b", "axb", "100%", "c(d)", "back\\slash", "ÇOK", "İstanbul"]);
    for (const [q, expected] of [["a.b", ["a.b"]], [".*", []], ["100%", ["100%"]], ["(d)", ["c(d)"]], ["\\", ["back\\slash"]], ["çok", []], ["İst", ["İstanbul"]]] as const) {
      const labels = flatten(withFilter(s, q)).filter((r) => r.node.kind === "table").map((r) => r.node.label);
      expect(labels, q).toEqual(expected);
    }
  });
  it("blank query does not filter and keeps normal expansion", () => {
    const base = tree(["a", "b"]);
    expect(flatten(withFilter(base, "   "))).toEqual(flatten(base));
  });
  it("only searches loaded nodes", () => {
    // 'hidden' tablosunun koleksiyonu yüklenmemiş: eşleşme yok
    expect(flatten(withFilter(treeWithUnloadedTables(), "hidden"))).toEqual([]);
  });
  it("does not change expansion or selection", () => {
    const base = { ...tree(["a"]), selected: "tbl-a" };
    const filtered = withFilter(base, "zzz");
    expect(filtered.selected).toBe("tbl-a");
    expect(filtered.expanded).toEqual(base.expanded);
  });
  it("filters 10,000 loaded nodes in under 50 ms", () => {
    const s = withFilter(tree(Array.from({ length: 10000 }, (_, i) => `table_${i}`)), "table_99");
    const times = Array.from({ length: 5 }, () => {
      const t = performance.now();
      flatten(s);
      return performance.now() - t;
    });
    expect(Math.min(...times)).toBeLessThan(50);
  });
});
```

`withFilter(s, q) = ({ ...s, filter: q })`. `tree(labels)` ve `treeWithUnloadedTables()` test içi yardımcılardır (node'ları `TreeNode` literal'leriyle `setChildren` zinciri kurar).

- [ ] **Step 2: Çalıştır → başarısız. Uygula**

`tree-model.ts` (döngüsel içe aktarmayı önlemek için `filterRows` aynı dosyada):

```ts
export function filterRows(s: TreeState, query: string): VisibleRow[] {
  const q = query.trim().toLowerCase();
  if (q === "") return [];
  const walk = (parentId: string, depth: number): VisibleRow[] => {
    const ids = s.children.get(parentId);
    if (!ids) return [];
    const kept: { id: string; node: TreeNode; sub: VisibleRow[] }[] = [];
    for (const id of ids) {
      const node = s.nodes.get(id);
      if (!node) continue;
      const sub = walk(id, depth + 1);
      if (sub.length > 0 || node.label.toLowerCase().includes(q)) kept.push({ id, node, sub });
    }
    const out: VisibleRow[] = [];
    kept.forEach(({ id, node, sub }, index) => {
      const kids = s.children.get(id);
      out.push({
        id, node, depth,
        expanded: sub.length > 0,
        loading: s.loading.has(id),
        error: s.errors.get(id) ?? null,
        childCount: kids ? kids.length : null,
        setSize: kept.length,
        posInSet: index + 1,
      });
      out.push(...sub);
    });
    return out;
  };
  return walk(ROOT, 0);
}

export function ancestorOfKind(s: TreeState, id: string | null, kinds: NodeKind[]): TreeNode | null {
  let cur: string | undefined = id ?? undefined;
  while (cur !== undefined && cur !== ROOT) {
    const node = s.nodes.get(cur);
    if (!node) return null;
    if (kinds.includes(node.kind)) return node;
    cur = s.parent.get(cur);
  }
  return null;
}
```

`flatten` başına: `if (s.filter.trim() !== "") return filterRows(s, s.filter);`. `TreeState`/`emptyTree`/`clone` `filter` alanını taşır. **`keyIntent`'te** süzülmüş görünümde `ArrowLeft` daraltma yerine üst düğüme çıkar (`if (row.expanded && s.filter.trim() === "")`) ve `Enter`/`ArrowRight` daraltma/genişletme `expanded` yerine yalnızca süzülmüş değilken çalışır; süzülmüşken `ArrowRight` çocuğa iner (`rows[index+1]?.depth === row.depth + 1`), `Enter` hiçbir şey yapmaz. Bu davranışlar için `keyIntent` testleri ekleyin.

`setFilter(q)`: `this.update((s) => ({ ...s, filter: q }))`. `controller.test.ts`: `setFilter` sonrası `flatten` süzülür; boş dize ile döner.

- [ ] **Step 3: Arayüz**

Toolbar'a filtre kutusu (`<input type="search">`, `placeholder={t.toolbar.filter}`, sağda temizle düğmesi, `aria-label`), `App` içinde `let filterEl` ve `window` `keydown` dinleyicisi: `Ctrl+F` → `preventDefault`, `filterEl.focus(); filterEl.select()`. Filtre kutusunda `Escape` → `setFilter("")` ve odak ağaca (`treeEl.focus()`); `ArrowDown` odağı ağaca taşır. Filtre etkinken ağaç boşsa (`rows().length === 0`) `t.explorer.noMatch` ("Eşleşme yok. Filtre yalnızca yüklenmiş düğümlerde arar.") gösterilir (mevcut "Henüz sunucu yok" mesajından ayrı; `isEmpty` mantığı filtreyi hesaba katar). `strings.ts`: `toolbar.filter: "Filtre…  (Ctrl+F)"`, `toolbar.clearFilter: "Filtreyi temizle"`, `explorer.noMatch`.

Bellek notu: filtre kutusu `createSignal` + her tuşta `setFilter` (debounce yok; 10.000 düğüm < 50 ms testle kanıtlı).

- [ ] **Step 4: Doğrula ve commit**

Run: `bun run test && bun run typecheck`
Expected: PASS. Elle: Ctrl+F, yazarken ağaç süzülür, Esc temizler, ↑↓ süzülmüş satırlarda gezer.

```bash
git add src
git commit -m "feat(explorer): yüklenmiş düğümlerde filtre (Ctrl+F)"
```

---

### Task 6: Sağ tık menüsü, kopyalama eylemleri, sunucu silme/bağlantıyı kesme

**Files:**
- Create: `src/features/explorer/actions.ts`, `src/features/explorer/ExplorerMenu.tsx`, `src/ui/ConfirmDialog.tsx`, `src/app/clipboard.ts`
- Modify: `src/features/explorer/TreeView.tsx`, `src/features/explorer/controller.ts`, `src/features/inspector/Inspector.tsx` (pano yardımcısı), `src/app/App.tsx`, `src/app/strings.ts`
- Test: `src/features/explorer/actions.test.ts`, `src/features/explorer/controller.test.ts` (ekleme)

**Interfaces:**
- Consumes: `commands.{quoteIdentifiers, getDdl, deleteServer, disconnectServer}`, `toast`, `ancestorOfKind`, `ExplorerController`.
- Produces:
  - `executeAction(deps: ActionDeps, action: ActionId, id: string): Promise<void>` (`actions.ts`; arayüz bağımlılıkları enjekte edilir, bu yüzden Vitest'te sahtelerle sınanır): `ActionDeps = { state: () => TreeState; controller: Pick<ExplorerController, "refresh" | "ensureConnected" | "expand" | "disconnect" | "serverRemoved" | "sessionLost">; api: Pick<Api, "quoteIdentifiers" | "getDdl" | "deleteServer">; copy(text: string): Promise<void>; notify(text: string, kind: ToastKind): void; editServer(serverId: string): void; confirmDelete(name: string): Promise<boolean> }`.
  - `type ActionId = "refresh" | "connect" | "disconnect" | "editServer" | "deleteServer" | "copyName" | "copyQualifiedName" | "copyDdl"`.
  - `actionsFor(s: TreeState, node: TreeNode): ActionId[]` (sıralı menü).
  - `qualifiedParts(s: TreeState, id: string): { parts: string[]; suffix: string } | null`.
  - `copyText(text): Promise<void>` (`src/app/clipboard.ts`).
  - `ExplorerController.disconnect(id)` ve `ExplorerController.serverRemoved(id)`; `<ConfirmDialog open title message confirmLabel danger onConfirm onCancel />`.

Menü içeriği (spec §6.2, yalnızca okuma): Yenile; Bağlan / Bağlantıyı kes; Sunucuyu düzenle / sil (onaylı); Adı kopyala; Nitelikli adı kopyala; DDL'i kopyala.

`actionsFor` kuralları:
- `serverGroup`: `["refresh", "copyName"]`
- `server` bağlı: `["refresh", "disconnect", "editServer", "deleteServer", "copyName"]`; bağlı değil: `["connect", "editServer", "deleteServer", "copyName"]`
- koleksiyon (`isCollection`): `["refresh", "copyName"]`
- diğer nesneler: `refresh` yalnızca `node.hasChildren` ise; `copyName`; `copyQualifiedName` yalnızca `qualifiedParts` ≠ null ise; `copyDdl` her zaman (DDL boşsa toast: "Bu nesne için DDL yok.").

`qualifiedParts` kuralları (şema bilgisi **ağaç atalarından** gelir; kimlik ayrıştırılmaz):
- `table | view | materializedView | sequence | type | domain | index`: `[şema.label, label]`, `suffix ""`. Şema = en yakın `schema` atası; yoksa `null`. **Bir `table`'ın ebeveyni `partitionCollection` ise `null`** (partition başka şemada olabilir; yanlış ad yerine eylem gizlenir).
- `function | procedure | triggerFunction`: `label` imzası `ad(argümanlar)` biçimindedir; `[şema.label, imzadan önceki ad]`, `suffix = "(" + imzanın kalanı` (örn. `fmt(integer)` → parts `[app, fmt]`, suffix `(integer)`). Parantez yoksa suffix `""`.
- `column`: `[şema.label, en yakın table|view|materializedView atası.label, label]`.
- `schema | database | role | tablespace | extension`: `[label]`.
- `constraint | trigger | policy`, koleksiyonlar, `server`, `serverGroup`: `null`.
Sonuç metni: `quoteIdentifiers(serverId, parts)` dönen adlar `.` ile birleştirilip `suffix` eklenir.

- [ ] **Step 1: Başarısız testler (`actions.test.ts`)**

Ağaç kurma yardımcılarını Task 5 testlerindeki gibi kullanın. Tablolarla:

```ts
it("qualifiedParts follows the schema ancestor", () => {
  expect(qualifiedParts(s, "tbl-orders")).toEqual({ parts: ["app", "orders"], suffix: "" });
  expect(qualifiedParts(s, "col-id")).toEqual({ parts: ["app", "orders", "id"], suffix: "" });
  expect(qualifiedParts(s, "fn-fmt-int")).toEqual({ parts: ["app", "fmt"], suffix: "(integer)" });
  expect(qualifiedParts(s, "db-app")).toEqual({ parts: ["app"], suffix: "" });
});
it("hides qualified name where the schema is not trustworthy", () => {
  expect(qualifiedParts(s, "tbl-partition")).toBeNull(); // partitionCollection altında
  expect(qualifiedParts(s, "idx-1")?.parts).toEqual(["app", "orders_pkey"]);
  expect(qualifiedParts(s, "con-1")).toBeNull();
  expect(qualifiedParts(s, "trg-1")).toBeNull();
});
it("menu per node kind", () => {
  expect(actionsFor(s, node("srv-down"))).toEqual(["connect", "editServer", "deleteServer", "copyName"]);
  expect(actionsFor(s, node("srv-up"))).toEqual(["refresh", "disconnect", "editServer", "deleteServer", "copyName"]);
  expect(actionsFor(s, node("coll-tables"))).toEqual(["refresh", "copyName"]);
  expect(actionsFor(s, node("tbl-orders"))).toEqual(["refresh", "copyName", "copyQualifiedName", "copyDdl"]);
  expect(actionsFor(s, node("col-id"))).toEqual(["copyName", "copyQualifiedName", "copyDdl"]);
  expect(actionsFor(s, node("con-1"))).toEqual(["copyName", "copyDdl"]);
});
```

`controller.test.ts`'e (`setup`'taki sahte `api` `disconnectServer: vi.fn()` ile genişler ve `setup` bunu da döndürür; `deps.api` türü `Pick<Api, "listChildren" | "connectServer" | "disconnectServer">` olur):

```ts
const srv = (connected: boolean) =>
  n("srv", { kind: "server", serverId: "u1", detail: connected ? "18.0" : null, flags: { ...flags, connected } });

it("disconnect marks the server disconnected, clears its subtree and calls the api once", async () => {
  const { c, get, disconnectServer } = setup({ [ROOT]: [srv(true)], srv: [n("dbs")] });
  await c.load(ROOT);
  await c.expand("srv");
  c.select("dbs");
  c.disconnect("srv");
  expect(disconnectServer).toHaveBeenCalledExactlyOnceWith("u1");
  expect(get().nodes.get("srv")!.flags.connected).toBe(false);
  expect(get().children.has("srv")).toBe(false);
  expect(get().selected).toBe("srv");
});

it("removed server clears the selection and its subtree", async () => {
  const children: Record<string, TreeNode[]> = { [ROOT]: [srv(true)], srv: [n("dbs")] };
  const { c, get } = setup(children);
  await c.load(ROOT);
  await c.expand("srv");
  c.select("dbs");
  children[ROOT] = [];
  await c.serverRemoved();
  expect([get().nodes.has("srv"), get().nodes.has("dbs"), get().selected]).toEqual([false, false, null]);
});
```

`actions.test.ts`'e (`executeAction` sahte bağımlılıklarla; `deps(state)` yardımcısı `{ state: () => state, controller, api: { quoteIdentifiers: vi.fn(), getDdl: vi.fn(), deleteServer: vi.fn() }, copy: vi.fn(), notify: vi.fn(), editServer: vi.fn(), confirmDelete: vi.fn(async () => true) }` döndürür; `controller` alanları `vi.fn()`'dir):

```ts
it("action targets are re-read at execution time", async () => {
  const d = deps(emptyTree());
  await executeAction(d, "copyName", "ghost");
  await executeAction(d, "deleteServer", "ghost");
  expect(d.copy).not.toHaveBeenCalled();
  expect(d.api.deleteServer).not.toHaveBeenCalled();
  expect(d.notify).toHaveBeenCalledWith(t.menu.gone, "error");
});

it("copyQualifiedName quotes server-side and appends the signature", async () => {
  const d = deps(sampleTree()); // fn-fmt-int: şema "app", etiket "fmt(integer)", sunucu serverId "u1"
  d.api.quoteIdentifiers.mockResolvedValue(["app", "fmt"]);
  await executeAction(d, "copyQualifiedName", "fn-fmt-int");
  expect(d.api.quoteIdentifiers).toHaveBeenCalledWith("u1", ["app", "fmt"]);
  expect(d.copy).toHaveBeenCalledWith("app.fmt(integer)");
});

it("deleteServer does nothing when the confirmation is declined", async () => {
  const d = deps(sampleTree());
  d.confirmDelete.mockResolvedValue(false);
  await executeAction(d, "deleteServer", "srv-up");
  expect(d.api.deleteServer).not.toHaveBeenCalled();
});

it("deleteServer failure keeps the tree and reports an error toast", async () => {
  const d = deps(sampleTree());
  d.api.deleteServer.mockRejectedValue(new IpcError({ code: "config", message: "kayıt salt-okunur", detail: null, hint: null }));
  await executeAction(d, "deleteServer", "srv-up");
  expect(d.notify).toHaveBeenCalledWith("kayıt salt-okunur", "error");
  expect(d.controller.serverRemoved).not.toHaveBeenCalled();
});
```

- [ ] **Step 2: Çalıştır → başarısız; uygula**

`actions.ts` yukarıdaki kuralları uygular (`ancestorOfKind`, `isCollection`). `controller.ts`:

```ts
disconnect(id: string): void {
  const node = this.get().nodes.get(id);
  if (node?.kind !== "server" || !node.serverId) return;
  void this.deps.api.disconnectServer(node.serverId);
  this.update((s) => {
    const n = s.nodes.get(id);
    return n ? invalidate(patchNode(s, id, { detail: null, flags: { ...n.flags, connected: false } }), id) : s;
  });
}

/** Sunucu kaydı silindi: düğümü ve alt ağacını kaldırmak için sunucu listesini yeniden yükler. */
async serverRemoved(): Promise<void> {
  await this.reloadServers();
}
```

(`reloadServers` zaten kaybolan sunucuyu `setChildren` ile alt ağacıyla temizler; `selected` alt ağaçtaysa ebeveyne düşer — testle doğrulayın; `selected` `null` olursa Inspector boşalır.)

`clipboard.ts`:
```ts
export async function copyText(text: string): Promise<void> {
  await navigator.clipboard.writeText(text);
}
```
`Inspector.tsx` bunu kullanır.

- [ ] **Step 3: Menü bileşeni (`ExplorerMenu.tsx`)**

Tek bir Kobalte `ContextMenu` tüm `TreeView`'ı sarar (satır başına örnek yok; sanallaştırmayla uyumlu): `ContextMenu.Trigger` ağaç kapsayıcısıdır; `onContextMenu` işleyicisi **Kobalte açmadan önce** `e.target.closest('[role="treeitem"]')`'den satır kimliğini okur (`data-node-id` özniteliği `TreeRow`'a eklenir; `rowDomId` yerine ham kimlik `encodeURIComponent` ile), satırı seçer (`controller.select`) ve `menuTarget` sinyalini ayarlar; satır yoksa (boş alan veya klavye `ContextMenu`/Shift+F10 tuşu) hedef `selected`'dır, o da yoksa menü açılmaz (`e.preventDefault()` + `e.stopPropagation()` ile Kobalte'ye geçmeyin; Kobalte `onContextMenu` bileşimini kullanır — kullandığınız sürümde işleyicinin önce çalıştığını doğrulayın). İçerik: `actionsFor(state, node)` sırasıyla `ContextMenu.Item`'lar, Türkçe etiketler `strings.ts`'ten (`menu.refresh: "Yenile"`, `connect: "Bağlan"`, `disconnect: "Bağlantıyı kes"`, `editServer: "Sunucuyu düzenle…"`, `deleteServer: "Sunucuyu sil…"`, `copyName: "Adı kopyala"`, `copyQualifiedName: "Nitelikli adı kopyala"`, `copyDdl: "DDL'i kopyala"`), ayırıcılar `refresh/connect` ile sunucu eylemleri ve kopyalama eylemleri arasında. Menü açıkken ağaç kaydırması/yenilemesi hedefi kaldırırsa menü kapanır (`menuTarget` düğümü `nodes`'ta yoksa `open=false`).

Eylem yürütücü `executeAction` (`actions.ts`; `App.tsx` gerçek bağımlılıkları enjekte eder: `copy = copyText`, `notify = toast.show`, `editServer = openDialog`, `confirmDelete` = `ConfirmDialog` sözü): **önce düğümü `deps.state().nodes.get(id)` ile yeniden okur**; yoksa `toast.show(t.menu.gone, "error")` ve çık (Review Focus 5). Sonra:
- `refresh` → `controller.refresh(id)`; `connect` → `controller.ensureConnected(id)` ardından `controller.expand(id)`; `disconnect` → `controller.disconnect(id)` + toast `t.menu.disconnected`.
- `editServer` → `openDialog(node.serverId)` (App'teki mevcut yardımcı).
- `deleteServer` → `ConfirmDialog` (başlık "Sunucu silinsin mi?", mesaj `‘${name}’ kaydı ve kayıtlı parolası silinir. PostgreSQL sunucusundaki hiçbir şey etkilenmez.`, düğme "Sil" `danger`) → onayda `commands.deleteServer(serverId)` → `controller.serverRemoved()` → `toast.show(t.menu.deleted(name), "success")`; hata (ör. salt-okunur kayıt) → `toast.show(message, "error")` ve ağaç değişmez.
- `copyName` → `copyText(node.label)`; `copyQualifiedName` → `qualifiedParts` → `commands.quoteIdentifiers(serverId, parts)` → `copyText(quoted.join(".") + suffix)`; `copyDdl` → `commands.getDdl(id)` (sonuç boşsa `t.menu.noDdl`) → `copyText`. Başarıda `toast.show(t.menu.copied, "success")`; hatada hata toast'u, oturum kaybı ise `controller.sessionLost(id, payload)`.
`Ctrl+C` (ağaç odaktayken, seçili düğüm varsa): `copyName`; `TreeView` `onKeyDown`'ında `e.ctrlKey && e.key === "c"` ve `window.getSelection()` boşsa.

`ConfirmDialog` mevcut `Dialog` kabuğunu kullanır; `Escape`/Vazgeç = iptal, odak "Vazgeç" düğmesinde başlar (yıkıcı eylem yanlışlıkla Enter ile onaylanmasın).

- [ ] **Step 4: Doğrula ve commit**

Run: `bun run test && bun run typecheck`
Expected: PASS. Elle: sunucuya, tabloya, kolona sağ tıklayın; menü içerikleri kurallara uyar; `Mixed View` ve `fmt(integer)` için nitelikli ad doğru tırnaklı; sunucu silme onayı çalışır, bağlıyken silince ağaç ve Inspector boşalır.

```bash
git add src
git commit -m "feat(explorer): sağ tık menüsü, kopyalama eylemleri, sunucu silme ve bağlantıyı kesme"
```

---

### Task 7: Düğüm hata rozeti, "Tekrar dene" ve `NotFound` akışı

**Files:**
- Create: `src/features/explorer/ErrorBadge.tsx`
- Modify: `src/features/explorer/TreeRow.tsx`, `src/features/explorer/controller.ts`, `src/features/inspector/Inspector.tsx`, `src/app/strings.ts`
- Test: `src/features/explorer/controller.test.ts` (ekleme)

**Interfaces:**
- Produces: `ExplorerController.retry(id): Promise<void>`, `ExplorerController.nodeGone(id, error): boolean`; `<ErrorBadge error onRetry onRefreshParent? />`.

Davranış (§7.2): hata satırında kırmızı rozet; üzerine gelince (Kobalte `HoverCard`, 150 ms gecikme; klavyede rozet odaklanabilir bir düğmedir, Enter = Tekrar dene) mesaj, `detail`, `hint` ve **Tekrar dene** düğmesi. `not_found` hatası: satır soluk + üstü çizili görünür, kart "Nesne artık mevcut değil." der ve **Tekrar dene** yerine **Üst düğümü yenile** sunar.

- [ ] **Step 1: Başarısız testler (`controller.test.ts`)**

```ts
const noApi = { connectServer: vi.fn(), disconnectServer: vi.fn() };

it("retry on a disconnected server reconnects then reloads", async () => {
  const srv = n("srv", { kind: "server", serverId: "u1" });
  const { c, get, connectServer, listChildren } = setup({ [ROOT]: [srv], srv: [n("dbs")] });
  await c.load(ROOT);
  await c.retry("srv");
  expect(connectServer).toHaveBeenCalledTimes(1);
  expect(listChildren).toHaveBeenCalledWith("srv");
  expect(get().nodes.get("srv")!.flags.connected).toBe(true);
  expect(get().errors.has("srv")).toBe(false);
  expect(flatten(get()).map((r) => r.id)).toEqual(["srv", "dbs"]);
});

it("retry on a regular node reloads it and clears the error", async () => {
  let calls = 0;
  const listChildren = vi.fn(async (id: string) => {
    if (id === ROOT) return [n("a")];
    if (calls++ === 0) throw err("query");
    return [n("a1")];
  });
  const { c, get } = setup({}, { api: { listChildren, ...noApi } });
  await c.load(ROOT);
  await c.expand("a");
  expect(get().errors.get("a")?.code).toBe("query");
  await c.retry("a");
  expect(get().errors.has("a")).toBe(false);
  expect(get().children.get("a")).toEqual(["a1"]);
});

it("nodeGone marks only not_found errors and only for existing nodes", async () => {
  const { c, get } = setup({ [ROOT]: [n("a")] });
  await c.load(ROOT);
  const payload = (code: string): ErrorPayload => ({ code, message: code, detail: null, hint: null });
  expect(c.nodeGone("a", payload("query"))).toBe(false);
  expect(c.nodeGone("ghost", payload("not_found"))).toBe(false);
  expect(c.nodeGone("a", payload("not_found"))).toBe(true);
  expect(get().errors.get("a")?.code).toBe("not_found");
});

it("a not_found load error keeps the node in the tree", async () => {
  const listChildren = vi.fn(async (id: string) => {
    if (id === ROOT) return [n("a")];
    throw err("not_found");
  });
  const { c, get } = setup({}, { api: { listChildren, ...noApi } });
  await c.load(ROOT);
  await c.expand("a");
  const row = flatten(get())[0];
  expect([row.id, row.error?.code]).toEqual(["a", "not_found"]);
});
```

- [ ] **Step 2: Çalıştır → başarısız; uygula**

```ts
async retry(id: string): Promise<void> {
  const node = this.get().nodes.get(id);
  if (!node) return;
  if (node.kind === "server" && !node.flags.connected) {
    if (!(await this.ensureConnected(id))) return;
    if (!this.get().nodes.has(id)) return;
    this.update((s) => expand(s, id));
  }
  await this.load(id);
}

nodeGone(id: string, error: ErrorPayload): boolean {
  if (error.code !== "not_found" || !this.get().nodes.has(id)) return false;
  this.update((s) => setError(s, id, error));
  return true;
}
```

`Inspector.tsx` `guarded`: `catch` içinde `sessionLost` başarısızsa `props.explorer.controller.nodeGone(id, payload)` da çağrılır (properties/DDL `not_found` → ağaçta düğüm "artık mevcut değil" olur).

- [ ] **Step 3: Bileşenler**

`ErrorBadge.tsx`: `HoverCard` (`@kobalte/core/hover-card`), tetikleyici `<button aria-label={t.error.badge}>` + `CircleAlert`; içerik: kalın `message`, `detail` (seçilebilir, `select-text`, `break-words`), `hint` (muted), altta düğme: `not_found` ise `t.error.refreshParent` → `onRefreshParent()`, değilse `t.error.retry` → `onRetry()`. Kart `max-w-[360px]`. `TreeRow.tsx` mevcut `title`'lı rozet yerine `<ErrorBadge>` kullanır; `row.error?.code === "not_found"` iken satıra `line-through opacity-60`. `TreeView` `onRetry`/`onRefreshParent` için `controller.retry(id)` ve `controller.refresh(parent)` (`state.parent.get(id)`; ebeveyn `ROOT` ise `reloadServers`) bağlar; `not_found` rozetinin metni "Nesne artık mevcut değil. Üst düğümü yenilemeyi deneyin." `strings.ts`: `error: { badge: "Hata ayrıntısı", retry: "Tekrar dene", refreshParent: "Üst düğümü yenile", gone: "Nesne artık mevcut değil." }`.

Başarı/başarısızlık sonucu toast'a taşınmaz (rutin hata yerinde görünür); yalnızca kullanıcı eylemleri (Task 6) toast üretir.

- [ ] **Step 4: Doğrula ve commit**

Run: `bun run test && bun run typecheck`
Expected: PASS. Elle: sunucuyu durdurup genişletin → rozet, kart, Tekrar dene; başka oturumda tabloyu silip yenilemeden seçin → "nesne artık mevcut değil".

```bash
git add src
git commit -m "feat(explorer): hata rozeti kartı, Tekrar dene ve NotFound işaretleme"
```

---

### Task 8: "Sistem nesnelerini göster" ayarı

**Files:**
- Modify: `src/features/explorer/controller.ts`, `src/app/Toolbar.tsx`, `src/app/App.tsx`, `src/app/strings.ts`
- Test: `src/features/explorer/controller.test.ts` (ekleme)

**Interfaces:**
- Consumes: `commands.setShowSystem` (Task 1), `readStorage/writeStorage`.
- Produces: `ExplorerController.reloadAll(): Promise<void>`; localStorage anahtarı `pgt.showSystem` (`"1"`/`"0"`, varsayılan `"0"`).

Davranış: Toolbar'da aç/kapa düğmesi (lucide `Eye`/`EyeOff`, `aria-pressed`, başlık "Sistem nesnelerini göster (pg_catalog, information_schema, pg_* roller…)"). Uygulama açılışında **ilk yüklemeden önce** `setShowSystem(saklı değer)` çağrılır. Değişince: `setShowSystem(yeni)` → `reloadAll()` → toast yok (ağaç yenilenir).

`reloadAll`: yüklü (`children` anahtarı olan) her düğümü, **derinliğe göre artan** sırayla yeniden yükler; yalnızca bağlı bir sunucunun altındakileri (ve ROOT/grupları) — bağlantısız sunucunun altındaki yüklü listeler zaten boşaltılmıştır. Sıralı çalışır (aynı sunucuya paralel yük bindirmez). Bir yükleme hatası sonrakileri durdurmaz (hata ilgili düğümde rozet olarak kalır).

- [ ] **Step 1: Başarısız testler**

```ts
it("reloadAll reloads every loaded list, parents first", async () => {
  const { c, listChildren } = setup({ [ROOT]: [n("a")], a: [n("a1")], a1: [n("a11")] });
  await c.load(ROOT);
  await c.expand("a");
  await c.expand("a1");
  listChildren.mockClear();
  await c.reloadAll();
  expect(listChildren.mock.calls.map(([id]) => id)).toEqual([ROOT, "a", "a1"]);
});

it("reloadAll skips disconnected servers", async () => {
  const srv = n("srv", { kind: "server", serverId: "u1", flags: { ...flags, connected: true } });
  const { c, get, listChildren } = setup({ [ROOT]: [srv], srv: [n("dbs")] });
  await c.load(ROOT);
  await c.expand("srv");
  c.disconnect("srv");
  listChildren.mockClear();
  await c.reloadAll();
  expect(listChildren.mock.calls.map(([id]) => id)).toEqual([ROOT]);
  expect(get().errors.size).toBe(0);
});

it("reloadAll keeps expansion and selection of surviving nodes", async () => {
  const { c, get } = setup({ [ROOT]: [n("a")], a: [n("a1")] });
  await c.load(ROOT);
  await c.expand("a");
  c.select("a1");
  await c.reloadAll();
  expect([get().expanded.has("a"), get().selected]).toEqual([true, "a1"]);
});

it("reloadAll continues after a failing load", async () => {
  let failA = false;
  const listChildren = vi.fn(async (id: string) => {
    if (id === "a" && failA) throw err("query");
    return id === ROOT ? [n("a"), n("b")] : id === "b" ? [n("b1")] : id === "a" ? [n("a1")] : [];
  });
  const { c, get } = setup({}, { api: { listChildren, connectServer: vi.fn(), disconnectServer: vi.fn() } });
  await c.load(ROOT);
  await c.expand("a");
  await c.expand("b");
  failA = true;
  await c.reloadAll();
  expect(get().errors.get("a")?.code).toBe("query");
  expect(get().children.get("b")).toEqual(["b1"]);
});
```

- [ ] **Step 2: Uygula**

```ts
async reloadAll(): Promise<void> {
  const depthOf = (id: string): number => {
    let d = 0;
    for (let cur = this.get().parent.get(id); cur !== undefined && cur !== ROOT; cur = this.get().parent.get(cur)) d++;
    return d;
  };
  const ids = [...this.get().children.keys()].sort((a, b) => (a === ROOT ? -1 : b === ROOT ? 1 : depthOf(a) - depthOf(b)));
  for (const id of ids) {
    if (id !== ROOT) {
      const node = this.get().nodes.get(id);
      if (!node) continue; // önceki yenilemede kalktı
      const server = serverOf(this.get(), id);
      if (server && !server.flags.connected) continue;
    }
    if (this.get().children.has(id)) await this.load(id);
  }
}
```

`load` hata yakalayıp düğüme yazar (fırlatmaz); yine de döngü `try/catch` ile korunur. `App.tsx`'te `showSystem` sinyali, `onMount` başında `await commands.setShowSystem(showSystem()).catch(() => {})`.

- [ ] **Step 3: Doğrula ve commit**

Run: `bun run test && bun run typecheck`
Expected: PASS. Elle: anahtar açılınca `pg_catalog`/`information_schema` şemaları ve `pg_*` roller görünür; kapanınca kaybolur; tercih yeniden başlatmada korunur.

```bash
git add src
git commit -m "feat(explorer): sistem nesnelerini göster ayarı"
```

---

### Task 9: Durum çubuğu, tembel diyaloglar ve devreden küçük işler

**Files:**
- Modify: `src/app/StatusBar.tsx`, `src/app/App.tsx`, `src/app/strings.ts`, `src/features/explorer/tree-model.ts` (yardımcı yoksa)
- Test: `src/app/status.test.ts` (saf `statusText`), `src/features/explorer/tree-model.test.ts` (ekleme)

**Interfaces:**
- Produces: saf `statusText(s: TreeState): string` (`src/app/status.ts`); `App` içinde `ServerDialog`, `ImportDialog`, `PasswordPrompt` ilk açılışa kadar yüklenmez.

Durum çubuğu metni (§6.1): `pg-prod-01 · PostgreSQL 16.4 · app · bağlı` — veritabanı adı, seçili düğümün `database` atası varsa eklenir (`ancestorOfKind(s, selected, ["database"])`, seçili düğüm `database` ise kendisi); sunucu yoksa `Hazır`; bağlantısız sunucuda sürüm olmaz: `pg-prod-01 · bağlı değil`.

- [ ] **Step 1: Başarısız testler**

```ts
it.each([
  [null, "Hazır"],
  ["srv-up", "pg-prod · PostgreSQL 16.4 · bağlı"],
  ["db-app", "pg-prod · PostgreSQL 16.4 · app · bağlı"],
  ["tbl-orders", "pg-prod · PostgreSQL 16.4 · app · bağlı"],
  ["srv-down", "pg-down · bağlı değil"],
])("statusText(%s)", (selected, expected) => {
  expect(statusText({ ...fixtureTree(), selected })).toBe(expected);
});
```

- [ ] **Step 2: Uygula**

`status.ts` `statusText`'i yazar (`t.status.*` kullanır); `StatusBar` yalnızca `<span class="truncate">{statusText(state())}</span>` gösterir (Task 3'te `notice` zaten kaldırıldı).

Tembel diyaloglar (`App.tsx`):
```tsx
const ServerDialog = lazy(() => import("../features/servers/ServerDialog").then((m) => ({ default: m.ServerDialog })));
const ImportDialog = lazy(() => import("../features/servers/ImportDialog").then((m) => ({ default: m.ImportDialog })));
```
Her biri `<Show when={everOpened()}><Suspense>…</Suspense></Show>` içinde; `everOpened` ilk açılışta `true` olur ve kalıcıdır (kapanış animasyonu/duruma zarar vermemek için bir kez yüklendikten sonra bileşen kalır). `PasswordPrompt` parola gerektiğinde yüklenir: `createPasswordPrompt` küçük olduğundan şimdilik bırakılabilir; boyut etkisi Task 10'da ölçülür ve ölçüm anlamlıysa (> 20 KB) aynı kalıpla tembelleştirilir.

- [ ] **Step 3: Doğrula ve commit**

Run: `bun run test && bun run typecheck && bun run build`
Expected: PASS; `dist` içinde diyaloglar ayrı parça (chunk) olarak üretilir (`ls dist/assets`).

```bash
git add src
git commit -m "feat(ui): durum çubuğunda veritabanı adı; diyaloglar tembel yüklenir"
```

---

### Task 10: Yayın exe doğrulaması, ölçümler, PG 13/15 matrisi ve belgeler

**Files:**
- Create: `scripts/smoke-release.ps1`
- Modify: `scripts/test-matrix.ps1` (varsayılan sürümler), `docs/performance.md`, `docs/smoke-test.md`, `docs/testing.md`, `docs/superpowers/specs/2026-10-05-object-explorer-design.md` (yalnızca "Durum" satırı: "Uygulandı")

**Interfaces:**
- Consumes: `scripts/measure.ps1` (Plan 2), `target\release\pg-tools.exe`, Docker.
- Produces: `scripts/smoke-release.ps1 [-Exe path] [-Seconds 6]`: çıkış kodu 0 = yayın exe'si başladı, ana pencere oluştu ve `-Seconds` sonunda hâlâ çalışıyor; ≠ 0 = çöktü/pencere yok.

- [ ] **Step 1: Yayın duman testi betiği**

`smoke-release.ps1`: geçici `APPDATA`/`LOCALAPPDATA` ile exe'yi başlatır (`measure.ps1`'deki izolasyon kalıbı), `MainWindowHandle` oluşana kadar en fazla 10 sn bekler (yoksa hata), `-Seconds` boyunca süreci izler (erken çıkarsa `ExitCode` ile hata), WebView2 alt süreçlerini de dahil ederek tüm ağacı kapatır, `%LOCALAPPDATA%\pg-tools\logs` altında `panicked` / `stack overflow` metni varsa hata verir. Parola/bağlantı bilgisi kullanılmaz. Betik **Plan 2 Task 10'da yakalanan çökmeyi** (release'de ~1 sn sonra yığın taşması) yeniden üretmeyecek kadar uzun izler (varsayılan 6 sn).

- [ ] **Step 2: Derle ve doğrula**

Run (sırayla):
```bash
bun run build
cargo build --release -p pg-tools
powershell -NoProfile -File scripts/smoke-release.ps1
powershell -NoProfile -File scripts/measure.ps1 -Runs 5
```
Expected: smoke 0 ile çıkar; ölçüm: boşta toplam özel çalışma kümesi < 150 MB, Rust süreci < 30 MB, pencere süresi < 1 sn. Hedef aşılırsa **yeni tembel yükleme/bayrak denemeden önce** `docs/performance.md`'deki karar kuralını uygulayın ve sonucu yazın; kuralda yazılı olmayan bir iyileştirme gerekiyorsa görevi `BLOCKED` olarak bildirin.

- [ ] **Step 3: Kurulum paketi boyutu**

Run: `bun run tauri build` (NSIS). `target\release\bundle\nsis\*.exe` boyutunu ölçün; hedef < 15 MB (§1.3, WebView2 gömülü olmayan varyant). Aşılırsa nedenini (`dist` ve exe boyutu) `docs/performance.md`'ye yazın; düzeltmeye çalışmayın (kapsam dışı), yalnızca raporlayın.

- [ ] **Step 4: Sürüm matrisini 13 ve 15 ile genişlet**

`scripts/test-matrix.ps1`: `$Versions` varsayılanı `@("10", "11", "12", "13", "14", "15", "16", "17", "18")`; `.PARAMETER Versions` açıklaması güncellenir; sabit port eşlemesi varsa (ör. `55400 + sürüm`) çakışmayı doğrulayın. Çalıştırın:

Run: `powershell -NoProfile -File scripts/test-matrix.ps1 -SkipPerf`
Expected: 9 sürümün hepsinde PASS. 13 veya 15'te kırmızı çıkan test varsa **kök nedeni sürüm kapılaması olarak** (`.pgNN.sql` varyantı / `VersionedSql` eşiği) düzeltin; DDL çıktısı farklıysa `@pg13`/`@pg15` snapshot'ı üretin (`INSTA_UPDATE=new`, `.snap.new` → `.snap`, `assertion_line:` satırını silin; cargo-insta kurulu değildir). Sürüm-dışı bir sorun çıkarsa `DONE_WITH_CONCERNS` ile bildirin.

- [ ] **Step 5: Belgeler**

`docs/performance.md`: Plan 3 ölçümleri (açılış, boşta özel çalışma kümesi, Rust süreci, `dist` ve exe boyutu, NSIS boyutu), filtre ölçümü (Task 5 testinin değeri), tarih. `docs/smoke-test.md`: Plan 3 elle kontrol listesi — pgAdmin içe aktarma (yinelenen/hatalı/SSH'li), Ctrl+F filtre, sağ tık menüsü (sunucu/tablo/kolon/fonksiyon), sunucu silme (bağlıyken), hata rozeti + Tekrar dene, `NotFound`, sistem nesneleri anahtarı, durum çubuğu; her satırda beklenen sonuç. `docs/testing.md`: `smoke-release.ps1` kullanımı, matris sürüm listesi (9), filtre perf testi, içe aktarmanın "önizleme–uygulama arasında dosya değişirse" sınırı. Spec başlığındaki "Durum"u "Uygulandı (Plan 1–3)" yapın.

- [ ] **Step 6: Doğrula ve commit**

Run: `cargo test --workspace --all-features`, `bun run test`, `bun run typecheck`
Expected: PASS

```bash
git add scripts docs
git commit -m "chore: yayın duman testi, PG 13/15 matrisi, Plan 3 ölçümleri ve belgeler"
```
