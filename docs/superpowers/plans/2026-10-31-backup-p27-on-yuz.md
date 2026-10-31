# Yedekleme / Geri Yükleme — Ön Yüz (Plan 27) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Explorer'dan **"Yedekle…"** (veritabanı, şema, tablo, MV, sequence) ve **"Geri yükle…"** (veritabanı, bağlı sunucu) diyalogları. Diyaloglar araç durumunu ve araç ayarlarını (bin klasörü, bulunan araçlar, yeniden tara), seçenekleri, komut önizlemesini, risk/onay kapısını, canlı ilerlemeyi (çubuk + bayt + süre + satır günlüğü + iptal) ve sonucu ("Klasörü göster" dahil) içerir. Plan 26 arka ucunu uçtan uca kullanılabilir yapar.

**Mimari:**
- Ana pakette yalnız şunlar bulunur: iki menü eylemi (`backup`, `restore`), tür kümesi, iki menü metni, `commands.ts` sarıcıları, App'teki tek sinyal ve tembel yükleyici.
- Tembel parça tek giriştir: `BackupDialogs.tsx` kipe göre `BackupDialog` ya da `RestoreDialog` çizer. Durum `backup-controller.ts`'tedir (enjekte api, saf testli). Kurallar ve metin üreticileri saf `backup-model.ts`'tedir. Metinler `src/app/strings-backup.ts`'tedir (yalnız tembel parça içe aktarır).
- İş akışı: `backup_tools` → gecikmeli `backup_plan`/`restore_plan` → `backup_start`/`restore_start` (Tauri `Channel<BackupEvent>`) → sonuç. İptal `backup_cancel(jobId)`.
- Tek arka uç ekleme: `backup_reveal(path)` ("Klasörü göster"; gerekçe Kararlar 9).

**Teknoloji:** SolidJS, Kobalte (yalnız tembel; mevcut `ui/Dialog`), `@tauri-apps/plugin-dialog` (mevcut; `open`/`save`), vitest; Rust yalnız `src-tauri/src/backup.rs` + `main.rs`.

**Spec:** `docs/superpowers/specs/2026-10-18-backup-restore-design.md`. Bağlayıcı olanlar: §3 arayüz özeti ve **§5 Plan 26 sözleşme notları** (komutlar, istek türleri, olaylar, hata kodları, plan alan yolları, ipuçları, bilinen sınırlar). Paket kuralları: `docs/testing.md` "Ana paket bütçesi". Emsal: Plan 25 (`docs/superpowers/plans/2026-10-18-maintenance-p25-on-yuz.md`, `src/features/maintenance/*`).

## Kararlar

1. **Menü:**
   - Eylem kimlikleri `backup` ve `restore`. Adları `Object` ile bitmez (designer kipi değildir). Grup 3 (`groupOf`'a açık kural).
   - `backup`: `BACKUP_KINDS` = `database`, `schema`, `table`, `materializedView`, `sequence`. Sistem nesnesinde (`flags.system`) ve `partitionCollection` altındaki düğümlerde görünmez: bölümün şeması atadan güvenle okunamaz (`qualifiedParts` ile aynı kural). Kilitli sunucuda **açıktır** (yedekleme salt okunurdur).
   - `restore`: `database` düğümünde ve **bağlı** sunucu düğümünde görünür. Kilitli sunucuda devre dışıdır, gerekçe `t.query.hint.locked`.
   - Nesne sırası: `editObject`, `renameObject`, `privilegesObject`, `defaultPrivilegesObject`, `maintenance`, `backup`, `restore`, `dropObject`. Bağlı sunucu: `…q, refresh, disconnect, restore, editServer, deleteServer, copyName`.
2. **Tek diyalog örneği:**
   - App'te tek sinyal `backupJob: { mode: "backup" | "restore"; nodeId: string } | null` vardır. Aynı anda tek diyalog ve dolayısıyla tek iş çalışır.
   - Bu, spec §5 bilinen sınır (f)'yi ("aynı veritabanına eşzamanlı iki geri yükleme ön yüzce engellenmeli") karşılar.
   - Diyalog kalıcıdır (modal). Çalışırken kapatma "İş sürüyor; iptal edilsin mi?" sorar: Evet → iptal, sonuç beklenir, sonra kapanır.
3. **Araç ayarları diyaloğun içindedir.** Uygulamada genel bir Ayarlar ekranı yoktur; spec §3'teki "Ayarlar" maddesi iki diyalogda ortak bir **"Araç ayarları"** bölümü (`ToolsPanel.tsx`) olarak uygulanır. İçeriği:
   - Mevcut `pgBinDir` (`settings_get`).
   - "Klasör seç…": `open({ directory: true })` → `settings_set_pg_bin_dir`.
   - "Temizle": `null`.
   - "Yeniden tara": `backup_tools`.
   - Bulunan kümeler tablosu: klasör, sürüm, pg_dump/pg_restore/psql varlığı, seçili işareti.
   - `problem` metni.

   Bölüm kapalı başlar; özet satırı seçili aracı ve sürümü gösterir. Seçili araç yoksa ya da `problem` doluysa açık başlar. Arka uç iletilerindeki "Ayarlardan … bin klasörü seçin" bu bölüme denk gelir. Ayar değişince araçlar yeniden taranır ve plan yeniden istenir.
4. **Sürüm uyarıları (spec §5 bilinen sınır (a)):** sunucu ana sürümü, sunucu düğümünün `detail` metninden (`ServerInfo.version`) okunur. Seçili aracın sürümü `ToolDetection.sets`'te seçili klasörün kümesinden alınır.
   - Ana sürüm farklıysa bilgi: "Sunucuyla aynı ana sürümden (M) araç önerilir; seçili: N."
   - Geri yüklemede `pg_restore` ≥ 17 ve sunucu < 17 ise uyarı (metin Task 2'de).
   - `psql` için sürüm uyarısı yoktur (spec: sürüm önemsiz). Düz SQL ile ilgili uyarıyı arka uç plan notu ve ipucu 4 verir.
5. **Hedef yolu yalnız yerel diyalogla seçilir.**
   - Yol alanı salt okunurdur. Üzerine yazma onayını Windows kayıt diyaloğu verir (spec §1 "üzerine yazma onayı ön yüzde istenir").
   - Dört formatta da `save()` kullanılır. Directory formatında kullanıcı yeni bir klasör adı yazar; boş ya da var olmayan hedef kuralını arka uç denetler.
   - Önerilen ad `<db>[_<şema>[_<ad>]]_<YYYYMMDD>_<HHmm><uzantı>`. Uzantılar: custom `.backup`, plain `.sql` (sıkıştırma > 0 ise `.sql.gz`), tar `.tar`, directory yok.
   - Başarılı yedekten sonra yol temizlenir. Aynı hedefe ikinci yedek ancak yeniden seçimle (ve yerel üzerine yazma onayıyla) alınır.
6. **Geri yükleme kaynağı ve hedefi:**
   - Kaynak "Dosya seç…" ya da "Klasör seç…" ile seçilir; ardından `restore_inspect` çağrılır.
   - Hedef radyosu:
     - **Mevcut veritabanı:** veritabanı düğümünden açıldıysa o veritabanıdır. Sunucu düğümünden açıldıysa seçicidir: `listChildren(sunucu)` → `databaseCollection` → `listChildren(koleksiyon)`; seçimde `ddlTargetOf(düğüm)` ile oid alınır.
     - **Arşivdeki adla oluştur:** arşivdeki ad gösterilir.
   - Varsayılan hedef: veritabanı düğümünde `existing`, sunucu düğümünde `create_from_archive`. Düz SQL'de `create_from_archive` gerekçeyle devre dışıdır.
   - `RestoreOptions.create` arayüzde ayrı kutu değildir; `target === "create_from_archive"` ile eşit gönderilir (arka uç `--create`'i hedeften türetir; spec §5).
7. **Onay kapısı (geri yükleme):** plan riski `destructive` → onay kutusu. `data_loss` + üretim sunucusu → kutu + sunucu adını yazma (designer `applyGate` kuralının küçük kopyası; büyük/küçük harf duyarlı, kırpılmış tam eşleşme). Plan sürümü değişince onay sıfırlanır.
   - Üretim bilgisi `isProductionLabel(ServerConfig.label)`'dan, ad `ServerConfig.name`'den gelir (`query.servers()`; emsal `data-opener.ts`).
   - Kilit, ağaçtaki sunucu düğümünün `flags.locked` değerinden canlı okunur (bakım diyaloğuyla aynı tek kaynak).
8. **Bağlantı ve parola:**
   - Açılışta `deps.connect(serverId)` çağrılır (gerekirse parola istemi). `false` dönerse yükleme hatası ve "Yeniden dene" gösterilir.
   - Plan çağrısı `not_connected` dönerse bir kez `connect` + yeniden plan denenir.
   - `password_required` (yalnız oturum parolasız kaldıysa; spec §5 (c)) yeni bağlantı tesisatı gerektirmez. İletiye sabit yönlendirme eklenir: "Sunucu bağlantısını kesip yeniden bağlanın; parola istenir."
9. **"Klasörü göster" için küçük arka uç komutu:** mevcut eklentilerde (yalnız `dialog`) dosyayı Gezgin'de seçili gösterme yoktur. `tauri-plugin-opener` eklemek yeni bir Rust ve JS bağımlılığı ile yetki demektir. Bunun yerine `backup_reveal(path)` eklenir:
   - Yol mutlak olmalı ve var olmalıdır; `"` ve denetim karakteri reddedilir.
   - Windows'ta `%SystemRoot%\explorer.exe` (PATH'ten değil), `raw_arg("/select,\"<yol>\"")` ile başlatılır ve beklenmez. `/select` dosyayı çalıştırmaz, yalnız seçer.
   - Windows dışında `invalid` döner.
   - Yol günlüğe yazılmaz. ts-rs türü yoktur (binding değişmez).
10. **İş sonrası:**
    - Geri yükleme `success` ya da `completed_with_errors` ile biterse Explorer yenilenir: mevcut hedefte veritabanı düğümü, oluşturmada sunucu düğümü. Ardından `refreshInspector`.
    - Yedeklemeden sonra yenileme yoktur.
    - Sonuçtan sonra seçenekler yeniden düzenlenebilir ve yeni `jobId` ile yeniden başlatılabilir.
11. **Kapsam dışı:** arka planda iş sürdürme (diyalog kapalıyken), birden çok eşzamanlı iş, günlüğü kopyalama/kaydetme, TOC seçimi, `pg_dumpall`, bölüm (partition) düğümünden yedek, bölümlenmiş üst tablonun alt bölümleriyle yedeklenmesi (`-t` yalnız üst tabloyu alır; PG 16+ `--table-and-children` yok).

- **Kullanıcı onayı (2026-10-31):** modal diyalog ilk sürüm için kabul; "arka planda sürdür" kapsam dışı. Paket telafisi yetmezse tavan yükseltilmez, kullanıcıya sorulur.
## Global Constraints

- **Dal:** `feat/backup-ui` (main'den). main'e birleştirme yalnız kullanıcı onayıyla.
- **Değişen alanlar:**
  - Ön yüz (`src/`), `docs/`, `scripts/bundle-budget.json`.
  - Rust'ta **yalnız** Kararlar 9 (`src-tauri/src/backup.rs`, `src-tauri/src/main.rs`). Rust testleri: `cargo test -p pg-tools reveal`, `cargo clippy -p pg-tools --all-targets -- -D warnings`.
  - Crate genelinde `cargo fmt` çalıştırılmaz; dokunulan satırlar elle biçimlendirilir.
- **Sözleşme:** komut ve alan adları spec §5 ile birebir (camelCase bağımsız değişkenler, `onEvent` kanalı, `jobId = crypto.randomUUID()`).
  - Arka uç tek doğruluk kaynağıdır. İstemci kuralları yalnız görünürlük ve devre dışılık içindir; plan `errors` alanı her zaman gösterilir.
  - `errors` doluysa başlat kapalıdır.
- **Paket:**
  - Ana paket ≤ `mainMaxBytes` (şu an 128,466 B), açılış kümesi ≤ `startupMaxBytes` (156,139 B). Preload tam `["solid","web"]`; açılış kümesinde yasak paket yok.
  - Task 1–4'te `bun run bundle:check` yalnız bu iki tavanın aşımı yüzünden düşebilir. Aşım rapora yazılır; başka her ihlal (preload, yasak paket, harita) görevi durdurur. Telafi ve tavan indirme Task 5'tedir.
  - Tembel parça ana modülleri (`app/strings`, `explorer/*`, `tree-model`, `designer/*`, `maintenance/*`) içe aktarmaz. Metin `strings-backup.ts`'ten, api/ağaç/bildirim App'ten props ile gelir. İzinli paylaşılan içe aktarmalar: `ui/*`, `ipc/*` türleri, `servers/server-form` (`isProductionLabel`; emsal `data-opener.ts`).
  - Yükleyici: `export const loadBackupDialogs = () => import("../features/backup/BackupDialogs")` (App.tsx), `prefetchIdle` listesine eklenir.
- **Gizlilik:**
  - Komut önizlemesi, satırlar, yollar, adlar ve parola `console.*`'a, `localStorage`'a ya da bildirimlere (toast) yazılmaz.
  - Önizleme yalnız arka ucun parolasız metnidir (`PGPASSWORD=***`); ön yüz parola görmez ve üretmez.
- **Erişilebilirlik:**
  - Başlık ve odak tuzağı `ui/Dialog` ile sağlanır. Seçenekler etiketlidir; devre dışı gerekçeleri `aria-describedby` ile bağlanır.
  - İlerleme `role="progressbar"`: `aria-valuenow/max` ya da belirsizse `aria-valuetext`.
  - Günlük `aria-live="off"` bölgesidir (2000 satır duyurulmaz). Sonuç `role="status"`, hata `role="alert"` ile duyurulur.
  - Enter başlatır (kapı açıksa, IME bileşimi sırasında değil). Esc kapatır (çalışırken onay sorar).
- **vitest:** ortam `node`. Mantık `backup-model.ts` ve `backup-controller.ts`'tedir ve orada test edilir. GUI ajanlarca test edilemez; elle kontrol maddeleri `docs/smoke-test.md` "## Yedekleme arayüzü (Plan 27)" altına `[—]` olarak eklenir (Task 5).
- **Diğer:**
  - Yorumlar ve metinler Türkçe, doğru Türkçe karakterlerle. PostgreSQL/araç terimleri (`--clean`, `pg_restore`) olduğu gibi kalır.
  - Bindingler değişmez. Değişmiş görünürse yalnız satır sonudur: `git checkout -- src/ipc/bindings`.
  - `graphify-out/` commit edilmez.
  - Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Doğrulama (her görev sonu):** `bun run typecheck`, `bunx vitest run`, `bun run build`, `bun run bundle:check`.

## Review Focus

- **Format değişimi ve yol:** kullanıcı yolu seçtikten sonra formatı (ya da plain sıkıştırmasını) değiştirir.
  - Beklenen: yol bizim önerdiğimiz uzantıyla bitiyorsa uzantı yeni formata çevrilir; directory'ye geçişte uzantı atılır. Kullanıcının kendi uzantısı korunur.
  - Uyumsuz hedef (dosya ↔ klasör) arka uç `outputPath` hatası olarak yol alanının altında görünür.
  - Test: Task 2 `swapExtension`, Task 3 "format değişince yol uyarlanır ve plan yenilenir".
- **Çalışırken kapatma ve yarışlar:** Esc ya da × ile kapatma, iptal ile `finished`'ın aynı anda gelmesi, diyalog kapandıktan sonra gelen olay ya da sonuç.
  - Beklenen: kapatma onayı gösterilir; gösterilen sonuç arka ucun `outcome`'udur (iş bittiyse iptal düğmesi etkisizdir); kapalı diyaloğa gelen olay durumu bozmaz.
  - Test: Task 3.
- **Onayın bayatlaması:** onay kutusu işaretlendikten sonra `clean` açılır (risk destructive → data_loss) ya da hedef değişir.
  - Beklenen: plan sürümü ilerler, onay ve yazılan ad sıfırlanır. Üretimde ad yazılmadan başlatılamaz.
  - Test: Task 2 `restoreGate`, Task 3 "plan değişince onay sıfırlanır".
- **Büyük ve uzun günlük:** 2000 satır + "… N satır daha gösterilmedi", 4 KiB satırlar, `done > total`.
  - Beklenen: satırlar olduğu gibi tutulur ve taşma satırı da görünür. Çubuk %100'ü aşmaz. Uzun satır kaydırılır (yatay taşma yok).
  - Test: Task 2 `applyEvent`/`progressView`.
- **Türkçe/ASCII dışı ad ve yol:** `satış` şeması, `a"b` tablosu, `C:\Yedek\ş ı.backup`, `＂` içeren yol.
  - Beklenen: kapsam adları ağaçtan değiştirilmeden isteğe girer. Arka ucun ANSI ve `scope.name` iletileri ilgili alanın (yol ya da üst şerit) altında görünür, genel "başarısız" olarak değil. Önerilen dosya adı Windows'ta geçersiz karakterleri `_` yapar.
  - Test: Task 2 `backupScopeOf`/`splitFieldErrors`/`suggestFileName`.

---

### Task 1: Komut yüzeyi, Explorer girişleri ve tembel yükleyici

**Dosyalar:**
- Değiştir: `src-tauri/src/backup.rs` (`reveal_arg` + `backup_reveal` + birim test), `src-tauri/src/main.rs` (kayıt).
- Değiştir: `src/ipc/commands.ts`, `src/features/designer/ddl-kinds.ts` (`BACKUP_KINDS`), `src/features/explorer/actions.ts` (+ `actions.test.ts`), `src/app/strings.ts` (`t.menu.backup`, `t.menu.restore`), `src/app/App.tsx`.
- Oluştur: `src/features/backup/BackupDialogs.tsx` (iskelet: başlık + "Hazırlanıyor…" + Kapat; Task 4 gerçeğini yazar).

**Arayüzler — Üretir:**

```rust
// src-tauri/src/backup.rs
pub fn reveal_arg(path: &str) -> Result<String, &'static str>;   // Ok("/select,\"C:\\x\\y.backup\"")
#[tauri::command] pub fn backup_reveal(path: String) -> CmdResult<()>;   // invalid | not_found ("Dosya ya da klasör bulunamadı.")
```
```ts
// commands.ts (parametre adları spec §5 tablosu)
backupTools: (serverId: string) => call<ToolDetection>("backup_tools", { serverId }),
backupPlan: (serverId: string, databaseOid: number, request: BackupRequest) => call<JobPlan>("backup_plan", { serverId, databaseOid, request }),
backupStart: (serverId: string, databaseOid: number, request: BackupRequest, jobId: string, onEvent: (e: BackupEvent) => void) =>
  call<JobSummary>("backup_start", { serverId, databaseOid, request, jobId, onEvent: channel(onEvent) }),
backupCancel: (jobId: string) => call<null>("backup_cancel", { jobId }),
backupReveal: (path: string) => call<null>("backup_reveal", { path }),
restoreInspect: (path: string) => call<RestoreSource>("restore_inspect", { path }),
restorePlan: (serverId: string, databaseOid: number | null, request: RestoreRequest) => call<JobPlan>("restore_plan", { serverId, databaseOid, request }),
restoreStart: (serverId: string, databaseOid: number | null, request: RestoreRequest, jobId: string, confirmed: boolean, confirmedProduction: boolean, onEvent: (e: BackupEvent) => void) =>
  call<JobSummary>("restore_start", { serverId, databaseOid, request, jobId, confirmed, confirmedProduction, onEvent: channel(onEvent) }),
settingsSetPgBinDir: (dir: string | null) => call<AppSettings>("settings_set_pg_bin_dir", { dir }),
// ddl-kinds.ts
export const BACKUP_KINDS: ReadonlySet<NodeKind>;   // database, schema, table, materializedView, sequence
// actions.ts
type ActionId = … | "backup" | "restore";
interface ActionDeps { …; openBackup?(id: string, mode: "backup" | "restore"): void }
// App.tsx
export const loadBackupDialogs = () => import("../features/backup/BackupDialogs");
// BackupDialogs.tsx (Task 4'te de aynı imza)
export function BackupDialogs(props: { job: { mode: "backup" | "restore"; nodeId: string } | null; deps: BackupDialogDeps; onClose: () => void }): JSX.Element;
```
- App, `<BackupDialogs job={backupJob()} deps={{ ...lazyDeps(), explorer: explorer.controller }} onClose={() => setBackupJob(null)} />` çizer. İlk açılışa dek `Show when={everOpened}` + `Suspense` kullanılır (bakım diyaloğu deseni).
- `BackupDialogDeps` türü `BackupDialogs.tsx`'te tanımlanır: `lazyDeps()` alanları + `explorer: Pick<ExplorerController, "refresh">` (yalnız tür içe aktarımı).

- [ ] **Adım 1: Başarısız testler.**
  - `actions.test.ts`:
    - `backup` beş türde var.
    - `backup` şu durumlarda yok: koleksiyonlar, `index`, `view`, `function`, `role`, `column`; `flags.system` şema; `partitionCollection` altındaki tablo.
    - `restore` `database`'de ve bağlı sunucuda var; bağlı olmayan sunucuda ve şemada yok.
    - Sıralar Kararlar 1'deki gibidir; `groupOf("backup") === groupOf("restore") === 3`.
    - Kilitli sunucuda `disabledReason("restore") === t.query.hint.locked`, `disabledReason("backup") === null`.
    - `executeAction` `openBackup(id, "backup")` ve `openBackup(id, "restore")` çağırır. Kilitte `restore` çağrılmaz.
    - `t.menu.backup === "Yedekle…"`, `t.menu.restore === "Geri yükle…"`.
  - Rust `backup.rs` testleri:
    - `reveal_arg("C:\\Yedek\\ş ı.backup") == Ok("/select,\"C:\\Yedek\\ş ı.backup\"")`.
    - Göreli yol `Err("Tam (mutlak) yol gerekir.")`.
    - `"` ya da `\n` içeren yol `Err("Yol geçersiz karakter içeriyor.")`.
    - Mutlaklık denetimi `#[cfg(windows)]` altındadır.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bunx vitest run src/features/explorer/actions.test.ts` → FAIL (`backup` yok). `cargo test -p pg-tools reveal` → derleme hatası (`reveal_arg` yok).
- [ ] **Adım 3: Uygula.**
  - Kararlar 1 ve 9.
  - `executeAction`'da `case "backup": case "restore":` → `if (!disabledReason(…)) deps.openBackup?.(id, action)`.
  - `prefetchIdle` listesine `loadBackupDialogs` eklenir.
- [ ] **Adım 4: Doğrula.** `cargo test -p pg-tools reveal` PASS, `cargo clippy -p pg-tools --all-targets -- -D warnings` temiz. Ardından dört ön yüz komutu çalıştırılır; `bundle:check` ana paket ve açılış kümesi baytları rapora yazılır (aşım Task 5'te telafi edilir).
- [ ] **Adım 5: Commit.**
  ```bash
  git add src-tauri/src/backup.rs src-tauri/src/main.rs src/ipc/commands.ts src/features/designer/ddl-kinds.ts src/features/explorer/actions.ts src/features/explorer/actions.test.ts src/app/strings.ts src/app/App.tsx src/features/backup/BackupDialogs.tsx
  git commit -m "feat(backup): Explorer \"Yedekle…/Geri yükle…\" menüsü, komutlar ve tembel yükleyici"
  ```

### Task 2: Saf model ve metinler

**Dosyalar:** Oluştur `src/features/backup/backup-model.ts`, `src/features/backup/backup-model.test.ts`, `src/app/strings-backup.ts` (`export const tb = { … } as const`).

**Arayüzler — Üretir:**

```ts
export type OptionState = { visible: boolean; enabled: boolean; reason: string | null };
export type BackupKey = keyof BackupOptions;   // encoding dahil
export type RestoreKey = Exclude<keyof RestoreOptions, "create">;
export function defaultBackupOptions(): BackupOptions;     // tüm bool false; compress/jobs/encoding null
export function defaultRestoreOptions(): RestoreOptions;   // tüm bool false; jobs null
export function backupOptionStates(format: DumpFormat, o: BackupOptions): Record<BackupKey, OptionState>;
export function normalizeBackup(format: DumpFormat, o: BackupOptions): BackupOptions;   // görünmez/devre dışı kalanı temizler
export function restoreOptionStates(src: SourceFormat, target: RestoreTarget, o: RestoreOptions): Record<RestoreKey, OptionState>;
export function normalizeRestore(src: SourceFormat, target: RestoreTarget, o: RestoreOptions): RestoreOptions;   // create = target === "create_from_archive"
export function backupScopeOf(tree: { nodes: Map<string, TreeNode>; parent: Map<string, string> }, nodeId: string):
  { scope: BackupScope; dbNodeId: string; dbName: string; label: string } | null;   // bölüm altı / bilinmeyen tür → null
export function fileExt(format: DumpFormat, compress: number | null): string;      // ".backup" | ".sql" | ".sql.gz" | ".tar" | ""
export function suggestFileName(parts: string[], format: DumpFormat, compress: number | null, now: Date): string;
export function swapExtension(path: string, from: { format: DumpFormat; compress: number | null }, to: { format: DumpFormat; compress: number | null }): string;
export function saveFilters(format: DumpFormat, compress: number | null): { name: string; extensions: string[] }[];
export function splitFieldErrors(errors: FieldError[], fields: readonly string[]): { byField: Record<string, string>; general: string[] };
export function toolAdvice(tool: ToolVersion | null, serverMajor: number | null, job: "backup" | "restore" | "psql"): { tone: "info" | "warning"; text: string } | null;
export function chosenTool(det: ToolDetection, which: "dump" | "restore" | "psql"): { dir: string; version: ToolVersion } | null;
export function serverMajor(detail: string | null): number | null;   // "16.4" → 16, "9.6.24" → 9, "18beta1" → 18, null/çözülemez → null
export type LogLine = { severity: LineSeverity; text: string };
export interface JobLog { preview: string | null; lines: LogLine[]; progress: { done: number; total: number | null; bytes: number | null } | null; summary: JobSummary | null }
export function emptyLog(): JobLog;
export function applyEvent(log: JobLog, ev: BackupEvent): JobLog;   // finished → summary (son olay; sonrakiler yok sayılır)
export function progressView(p: JobLog["progress"], job: "backup" | "restore"): { percent: number | null; text: string };
export function formatBytes(n: number): string;
export function elapsedLabel(ms: number): string;   // mm:ss, ≥ 1 sa ise h:mm:ss
export type ResultTone = "success" | "warning" | "error" | "neutral";
export function resultView(s: JobSummary, job: "backup" | "restore", singleTransaction: boolean): { tone: ResultTone; title: string; lines: string[] };
export type Gate =
  | { kind: "blocked"; reason: keyof typeof tb.blocked }
  | { kind: "ready" } | { kind: "needsConfirm" } | { kind: "needsProductionName"; expected: string };
export function backupGate(i: { running: boolean; hasPath: boolean; pending: boolean; planCurrent: boolean; plan: JobPlan | null }): Gate;
export function restoreGate(i: { locked: boolean; running: boolean; inspecting: boolean; hasSource: boolean; hasTarget: boolean; pending: boolean; planCurrent: boolean; plan: JobPlan | null; production: boolean; serverName: string }): Gate;
export function canStart(g: Gate, c: { checked: boolean; typedName: string }): boolean;
export function confirmFlags(g: Gate, c: { checked: boolean; typedName: string }): { confirmed: boolean; confirmedProduction: boolean };
export function errorText(p: ErrorPayload): string;   // password_required → message + tb.passwordAdvice
```

- **Görünürlük ve dışlamalar (spec §1 + §5 alan iletileri):**
  - Yedek:
    - `clean`, `ifExists`, `inserts`, `columnInserts` yalnız plain'de görünür.
    - `ifExists` `clean` kapalıyken devre dışıdır: "--if-exists, temizle (--clean) seçeneğini gerektirir."
    - `compress` tar'da gizlidir.
    - `jobs` yalnız directory'de görünür.
    - `schemaOnly` ⟂ `dataOnly` ve `inserts` ⟂ `columnInserts`: biri açıkken diğeri devre dışıdır ve gerekçesi arka uç iletisinin aynısıdır.
    - `clean` ⟂ `dataOnly`.
    - `create` her formatta görünür (plain dışında açıklama "Arşive yazılır; geri yüklemede kullanılır.").
  - Geri yükleme:
    - Plain'de yalnız `singleTransaction` görünür.
    - `jobs` tar'da devre dışıdır ("Tar arşivinde paralel geri yükleme yapılamaz.") ve `singleTransaction` ile de devre dışıdır.
    - `singleTransaction` `create_from_archive`'da ve `jobs` doluyken devre dışıdır.
    - `ifExists` `clean` gerektirir. `schemaOnly` ⟂ `dataOnly`. `clean` ⟂ `dataOnly`.
- **Alan eşlemesi:**
  - `fields` görünür alan yollarıdır (`outputPath`, `path`, `target`, `options.<anahtar>`, `scope.name`, `scope.schema`).
  - Diğer tüm yollar `general`'a gider: `tool`, `database`, `user`, `host`, `request`, `ssl.*`, `ssl.rootCert` …
  - Aynı alanda ilk ileti kazanır.
- **Uyarı metinleri (`tb.advice`):**
  - Bilgi: `(M, N) => \`Sunucuyla aynı ana sürümden (${M}) araç önerilir; seçili: ${N}.\``
  - Geri yükleme uyarısı: `(N, M) => \`Seçili pg_restore ${N}, sunucu ${M}: araç sunucudan yeni olduğundan SET transaction_timeout satırı hata verir ("hatada dur" kapalıyken atlanır ve iş "tamamlandı, hatalarla" biter). Sunucuyla aynı ana sürümden (${M}) PostgreSQL istemci araçlarını kurup Araç ayarlarından seçin.\``
- **Sonuç metinleri:**
  - `success`: "Tamamlandı (mm:ss)".
  - `completed_with_errors`: "Tamamlandı, N hata — veri yüklenmiş olabilir".
  - `failed`: "Başarısız" + `hint`.
  - `cancelled`: "İptal edildi". Geri yüklemede `singleTransaction` kapalıysa ek satır: "Veritabanı yarım kalmış olabilir (tek transaction kapalıydı)."
  - `truncatedLines > 0` → "N satır gösterilmedi". `warnings`/`errors` sayıları satır olarak eklenir.
  - Ton eşlemesi: success → success, completed_with_errors → warning, failed → error, cancelled → neutral.

- [ ] **Adım 1: Başarısız testler (`backup-model.test.ts`).**
  - Görünürlük ve dışlama tabloları:
    - 4 format × her yedek anahtarı.
    - 4 kaynak formatı × 2 hedef × her geri yükleme anahtarı.
    - Gerekçe metinleri arka uç iletileriyle birebir.
  - `normalizeBackup`:
    - plain → custom geçişinde `clean/ifExists/inserts/columnInserts` false olur.
    - tar'da `compress` null, directory dışında `jobs` null olur.
  - `normalizeRestore`:
    - plain'de `singleTransaction` dışında hepsi false/null olur.
    - `create === (target === "create_from_archive")`.
  - `backupScopeOf`:
    - db → `{kind:"database"}`.
    - Şema `satış` → `{kind:"schema", name:"satış"}`.
    - Tablo `a"b` (şema `public`) → `{kind:"table", schema:"public", name:"a\"b"}`.
    - MV ve sequence → `table` kapsamı.
    - `partitionCollection` altı → null.
    - `dbName` veritabanı atasının etiketidir.
  - `suggestFileName(["satış","public",'a"b'], "custom", null, new Date(2026, 9, 31, 14, 5))` → `"satış_public_a_b_20261031_1405.backup"`. Ayrıca `<>:"/\|?*` ve denetim karakterleri `_` olur.
  - `fileExt`: plain + compress 0 → `.sql`; plain + 5 → `.sql.gz`; directory → `""`.
  - `swapExtension`:
    - `C:\a\x.backup` custom → plain = `C:\a\x.sql`.
    - plain 0 → plain 6 = `.sql.gz`.
    - custom → directory = `C:\a\x`.
    - directory → tar = `C:\a\x.tar`.
    - Kullanıcı uzantısı `C:\a\x.dmp` custom → plain değişmez.
    - Büyük harfli `.BACKUP` tanınır.
  - `splitFieldErrors`: `outputPath` ANSI iletisi `byField.outputPath`'e; `request`, `tool`, `database` `general`'e; aynı alanda iki ileti → ilki.
  - `serverMajor` örnekleri; `chosenTool` seçili klasörün kümesini bulur, yoksa null.
  - `toolAdvice`:
    - restore 18 / sunucu 16 → warning.
    - backup 18 / 16 → info.
    - eşit → null.
    - psql 18 / 16 → null.
    - sunucu null → null.
  - `applyEvent`:
    - started → preview.
    - 2001 satır (son satır "… 5 satır daha gösterilmedi") olduğu gibi tutulur.
    - progress son değeri tutar.
    - finished → summary; finished sonrası gelen olay yok sayılır.
  - `progressView`:
    - `{done: 7, total: 5}` → percent 100, metin "5 / 5 tablo".
    - total null → percent null, "7 tablo · 1,5 KB".
    - Geri yüklemede "öğe".
  - `formatBytes`: 0 → "0 B", 1023 → "1023 B", 1536 → "1,5 KB", 5·1024² → "5,0 MB", 3·1024³ → "3,0 GB".
  - `elapsedLabel`: 65000 → "01:05", 3_700_000 → "1:01:40".
  - `resultView`: dört sonuç + iptal/tek transaction satırı + `hint` + `truncatedLines`.
  - Kapılar:
    - `backupGate` sırası: running > noPath > pending > stale > invalid > ready.
    - `restoreGate` sırası: locked > running > inspecting > noSource > noTarget > pending > stale > invalid > risk.
    - `destructive` → needsConfirm.
    - `data_loss` + üretim → needsProductionName (`expected` = sunucu adı); üretim değilse needsConfirm.
    - `canStart`/`confirmFlags`: kırpılmış tam ad eşleşmesi, büyük/küçük harf duyarlı, boş beklenen ad asla eşleşmez.
  - `errorText({code:"password_required", …})` iletiye `tb.passwordAdvice` ekler; diğer kodlarda `message` (+ varsa `hint`).
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bunx vitest run src/features/backup/backup-model.test.ts` → FAIL (modül yok).
- [ ] **Adım 3: Uygula.** Metinler `tb`'de toplanır:
  - format adları (Custom, Düz SQL (plain), Directory, Tar);
  - seçenek etiketleri ve bayrakları ("Yalnız şema (-s)" …);
  - gerekçeler, kapı gerekçeleri (`tb.blocked`), sonuç ve ilerleme metinleri;
  - araç ayarları, onay ve kapatma metinleri;
  - `passwordAdvice`.
- [ ] **Adım 4: Doğrula.** Odak testi PASS; dört doğrulama komutu.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/backup/backup-model.ts src/features/backup/backup-model.test.ts src/app/strings-backup.ts
  git commit -m "feat(backup): yedekleme/geri yükleme saf modeli ve metinleri"
  ```

### Task 3: Denetleyiciler

**Dosyalar:** Oluştur `src/features/backup/backup-controller.ts`, `src/features/backup/backup-controller.test.ts`.

**Arayüzler:**
- Tüketir: Task 1 `commands` sarıcıları (`Api` üzerinden `Pick`), Task 2'nin tamamı.
- Üretir:

```ts
export const PLAN_DELAY_MS = 150;
export type Picker = {
  save(o: { defaultPath: string; filters: { name: string; extensions: string[] }[] }): Promise<string | null>;
  openFile(): Promise<string | null>;
  openDir(): Promise<string | null>;
};
export interface BackupControllerDeps {
  api: Pick<Api, "ddlTargetOf" | "ddlLookup" | "listChildren" | "backupTools" | "backupPlan" | "backupStart" | "backupCancel" | "backupReveal"
    | "restoreInspect" | "restorePlan" | "restoreStart" | "settingsGet" | "settingsSetPgBinDir">;
  tree: () => { nodes: Map<string, TreeNode>; parent: Map<string, string> };
  servers: () => ServerConfig[];
  connect: (serverId: string) => Promise<boolean>;
  refreshNode?: (nodeId: string) => void;
  refreshInspector?: () => void;
  notify: (text: string, kind: ToastKind) => void;
  pick: Picker;
  onClosed(): void;
  newId(): string; now(): number;
  setTimer(fn: () => void, ms: number): unknown; clearTimer(h: unknown): void;
}
export type Phase = "loading" | "loadError" | "ready" | "running";
interface Common {
  nodeId: string; serverId: string | null; serverName: string; production: boolean; serverMajor: number | null;
  phase: Phase; loadError: string | null;
  tools: ToolDetection | null; toolsError: string | null; pgBinDir: string | null; binDirError: string | null;
  version: number; plan: JobPlan | null; planFor: number; pending: boolean; planError: string | null;
  jobId: string | null; log: JobLog; startedAt: number | null; cancelRequested: boolean; startError: string | null;
  closeAsk: boolean; closing: boolean;
}
export interface BackupState extends Common {
  databaseOid: number | null; scope: BackupScope | null; scopeLabel: string; dbName: string;
  format: DumpFormat; options: BackupOptions; outputPath: string; encodings: string[];
}
export interface RestoreState extends Common {
  databaseOid: number | null; fromServer: boolean; dbNodeId: string | null; databases: { nodeId: string; label: string }[] | null;
  source: RestoreSource | null; inspecting: boolean; sourceError: string | null;
  target: RestoreTarget; options: RestoreOptions; confirm: { checked: boolean; typedName: string }; locked: boolean;
}
export function createBackupController(deps: BackupControllerDeps): {
  state: () => BackupState | null; gate: () => Gate;
  open(nodeId: string): void; close(): void; requestClose(): void; answerClose(cancel: boolean): void;
  setFormat(f: DumpFormat): void; setOption<K extends BackupKey>(k: K, v: BackupOptions[K]): void; chooseOutput(): Promise<void>;
  start(): Promise<void>; cancel(): void; reveal(): Promise<void>;
  rescanTools(): Promise<void>; chooseBinDir(): Promise<void>; clearBinDir(): Promise<void>;
};
export function createRestoreController(deps: BackupControllerDeps): {
  state: () => RestoreState | null; gate: () => Gate;
  open(nodeId: string): void; close(): void; requestClose(): void; answerClose(cancel: boolean): void;
  chooseSource(kind: "file" | "dir"): Promise<void>; setTarget(t: RestoreTarget): void; setTargetDb(nodeId: string): Promise<void>;
  setOption<K extends RestoreKey>(k: K, v: RestoreOptions[K]): void; setConfirm(c: Partial<{ checked: boolean; typedName: string }>): void;
  start(): Promise<void>; cancel(): void;
  rescanTools(): Promise<void>; chooseBinDir(): Promise<void>; clearBinDir(): Promise<void>;
};
```
- **Ortak kurallar** (maintenance-controller deseni):
  - Kuşak sayacı: aç/kapa kuşağı artırır; eski kuşağın yanıtı ve olayları durum yazmaz.
  - Her seçenek, format, yol, hedef ya da kaynak değişikliği `version`'ı artırır ve `PLAN_DELAY_MS` sonra plan ister. Yanıt yalnız `planFor === version` ise geçerlidir.
  - Geri yükleme onayı her `version` artışında sıfırlanır.
  - Araç ve ayar paylaşılan yardımcıdır. Değişiklikten sonra `backupTools` yeniden çağrılır ve plan yenilenir. `settingsSetPgBinDir` hatası `binDirError`'a yazılır.
- **Açılış:**
  - Yedek: `backupScopeOf` → `connect` → `ddlTargetOf(dbNodeId)` (oid) → paralel `backupTools`, `settingsGet`, `ddlLookup(…, {kind:"encodings"})` (lookup hatası yalnız listeyi boş bırakır) → plan.
  - Geri yükleme: veritabanı düğümünde oid; sunucu düğümünde `databases = null` ve `target = "create_from_archive"`. Seçici açılınca liste Kararlar 6'daki gibi tembel yüklenir.
- **Başlatma:**
  - Kapı (`canStart`) yeniden denetlenir. `jobId = newId()`, `phase = "running"`, `startedAt = now()`, `log = emptyLog()`.
  - Olaylar `applyEvent` ile işlenir. Sonuç `start` dönüşünden ya da `finished` olayından gelir (ilk gelen; ikisi eşittir).
  - Hata `errorText` ile `startError`'a yazılır ve `phase = "ready"` olur.
  - Yedek `success` → `outputPath = ""`. Geri yükleme `success`/`completed_with_errors` → `refreshNode` (Kararlar 10) + `refreshInspector`.
- **İptal:** `cancelRequested = true` + `backupCancel(jobId)`. Hata sessizce yutulur; sonuç arka uçtan beklenir.
- **Kapatma:**
  - Çalışmıyorken `requestClose` hemen kapatır.
  - Çalışırken `closeAsk = true` olur. `answerClose(true)` iptal eder, `closing = true` yapar ve sonuç gelince `onClosed()` çağrılır. `answerClose(false)` diyaloğu açık bırakır.

- [ ] **Adım 1: Başarısız testler (`backup-controller.test.ts`; sahte api, sahte zamanlayıcı, `createRoot`).**
  - Açılış:
    - `connect` false → `loadError`; yeniden açma (yeniden dene) başarılı akışa döner.
    - `backupTools` hatası `toolsError`'a yazılır, diyalog açık kalır.
  - Plan:
    - 150 ms debounce: art arda üç değişiklik tek `backupPlan` çağrısı yapar.
    - Eski sürümün yanıtı yok sayılır.
    - İstek `{scope, format, options, outputPath}` biçimindedir ve `options` normalize edilmiştir.
    - `not_connected` → bir kez `connect` + yeniden plan; ikinci `not_connected` → `planError`.
  - Format ve yol:
    - Yol seçildikten sonra format değişince `swapExtension` uygulanır ve plan yenilenir.
    - `chooseOutput` `pick.save`'e `suggestFileName` + `saveFilters` verir; `null` (vazgeç) yolu değiştirmez.
  - Başlatma (yedek):
    - Olay sırası `started → line×3 → progress → finished` durumu doğru kurar.
    - `success` sonrası `outputPath === ""`.
    - `busy` hatası `startError`'a yazılır, `phase === "ready"`.
  - İptal ve yarışlar:
    - `cancel` → `backupCancel(jobId)`. Sonuç `cancelled` gelince "İptal edildi" görünür.
    - `finished: success` iptalden önce gelirse sonuç success kalır.
    - `close()` sonrası gelen olay ve dönüş durumu yazmaz.
  - Kapatma:
    - Çalışırken `requestClose` → `closeAsk`.
    - `answerClose(true)` → iptal + sonuç gelince `onClosed` bir kez çağrılır.
    - `answerClose(false)` → açık kalır.
  - Geri yükleme:
    - `chooseSource("file")` → `restoreInspect`. `invalid_source` → `sourceError` (yol iletide yok).
    - Plain kaynakta hedef `existing`'e zorlanır, seçenekler normalize edilir.
    - Plan değişince (`clean` açılınca) onay ve yazılan ad sıfırlanır.
    - Üretim + `data_loss`'ta ad yazılmadan `restoreStart` çağrılmaz. Doğru adla `confirmed: true, confirmedProduction: true` gider.
    - Kilit ağaçta açılınca `gate().reason === "locked"` ve `start` çağrı yapmaz.
    - Sunucu düğümünde `setTargetDb` → `ddlTargetOf` oid'i istekte kullanılır. Oluşturma hedefinde `databaseOid` null gider.
    - `success` → `refreshNode` (oluşturmada sunucu düğümü, mevcut hedefte veritabanı düğümü).
    - `completed_with_errors` → "Tamamlandı, N hata".
  - Araç ayarları:
    - `chooseBinDir` → `settingsSetPgBinDir(dir)` → `backupTools` → plan.
    - `invalid` hatası `binDirError`'a yazılır.
    - `clearBinDir` `null` gönderir.
  - `password_required` start hatası `passwordAdvice` içerir.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bunx vitest run src/features/backup/backup-controller.test.ts` → FAIL.
- [ ] **Adım 3: Uygula.** Gizlilik: `console.*` yok; bildirimlere (toast) yol, ad ve satır yazılmaz (yalnız sabit metinler).
- [ ] **Adım 4: Doğrula.** Odak testi PASS; dört doğrulama komutu.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/backup/backup-controller.ts src/features/backup/backup-controller.test.ts
  git commit -m "feat(backup): yedekleme ve geri yükleme denetleyicileri"
  ```

### Task 4: Diyaloglar

**Dosyalar:** Oluştur `src/features/backup/BackupDialog.tsx`, `RestoreDialog.tsx`, `ToolsPanel.tsx`, `JobView.tsx`, `picker.ts` (+ `picker.test.ts`); Task 1 iskeleti `BackupDialogs.tsx`'i gerçeğiyle değiştir.

**Arayüzler:**
- Tüketir: Task 3 denetleyicileri, Task 2 `backupOptionStates`/`restoreOptionStates`/`progressView`/`resultView`/`toolAdvice`/`chosenTool`/`elapsedLabel`, Task 1 `BackupDialogs` imzası ve `BackupDialogDeps`.
- Üretir: `BackupDialogs` (dışa açık tek bileşen) ve `picker.ts`: `export function makePicker(dialog: { save(o: unknown): Promise<string | null>; open(o: unknown): Promise<string | string[] | null> }): Picker` (eklentiyi içe aktarmaz; `BackupDialogs.tsx` `@tauri-apps/plugin-dialog`'un `save`/`open`'ını verir).

- `BackupDialogs.tsx`: `job.mode`'a göre `<BackupDialog>` ya da `<RestoreDialog>` çizer. Denetleyici deps'ini kurar:
  - `pick` → `makePicker({ save, open })` (`@tauri-apps/plugin-dialog`);
  - `servers` → `deps.query.servers`;
  - `refreshNode` → `deps.explorer.refresh` (hatası yutulur);
  - `newId` → `crypto.randomUUID()`.
- **`ToolsPanel.tsx`** (props: araç durumu, `pgBinDir`, hatalar, işleyiciler; metin `tb.tools`):
  - Özet satırı (`chosenTool` + `toolAdvice`) ve `problem` gösterilir.
  - Açılır içerikte küme tablosu, bin klasörü, "Klasör seç…", "Temizle" ve "Yeniden tara" bulunur.
  - Varsayılan açıklık Kararlar 3'teki gibidir.
- **`JobView.tsx`** (ortak):
  - Önizleme salt okunur `<pre>`'dir (kaydırmalı, `whitespace-pre-wrap break-all`). Çalışırken `started.preview`, aksi hâlde `plan.preview` gösterilir.
  - `plan.notes` listesi ve `general` hataları gösterilir.
  - Çalışırken:
    - İlerleme çubuğu (`progressView`; belirsizse belirsiz çubuk) ve bayt.
    - Geçen süre: 1 sn'lik `setInterval`, `onCleanup` ile temizlenir.
    - Satır günlüğü: en çok ~16 satır yükseklikte. Uyarı amber, hata kırmızı. Kullanıcı yukarı kaydırmadıysa sona yapışır.
    - İptal düğmesi; `cancelRequested` iken "İptal ediliyor…" olarak devre dışıdır.
  - Sonuç bölümü `resultView` tonuyla çizilir. Yedek başarısında "Klasörü göster" vardır (`reveal`; hata bildirimi sabit metindir).
- **`BackupDialog.tsx`** (genişlik 640px):
  - Kapsam başlığı ("Veritabanı: X", "Şema: X.Y", "Tablo: X.Y.Z"), ardından ToolsPanel.
  - Format radyo grubu.
  - Hedef: salt okunur yol + "Seç…"; alan hatası altında.
  - Seçenekler grupları: Genel / Düz SQL / Sıkıştırma ve paralel / Kodlama. Kodlama seçicisi "Sunucu varsayılanı" + `encodings` içerir. Sıkıştırma "Varsayılan" + 0–9; iş sayısı 1–16, boş = null.
  - JobView.
  - Alt bar: "Kapat" ve "Yedekle" (kapı kapalıyken `tb.blocked[reason]` `title`/`aria-describedby` ile bağlanır).
  - Çalışırken seçenekler `fieldset disabled` olur.
- **`RestoreDialog.tsx`:**
  - Kaynak: yol + "Dosya seç…"/"Klasör seç…". İnceleme sürerken gösterge, hata alan altında.
  - Kaynak özeti: format; arşivde ad, `pg_dump` sürümü, kaynak sunucu sürümü, öğe sayısı, ilk 6 tür sayımı + "…".
  - Hedef radyosu (Kararlar 6) ve seçenekler.
  - Risk rozeti: metin "Değişiklik" / "Veri kaybı".
  - Onay: kutu "Bu geri yüklemenin veritabanını değiştireceğini anlıyorum."; üretim + veri kaybında "Onaylamak için sunucu adını yazın: ‹ad›" girişi.
  - Düz SQL notları plan notlarından gelir. Alt bar: "Kapat" ve "Geri yükle" (veri kaybında tehlike varyantı).
- **Kapatma onayı:** çalışırken diyalog içi küçük onay şeridi: "İş sürüyor; iptal edilsin mi?" [İptal et ve kapat] [Devam et]. `onOpenChange(false)` → `requestClose`.
- **Klavye:** Enter → başlat (kapı ve onay tamamsa; `e.isComposing` değilse; odak `textarea`/`select`'te değilse). Esc → `requestClose`.

- [ ] **Adım 1: Başarısız test (`picker.test.ts`, sahte `save`/`open`).** `save` bağımsız değişkenleri olduğu gibi geçer; `openFile` → `open({ multiple: false, directory: false, filters: [{ name: "PostgreSQL yedeği", extensions: ["backup", "dump", "sql", "tar", "gz"] }, { name: "Tüm dosyalar", extensions: ["*"] }] })`; `openDir` → `open({ multiple: false, directory: true })`; dizi dönüşü ilk öğeye, `null` `null`'a indirgenir; eklenti hatası `null` değil fırlatma olarak geçer (denetleyici iletir).
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bunx vitest run src/features/backup/picker.test.ts` → FAIL (`makePicker` yok).
- [ ] **Adım 3: Uygula.** Görünümler yalnız çizer; mantık denetleyicide kalır.
- [ ] **Adım 4: Doğrula.**
  - Dört doğrulama komutu.
  - `bun run build` çıktısında yeni tembel parça adı (`BackupDialogs-*.js`) ve boyutu rapora yazılır.
  - `rg "@kobalte|plugin-dialog" src/app src/features/explorer src/ipc` → sonuç yok.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/backup/
  git commit -m "feat(backup): \"Yedekle…\" ve \"Geri yükle…\" diyalogları"
  ```

### Task 5: Bütçe telafisi, smoke maddeleri ve belgeler

**Dosyalar:** `src/app/strings.ts` ve telafi için seçilen tembel tüketiciler (aşağıda), `scripts/bundle-budget.json`, `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-18-backup-restore-design.md`, `docs/testing.md`.

- [ ] **Adım 1: Ölç.** `bun run bundle:check`. Ana paket ve açılış kümesi baytları, tavanlarla farkları (Task 1–4 artışı) kaydedilir.
- [ ] **Adım 2: Telafi (yalnız tembel parçaların kullandığı metinleri tembel dosyalara taşıyarak).** Aday sırası:
  1. `t.query.close` ve `t.query.bulk` içinde yalnız `src/features/query/tx-dialogs.tsx`'in kullandığı anahtarlar:
     - `close`: `title`, `message`, `failedMessage`, `busyNote`, `commit`, `rollback`;
     - `bulk`: `title`, `commitAll`, `rollbackAll`, `failedNote`, `tx`, `busy`.

     Bunlar yeni `src/features/query/strings-tx.ts`'e taşınır (yalnız `tx-dialogs.tsx` içe aktarır). `query-controller.ts`'in kullandıkları (`close.failed`, `close.closing`, `bulk.serverMessage`, `bulk.exitMessage`) `strings.ts`'te kalır.
  2. `t.query.hint.autocommit`, `rollback`, `sessionReset`: yalnız `src/features/query/errors.ts` kullanır. Yalnız `errors.ts` ana pakette değilse taşınır (`bundle:check` kaynak tablosunda `errors.ts` görünmüyorsa). `hint.locked` ana pakette kalır (`actions.ts`).
  3. Hâlâ fazla varsa: `rg "t\.<blok>\." src` ile her `strings.ts` bloğunun tüketicileri listelenir; tüketicilerinin hepsi tembel parçada olan blok taşınır. Tembellik `bundle:check` kaynak tablosuyla kanıtlanır.

  Taşınan her anahtar için `rg` kanıtı rapora yazılır. Metinler birebir korunur. İlgili testler (ör. `tx-dialogs` metinlerini kullanan testler varsa) yeni modülden okur.
- [ ] **Adım 3: Tavanı indir.**
  - Hedef: ana paket ≤ 128,466 B ve açılış kümesi ≤ 156,139 B.
  - `scripts/bundle-budget.json`'da `mainMaxBytes` ve `startupMaxBytes` yeni ölçülen değerlere indirilir (sıfır pay).
  - Telafi tam yetmezse tavan yükseltilmez: görev BLOCKED raporlanır (bayt farkı ve denenen taşımalar yazılır); karar kullanıcıya aittir (kullanıcı kararı 2026-10-31).
  - `bun run bundle:check` PASS.
- [ ] **Adım 4: Smoke maddeleri.** `docs/smoke-test.md`'de "## Yedekleme arka ucu (Plan 26)" bölümünden sonra **"## Yedekleme arayüzü (Plan 27)"** eklenir.
  - Ön koşul paragrafı:
    - Docker PG 10, 16 ve 17/18 sunucuları; kurulu PostgreSQL 18 istemci araçları (ve mümkünse 16).
    - Büyükçe tablo (ör. 5 milyon satır); Türkçe adlı şema/tablo; bölümlenmiş tablo.
    - "Üretim" etiketli ve kilitli sunucu kaydı.
    - Diyalog tembel parçadadır (`BackupDialogs-*.js`).
  - `[—]` maddeleri:
    - menü görünürlüğü (beş tür; bölüm, sistem şeması ve bağlı olmayan sunucu yok);
    - kilitli sunucuda "Geri yükle…" gerekçeyle kapalı, "Yedekle…" açık;
    - dört formatta yedek ve kayıt diyaloğu önerisi/uzantı;
    - format değişince uzantı;
    - mevcut dosyaya yazarken Windows üzerine yazma sorusu;
    - directory formatında dolu klasör hatası;
    - plain seçenekleri (`--clean`/`--if-exists`/`--inserts` dışlamaları) ve sıkıştırma `.sql.gz` notu;
    - kodlama listesi;
    - ilerleme çubuğu, bayt ve süre (büyük tablo);
    - yalnız şemada belirsiz çubuk;
    - iptal: geçici dosya kalmaz;
    - çalışırken Esc/× → onay;
    - "Klasörü göster" Gezgin'de dosyayı seçer;
    - araç ayarları: bin klasörü seç/temizle/yeniden tara;
    - geçersiz klasör iletisi; araç yokken `problem` metni ve açık bölüm;
    - PG 16 sunucu + 18 araç: bilgi/uyarı satırları;
    - geri yükleme custom/directory/tar/plain kaynakları ve özet;
    - `.sql.gz` kaynak iletisi;
    - mevcut veritabanına ve "arşivdeki adla oluştur" (sunucu düğümünden, veritabanı seçicisi dahil);
    - `--clean` ile veri kaybı rozeti ve üretimde ad yazma;
    - `-j` ⟂ `-1` ve tar'da `-j` devre dışı;
    - PG 16 sunucuya 18 `pg_restore` ile "Tamamlandı, N hata";
    - iptal edilen geri yüklemede "yarım kalmış olabilir" uyarısı;
    - başarı sonrası Explorer'da yeni veritabanı;
    - düz SQL meta komut notu;
    - ANSI dışı yol (`C:\Temp\a＂ -h x ＂.dump`) alan altında hata;
    - Türkçe yol (1254) çalışır;
    - parola ve URL hiçbir yerde görünmez (önizlemede `PGPASSWORD=***`);
    - klavye (Enter/Esc, IME) ve ekran okuyucu (ilerleme ve sonuç duyurusu).
- [ ] **Adım 5: Spec ve test belgesi.**
  - Spec `Durum`: "Plan 26 arka ucu tamam; Plan 27 ön yüzü uygulandı (feat/backup-ui); elle kontrol bekliyor (`docs/smoke-test.md` "Yedekleme arayüzü (Plan 27)")".
  - Spec'e yeni **"## 6. Plan 27 uygulama notları"** eklenir: Kararlar 1–11'in özeti (özellikle araç ayarlarının diyalog içinde olması, salt okunur yol, `backup_reveal`, tek iş), paket ölçümleri (ana / açılış / tembel parça) ve telafi kaydı.
  - `docs/testing.md`'ye "## Yedekleme ön yüzü testleri (Plan 27)" eklenir: dosya → kapsam tablosu (`backup-model.test.ts`, `backup-controller.test.ts`, `actions.test.ts`, Rust `reveal_arg`), paket tavanları ve elle kontrol bağlantısı.
- [ ] **Adım 6: Kapanış taramaları ve doğrulama.**
  - `rg "strings-backup" src` → yalnız `src/features/backup/*`.
  - `rg "console\." src/features/backup` → yok.
  - `rg "@kobalte|plugin-dialog" src/app src/features/explorer src/ipc` → yok.
  - `git status` → `graphify-out/` ve binding satır sonu gürültüsü sahnelenmemiş.
  - Dört doğrulama komutu PASS.
- [ ] **Adım 7: Commit.**
  ```bash
  git add src/app/strings.ts src/features/query/strings-tx.ts src/features/query/tx-dialogs.tsx scripts/bundle-budget.json docs/smoke-test.md docs/superpowers/specs/2026-10-18-backup-restore-design.md docs/testing.md
  git commit -m "docs(backup): Plan 27 paket telafisi, smoke maddeleri ve notlar"
  ```
  (Adım 2'de başka dosya taşındıysa `git add` listesine o dosyalar da eklenir.)

## Öz inceleme

- **Spec kapsamı:**
  - §3 menü → T1; tembel diyaloglar (araç durumu, seçenekler, önizleme, risk/onay, çalıştırma, sonuç, "Klasörü göster") → T2–T4; ayarlar → T3/T4 ToolsPanel (Kararlar 3).
  - §5 komutlar → T1; istek türleri → T2/T3.
  - §5 olaylar, sınırlar ve `completed_with_errors` → T2 `applyEvent`/`resultView`.
  - §5 hata kodları → T3 (`not_connected`, `password_required`, `busy`, `invalid_source`, `locked`).
  - §5 alan yolları → T2 `splitFieldErrors`; ipuçları → T2 `resultView`.
  - §5 bilinen sınırlar: (a) → T2 `toolAdvice`; (b) → Review Focus 5; (c) → Kararlar 8; (d) → `progressView` belirsiz; (e) → kapsam dışı (arka uç takip işi); (f) → Kararlar 2.
- **Tip tutarlılığı:**
  - `BackupDialogs` imzası T1 = T4.
  - `Gate`/`canStart`/`confirmFlags` T2 → T3.
  - `Picker` T3 → T4 `makePicker` (`picker.ts`).
  - `loadBackupDialogs` T1 → prefetch.
  - `BackupKey`/`RestoreKey` T2 → T3 `setOption`.
- **Orantı:** kod gövdesi yok; yalnız imzalar, sabit metinler ve test beklentileri.
