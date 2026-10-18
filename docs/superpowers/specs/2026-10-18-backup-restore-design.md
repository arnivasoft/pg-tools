# pg-tools — Yedekleme / Geri Yükleme Tasarımı (pg_dump / pg_restore / psql)

- **Tarih:** 2026-10-18
- **Durum:** Plan 26 arka ucu tamam (branch feat/backup-backend; sözleşme §5); Plan 27 ön yüzü kalan
- **Kapsam:**
  - Explorer'dan veritabanı, şema ya da tablo yedeği almak (`pg_dump`).
  - Yedek dosyasını bir veritabanına geri yüklemek: custom/directory/tar için `pg_restore`, düz SQL için `psql`.
  - Araçlar otomatik bulunur; ayarlardan elle bin klasörü de verilebilir.
  - Canlı ilerleme, iletiler ve iptal.
- **Kullanıcı kararları (2026-10-18):** araçlar otomatik bulunur ve elle yol verilebilir (paketlenmez); ilk sürüm yedekleme + geri yüklemedir (globals/`pg_dumpall` yok).
- **Teslim:** Plan 26 (arka uç: yeni `pgt-backup` crate'i, araç bulma, iş yürütücü, Tauri komutları, Docker testleri), Plan 27 (ön yüz: "Yedekle…" ve "Geri yükle…" diyalogları, ayar).

---

## 1. Kararlar

### Araç bulma ve sürüm uyumu

- **Aday klasörler** (sırayla, tekrarlar elenir):
  1. Ayardaki `pgBinDir` (verildiyse).
  2. `PATH`.
  3. `%ProgramFiles%\PostgreSQL\*\bin` ve `%ProgramFiles(x86)%\PostgreSQL\*\bin`.
  4. `%ProgramFiles%\pgAdmin 4\runtime` (ya da `…\pgAdmin 4\v*\runtime`).
- Her adayda `pg_dump.exe`, `pg_restore.exe` ve `psql.exe` için `--version` çalıştırılır (zaman aşımı 5 sn) ve çıktıdan sürüm okunur. Sonuç bir `ToolSet { dir, version_num, has: {pg_dump, pg_restore, psql} }` listesidir.
- **Seçim:**
  - `pg_dump` sürümü sunucunun ana sürümüne **eşit ya da büyük** olmalıdır; aksi hâlde araç kullanılmaz. Uygun olanlardan sunucuyla aynı ana sürüm tercih edilir, yoksa en küçük büyük sürüm.
  - `pg_restore` arşivi okuyabilmelidir: arşivi üretenle aynı ya da daha yeni sürüm gerekir. Arşiv sürümü `pg_restore -l` özetinden okunur. Sıra: sunucuyla aynı ana sürüm (arşivi okuyabiliyorsa); yoksa arşivi okuyabilen en küçük ana sürüm; yoksa en yeni (uyumsuzluğu PG kendi hatasıyla bildirir). Eşitlikte aday sırası (ayardaki klasör önce). Arşiv sürümü bilinmiyorsa sunucuyla aynı ana sürüm, yoksa en yeni.
  - `psql` için sürüm şartı yoktur, en yeni seçilir (düz SQL'in uyumu dosyayı üreten `pg_dump`'a bağlıdır).
  - Uygun araç yoksa iletide sunucu sürümü ve bulunan en yeni araç sürümü belirtilir, "Ayarlardan bin klasörü seçin" önerisi verilir.
- **Ayar:** `AppSettings.pgBinDir: Option<String>`. Kaydedilmeden önce doğrulanır: klasör var mı, içinde en az `pg_dump.exe` var mı.

### Yedekleme

- **Kapsam:**
  - Veritabanı düğümü: o veritabanının tamamı.
  - Şema düğümü: `-n <şema>`.
  - Tablo, MV ve sequence düğümü: `-t <şema.ad>`.
  - Bağlantı Explorer'daki sunucu kaydının bağlantı bilgileriyle yapılır.
- **Format:** `custom` (`-Fc`, varsayılan), `plain` (`-Fp`, .sql), `directory` (`-Fd`), `tar` (`-Ft`).
- **Seçenekler:**
  - Genel: yalnız şema (`-s`), yalnız veri (`-a`) (bu ikisi birbirini dışlar), sahiplik hariç (`-O`), yetkiler hariç (`-x`).
  - Yalnız plain: `--clean` (`-c`), `--if-exists` (`--clean` gerektirir; diğer formatlarda geri yüklemede verilir), `--create` (`-C`; plain'de veritabanını oluşturur, diğer formatlarda arşive yazılır ve geri yüklemede kullanılır), `--inserts` / `--column-inserts` (birbirini dışlar).
  - Sıkıştırma `-Z 0–9`: custom ve directory'de geçerli, plain'de isteğe bağlı (gzip ile .sql.gz), tar'da yok.
  - Paralel iş `-j 1–16`: yalnız directory.
  - Kodlama `-E`: isteğe bağlı, sunucu kodlama listesinden.
  - `--verbose` her zaman açıktır; ilerleme için gerekir.
- **Hedef yol:** kayıt diyaloğuyla seçilir. Dosya formatlarında uzantı önerilir (.backup/.dump, .sql, .tar). Directory formatında hedef boş ya da var olmayan bir klasör olmalıdır.
- **Atomik yazım:** dosya formatlarında çıktı önce `<hedef>.pgt-partial` olarak yazılır, başarıda hedefe taşınır (`rename`, varsa üzerine). Başarısızlık ya da iptalde geçici dosya silinir. Directory formatında aynısı geçici klasörle yapılır. Hedef dosya zaten varsa üzerine yazma onayı ön yüzde istenir (kayıt diyaloğu da sorar).
- **Risk:** yedekleme salt okunurdur; kilitli sunucuda da serbesttir. Veritabanı yazılmaz.

### Geri yükleme

- **Kaynak:** dosya ya da klasör seçilir. Format otomatik algılanır:
  - Dosyanın ilk baytları `PGDMP` ise custom.
  - Klasörde `toc.dat` varsa directory.
  - Tar başlığında `toc.dat` varsa tar.
  - Diğer metin dosyaları plain SQL.
  - Custom/directory/tar için `pg_restore -l` ile arşiv özeti okunur: arşiv sürümü, kaynak veritabanı adı, öğe sayısı ve öğe türlerinin sayımı. Bu ilerlemenin toplamı olarak da kullanılır.
- **Hedef:**
  - **Mevcut veritabanı:** düğüm veritabanı ya da hedef seçici.
  - **Arşivdeki adla oluştur** (`--create`): bakım veritabanına bağlanılır. Sunucu kapsamlıdır.
- **Seçenekler (`pg_restore`):** temizle (`-c`, `--if-exists` ile), oluştur (`-C`), tek transaction (`-1`), paralel iş (`-j`), yalnız şema/veri (birbirini dışlar), sahiplik hariç (`-O`), yetkiler hariç (`-x`), hatada dur (`-e`).
  - `-j` ile `-1` birlikte kullanılamaz.
  - `-C` ile hedef bakım veritabanıdır.
  - `--verbose` her zaman açıktır.
- **Düz SQL (`psql`):** `psql -X -v ON_ERROR_STOP=1 -f <dosya>`, isteğe bağlı `-1` (tek transaction). Yalnız "mevcut veritabanına" yüklenir; temizle ve oluştur seçenekleri yoktur (SQL içeriğine bağlıdır). İçerik taranmaz. Düz SQL dosyası psql meta komutları (`\!`, `\connect`, `\copy … program`) çalıştırabilir; plan notu yalnız güvenilen dosyaların yüklenmesini söyler.
- **Risk:**
  - Geri yükleme veritabanını değiştirir.
  - Kilitli sunucuda reddedilir.
  - `--clean` ya da `--create` **DataLoss** sayılır: mevcut nesneler silinir. Onay kutusu gerekir; üretim sunucusunda sunucu adı yazılarak onay alınır (mevcut kapı kuralı).
  - Diğer geri yüklemeler **Destructive** sayılır ve onay ister.
  - Plain SQL içerik bilinmediği için **DataLoss** sayılır.

### Ortak kurallar

- **Komut önizlemesi:** diyalog, çalıştırılacak komut satırını gösterir. Parola gösterilmez (`PGPASSWORD` ortamda, önizlemede `***`). Argümanlar dizi olarak verilir; kabuk kullanılmaz, enjeksiyon olmaz.
- **Kimlik bilgisi:**
  - Parola, sunucunun oturum ya da kalıcı parolasından alınır (`Credentials::password`). İş başlamadan ön yüz sunucuya bağlanır (gerekirse parola istemi açılır).
  - Parola yalnız çocuk sürecin ortamına `PGPASSWORD` olarak verilir. Günlüğe, hata iletisine ya da diske yazılmaz.
  - Diğer bağlantı değerleri ortam değişkenleriyle verilir:
    - `PGHOST`, `PGPORT`, `PGUSER`, `PGDATABASE`.
    - `PGSSLMODE`: `disable`, `prefer`, `require`, `verify-ca` ya da `verify-full`.
    - `PGSSLROOTCERT`, `PGSSLCERT`, `PGSSLKEY`.
    - `PGAPPNAME=pg-tools backup|restore`, `PGCONNECT_TIMEOUT=15`.
  - Süreçten miras gelen tüm `PG*` ortam değişkenleri temizlenir.
- **İlerleme:** iş olayları bir Tauri `Channel<BackupEvent>` ile akar:
  - `Started { command }` (parolasız önizleme metni).
  - `Line { severity: info|warning|error, text }`: stderr satırları. En çok 2000 satır tutulur; satır başına 4 KiB, sınırda kesilir.
  - `Progress { done, total: Option, bytes }`:
    - Yedeklemede `done`, "dumping contents of table" satırlarının sayısıdır. `total`, işten önce katalogdan sayılan tablo sayısıdır (kapsama göre). `bytes`, hedef/geçici dosyanın boyutudur ve 1 sn'de bir okunur.
    - Geri yüklemede `done` öğe başına bir satırdır: `creating …`, `processing data for table …` (eski araçlarda `restoring data …`), `executing …`, `processing BLOBS`. `processing item`/`launching item`/`finished item` sayılmaz (paralel kipte aynı öğe için ek satırlardır). `total`, arşiv özetindeki öğe sayısıdır; yalnız şema, yalnız veri ya da oluştur (`--create`) seçiliyse `done` toplama ulaşmayacağından `total` yoktur (belirsiz ilerleme). Plain'de `total` yoktur. Ön yüz `done`'u `total` ile sınırlar.
  - `Finished { outcome: success|completed_with_errors|failed|cancelled, exit_code, elapsed_ms, warnings, errors, output_path? }`.
- **Hata eşleme:** çıkış kodu sıfır değilse sonuç failed olur. Son `error:` satırları özetlenir.
  - "password authentication failed" ve "no password supplied" için ipucu: parolayı kaydedin ya da yeniden bağlanın.
  - "server version mismatch" / "aborting because of server version mismatch" için ipucu: daha yeni pg_dump gerekir, bin klasörü seçin.
  - `pg_restore` "unsupported version" için ipucu: daha yeni pg_restore gerekir.
  - `pg_restore` `-e` olmadan hataları atlar, sonunda `errors ignored on restore: N` yazar ve **1** ile çıkar. Bu durumda (geri yükleme, `pg_restore`, "hatada dur" kapalı, çıkış kodu 1 ve sayı satırı görüldü) sonuç `completed_with_errors` olur: "tamamlandı, N hata" olarak vurgulanır, veri yüklenmiş olabilir. Sayı satırı yoksa (bağlantı ya da arşiv okuma hatası gibi erken çıkış) sonuç `failed`'dır.
- **İptal:**
  - `backup_cancel(job_id)` çocuk süreci sonlandırır (Windows'ta `TerminateProcess`; `-j` iş parçacıklı olduğu için tek süreç yeter). Geçici çıktı silinir.
  - Uygulama kapanırken ya da sayfa yenilenirken tüm işler sonlandırılır (`cancel_all`, mevcut `on_page_load` ve `RunEvent::Exit` kancaları).
  - Yarıda kalan geri yükleme veritabanını yarım bırakabilir (`-1` yoksa). Bu, diyalogda ve sonuçta uyarılır.
- **Günlük:** yalnız iş türü, süre, çıkış kodu ve sonuç yazılır. Argümanlar, yollar, adlar, parola ve stderr metni yazılmaz.

### Kapsam dışı

- `pg_dumpall` / globals.
- Zamanlanmış yedekleme.
- Uzak (SSH) çalıştırma.
- Seçili nesne listesiyle geri yükleme (TOC düzenleme).
- `--exclude-table` / `--exclude-schema` listeleri.
- Sıkıştırma algoritması seçimi (PG 16+ `lz4`/`zstd`).
- Linux/macOS araç yolları (yapı taşınabilir tutulur, ama keşif yalnız Windows'a göre test edilir).

## 2. Model (özet)

- `ToolSet { dir, versionNum, version, pgDump, pgRestore, psql: bool }`; `ToolDetection { sets: ToolSet[], chosenDump?, chosenRestore?, chosenPsql?, problem? }`.
- `BackupRequest { scope: Database | Schema{name} | Table{schema,name}, format, options, outputPath, encoding? }`.
- `RestoreSource { path, format (algılanan), archive?: ArchiveInfo }`, `ArchiveInfo { archiveVersion, dbName, items, counts }`.
- `RestoreRequest { source, target: Existing{databaseOid} | CreateFromArchive, options }`.
- `JobPlan { command: string (parolasız), risk, notes }`; uygulama `backup_start` / `restore_start` (+ `job_id`, `Channel<BackupEvent>`, onaylar) → `JobSummary`.

## 3. Arayüz (Plan 27 özeti)

- Explorer: veritabanı, şema, tablo, MV ve sequence düğümlerinde **"Yedekle…"**; veritabanı ve sunucu düğümlerinde **"Geri yükle…"** (kilitli sunucuda devre dışı).
- Tembel diyaloglar: araç durumu (bulunan sürüm, uyarı), seçenekler, komut önizlemesi, risk/onay, çalıştırma (ilerleme çubuğu + bayt + satır günlüğü + iptal), sonuç (süre, uyarı/hata sayıları, "Klasörü göster").
- Ayarlar: "PostgreSQL araç klasörü" seçimi, bulunan araç listesi, "yeniden tara".

## 4. Test stratejisi

- Saf birimler: argüman/ortam üretimi (her format ve seçenek, kural dışlamaları), sürüm ayrıştırma (`pg_dump (PostgreSQL) 16.4`, `18devel`, `17beta1`, Windows yolları), araç seçimi tablosu, format algılama (örnek baytlar), satır sınıflandırma ve ilerleme sayımı, hata ipuçları.
- Docker PG 10–18:
  - Gerçek `pg_dump`/`pg_restore`/`psql` gerekir. Test ortamında en yeni istemci araçları `PGT_TEST_PGBIN` ile verilir; yoksa testler atlanır.
  - Senaryolar:
    - Her formatta yedekle → yeni veritabanına geri yükle → tablo ve satır sayıları eşit.
    - Şema ve tablo kapsamı.
    - İptal: büyük tablo, iptal sonrası geçici dosya yok.
    - Yanlış parola ve sürüm uyumsuzluğu ipuçları.
    - `--clean` ile geri yükleme.
    - Plain + `psql -1` hata durumunda geri alma.
    - Günlükte parola, yol ya da ad yok.

## 5. Plan 26 sözleşme notları (Plan 27 için)

Kaynak: `crates/pgt-backup/src/*` ve `src-tauri/src/backup.rs`. Tür tanımları `bindings/` altına ts-rs ile dışa aktarılır (alanlar camelCase; enum değerleri snake_case). §1–§2'deki taslak adlarla fark varsa **bu bölüm geçerlidir** (ör. `Started.preview`, `ToolSet.version`).

### Komutlar (Tauri)

| Komut | Bağımsız değişkenler (camelCase) | Dönüş |
|---|---|---|
| `backup_tools` | `serverId` | `ToolDetection` (her çağrı yeniden tarar; sunucu bağlıysa `chosenDump` sunucu sürümüne göre) |
| `backup_plan` | `serverId`, `databaseOid: number`, `request: BackupRequest` | `JobPlan` (alan hataları `errors` içinde; komut hatası vermez) |
| `backup_start` | `serverId`, `databaseOid`, `request`, `jobId: Uuid`, `onEvent: Channel<BackupEvent>` | `JobSummary` |
| `backup_cancel` | `jobId` | `void` (bilinmeyen kimlik sessizce yok sayılır) |
| `restore_inspect` | `path: string` | `RestoreSource` (sunucu gerekmez) |
| `restore_plan` | `serverId`, `databaseOid: number \| null`, `request: RestoreRequest` | `JobPlan` |
| `restore_start` | `serverId`, `databaseOid \| null`, `request`, `jobId`, `confirmed: bool`, `confirmedProduction: bool`, `onEvent` | `JobSummary` |
| `settings_set_pg_bin_dir` | `dir: string \| null` | `AppSettings` (boş/`null` ayarı temizler; klasör ve `pg_dump` var mı doğrulanır; hata `invalid`) |

`jobId` ön yüzde üretilir (`crypto.randomUUID()`); `backup_start`/`restore_start` iş bitince döner, ilerleme `onEvent` ile akar. İptal için ayrı çağrı gerekir (`backup_cancel`); iptal edilen işin `start` çağrısı hata değil `outcome: "cancelled"` ile döner.

### İstek türleri

- `BackupRequest { scope, format, options, outputPath }`.
  - `scope`: `{ kind: "database" }` | `{ kind: "schema", name }` | `{ kind: "table", schema, name }` (tablo, MV ve sequence).
  - `format`: `"custom" | "plain" | "directory" | "tar"`.
  - `options` (`BackupOptions`): `schemaOnly`, `dataOnly`, `noOwner`, `noPrivileges`, `ifExists`, `clean`, `create`, `inserts`, `columnInserts` (bool); `compress: number|null` (0–9), `jobs: number|null` (1–16), `encoding: string|null`.
- `RestoreRequest { path, target, options }`.
  - `target`: `"existing"` (komutun `databaseOid` değeri) | `"create_from_archive"` (bakım veritabanına bağlanır, `databaseOid` yok sayılır).
  - `options` (`RestoreOptions`): `clean`, `ifExists`, `create`, `singleTransaction`, `schemaOnly`, `dataOnly`, `noOwner`, `noPrivileges`, `exitOnError` (bool); `jobs: number|null`.
  - Format istekte yoktur: `restore_inspect`/sunucu tarafı içerikten algılar; kullanıcı zorlayamaz.
- `RestoreSource { path, format: "custom"|"directory"|"tar"|"plain", archive: ArchiveInfo|null, isDir }`; `ArchiveInfo { archiveVersion, serverVersion, dbName (hepsi string|null), items: number, counts: [string, number][] }` (`counts` çoktan aza).
- `ToolDetection { sets: ToolSet[], problem: string|null, chosenDump, chosenRestore, chosenPsql: string|null }` (seçili klasör yolları); `ToolSet { dir, version: { major, versionNum, text }, pgDump, pgRestore, psql }`.
- `JobPlan { preview, risk: "safe"|"destructive"|"data_loss" (pgt_ddl::Risk biçimi), notes: string[], errors: FieldError[] }`. `errors` doluysa `preview` boştur, iş başlatılamaz. `restore_plan` hata durumunda `risk: destructive`, `backup_plan` `safe` döner.
- `FieldError { path, message }`.

### Olaylar ve özet

Sıra: `started` → `line`/`progress`* → `finished` (daima son). Etiket alanı `event`.

- `{ event: "started", preview }` (parolasız tek satır; `PGPASSWORD=***`).
- `{ event: "line", severity: "info"|"warning"|"error", text }`.
- `{ event: "progress", done, total: number|null, bytes: number|null }`. `bytes` yalnız yedeklemede ve ~1 sn aralıkla (klasörde dosya toplamı); geri yüklemede `null`.
- `{ event: "finished", outcome: "success"|"completed_with_errors"|"failed"|"cancelled", exitCode: number|null, elapsedMs, warnings, errors, outputPath: string|null, hint: string|null, truncatedLines }`.
- `JobSummary` aynı alanlardır (`start` komutlarının dönüş değeri `finished` ile eştir). `outputPath` yalnız başarıda dolu. `warnings`/`errors` gösterilmeyen satırları da sayar.
- Sınırlar: en çok **2000** satır (`MAX_LOG_LINES`), fazlası `truncatedLines`'a sayılır ve sona "… N satır daha gösterilmedi" satırı eklenir; satır başına **4096 bayt** (`MAX_LINE_BYTES`, UTF-8 sınırında kesilir, sonuna `…`); ilerleme aralığı 1 sn. Pencere kapanır/olay gönderilemezse iş iptal edilir.
- `completed_with_errors`: `pg_restore` "hatada dur" kapalıyken hataları atlayıp sonuna dek çalıştı ve `errors ignored on restore: N` yazıp 1 ile çıktı (`exitCode: 1`; `errors` hata satırı sayısıdır). Ön yüz "tamamlandı, N hata" gösterir; veri yüklenmiş olabilir. `hint` bu sonuçta da dolabilir (ör. ipucu 4). `success` her zaman sıfır çıkış kodudur.

### Ön koşullar, kapılar ve hata kodları

Hatalar `ErrorPayload { code, message, detail?, hint? }` olarak gelir.

| Kod | Ne zaman |
|---|---|
| `not_connected` | Sunucu bağlı değil (ön yüz önce bağlanmalı). |
| `password_required` | Kayıtlı/oturum parolası yok **ve** oturum parolayla açılmış: ön yüz parola isteyip yeniden bağlanır. Oturum parolasız açıldıysa (trust vb.) araç parolasız çalışır. |
| `locked` | `restore_start` ve sunucu salt okunur kilitli (her şeyden önce; araç bulma/bağlantı bile yapılmaz). Yedekleme kilitli sunucuda serbesttir. İleti: "Sunucu salt okunur kilitli; geri yükleme yapılamaz." |
| `invalid_request` | Plan alan hatası; `message` ilk hatanın iletisi, `detail` alan yolu. |
| `confirmation_required` | Risk ≥ destructive ve `confirmed = false`. İleti: "Geri yükleme veritabanını değiştirir ve onay gerektiriyor." |
| `production_confirmation_required` | Risk = data_loss, sunucu üretim ve `confirmedProduction = false`. İleti: "Üretim sunucusunda veri kaybı olan geri yükleme ek onay gerektiriyor." |
| `invalid_source` | `restore_inspect` kaynağı okuyamadı/uygunsuz (iletide yol yok). |
| `busy` | Aynı hedefe yazan yedekleme zaten çalışıyor (`register`). |
| `invalid` | Aynı `jobId` ikinci kez kayıtlandı. |
| `not_found` / `unknown_server` | Veritabanı oid'i yok / sunucu kaydı yok. |

`restore_start` sırası: kilitli → plan doğrulaması → onay → üretim onayı → parola → iş kaydı. `backup_start` yalnız plan doğrulamasını ve parola/kayıt kontrolünü yapar (onay yok; risk her zaman safe).

**Risk kuralları:** yedekleme `safe`. Geri yükleme: düz SQL `data_loss` (içerik taranmaz); `pg_restore` `clean` ya da `create/create_from_archive` ile `data_loss`, aksi `destructive`. `data_loss` + üretim sunucusu → sunucu adı yazdırılarak ek onay (ön yüz).

### Plan alan hataları (`FieldError.path` → ileti)

Yedekleme:

- `tool`: "Seçilen araç klasöründe pg_dump yok." ya da seçim iletisi ("Sunucu sürümü 16 için uygun pg_dump yok (bulunan en yeni pg_dump: 15.4). Ayarlardan PostgreSQL bin klasörü seçin."); Windows'ta araç yolu kod sayfasında birebir temsil edilemiyorsa ANSI iletisi (aşağıda).
- **ANSI (yalnız Windows; bkz. bilinen sınırlar (b)):** `outputPath`/`path`/`tool`/`ssl.rootCert`/`ssl.clientCert`/`ssl.clientKey`: "Bu yol Windows kod sayfasında güvenle temsil edilemiyor; araç onu farklı karakterlerle alır. Yalnız sistem kod sayfasındaki karakterleri içeren bir yol seçin."; `host` ve (savunma süpürmesi) `request`: "Bu değer Windows kod sayfasında güvenle temsil edilemiyor; araç onu farklı karakterlerle alır."
- `database` / `user` (yalnız Windows): "Windows'ta PostgreSQL araçları ASCII dışı karakter içeren veritabanı adına / kullanıcı adıyla bağlanamaz."
- `scope.name`, `scope.schema`: "Ad boş olamaz." / "Ad geçersiz karakter içeriyor."; `scope.name` ayrıca ASCII dışı ad belirsiz joker desene uyuyorsa: "Ad ASCII dışı karakter içeriyor: araç adı joker desenle aktarır ve desen başka nesnelere de uyuyor. Veritabanının tamamını yedekleyin ya da nesneyi yeniden adlandırın."; desen sayım sorgusu hata verirse kapalı kalınır: "Ad ASCII dışı karakter içeriyor ve joker desenin başka nesnelere uyup uymadığı denetlenemedi. Veritabanının tamamını yedekleyin ya da yeniden deneyin." Denetim plan ve başlatma anında yapılır; arada ya da iş sürerken desene uyan yeni nesne oluşturulursa (TOCTOU) o da yedeğe girebilir.
- `outputPath`: "Yol boş olamaz." / "Yol geçersiz karakter içeriyor." / "Dosya yolu klasör ayracıyla bitemez." / "Tam (mutlak) yol gerekir." / hedef denetimi: "Hedef klasör boş değil; boş bir klasör ya da yeni bir ad seçin." / "Hedef bir dosya; klasör formatı için boş bir klasör ya da yeni bir ad seçin." / "Hedef bir klasör; bir dosya adı seçin." / "Hedef denetlenemedi: …".
- `options.dataOnly`: "Yalnız şema ile yalnız veri birlikte seçilemez."
- `options.clean`: "Yalnız düz SQL (plain) formatında kullanılabilir." ya da "Temizle (--clean) yalnız veri ile birlikte kullanılamaz."
- `options.inserts`: plain dışı aynı ileti; `options.columnInserts`: plain dışı ya da "--inserts ile --column-inserts birlikte seçilemez."
- `options.ifExists`: "--if-exists yalnız düz SQL (plain) formatında temizle (--clean) ile kullanılabilir." / "--if-exists, temizle (--clean) seçeneğini gerektirir."
- `options.compress`: "Tar formatında sıkıştırma yok." / "Sıkıştırma düzeyi 0–9 olmalı."
- `options.jobs`: "Paralel iş yalnız directory formatında kullanılabilir." / "Paralel iş sayısı 1–16 olmalı."
- `options.encoding`: "Geçersiz kodlama adı."

Geri yükleme:

- `path`: ANSI iletisi (arşivde `pg_restore -l` bile çalıştırılmadan önce), arşiv okunamadı iletisi ("Arşiv okunamadı: …"), kaynak algılama iletileri ("Kaynak yolu geçersiz.", "Kaynak için tam (mutlak) yol gerekir.", "Kaynak bulunamadı.", "Klasörde toc.dat yok; directory formatında alınmış bir pg_dump yedeği seçin.", "Dosya gzip ile sıkıştırılmış (.sql.gz); psql bunu doğrudan okuyamaz. Önce açın (ör. 7-Zip) ve .sql dosyasını seçin.") ve yol iletileri (yedeklemedekiyle aynı).
- `target`: "Hedef veritabanı seçilmedi." (`existing`, `databaseOid` null) / "Düz SQL yalnız mevcut bir veritabanına yüklenir." / "Bakım veritabanı bilinmiyor."
- `tool`, `database`, `user`: yedeklemedekiyle aynı (`tool` için `pg_restore`/`psql`).
- Düz SQL'de `options.clean|ifExists|create|jobs|schemaOnly|dataOnly|noOwner|noPrivileges`: "Düz SQL geri yüklemede kullanılamaz (SQL içeriğine bağlıdır)."
- `options.create`: "Oluştur (--create) için hedef \"arşivdeki adla oluştur\" olmalı."
- `options.dataOnly` / `options.clean` / `options.ifExists`: yedeklemedekiyle aynı.
- `options.singleTransaction`: "Oluştur (--create) ile tek transaction birlikte kullanılamaz."
- `options.jobs`: "Paralel iş sayısı 1–16 olmalı." / "Paralel iş ile tek transaction birlikte kullanılamaz." / "Tar arşivinde paralel geri yükleme yapılamaz."

Plan notları (`JobPlan.notes`, ön yüz olduğu gibi gösterebilir): directory için "Hedef klasör boş olmalı ya da hiç olmamalı."; plain + `compress>0` için gzip (.sql.gz) notu; `create` + plain dışı için arşive yazılma notu; geri yüklemede bakım veritabanı, "Arşivdeki nesneler hedefte varsa önce silinir.", "İş yarıda kalırsa veritabanı yarım kalabilir (tek transaction kapalı).", "Hatalar atlanır ve sonunda sayıları bildirilir; …" ve düz SQL için "Düz SQL psql ile çalıştırılır; içerik taranmaz ve ilk hatada durulur." (psql `ON_ERROR_STOP=1`, `--no-psqlrc`) ile "Düz SQL dosyası psql meta komutları (\!, \connect, \copy … program) çalıştırabilir; yalnız güvendiğiniz dosyaları yükleyin." Plain yedekte araç ≥ 17 ve sunucu < 17 ise: "Bu yedek `SET transaction_timeout` içerir; PostgreSQL < 17 sunucuya düz SQL olarak geri yüklenemez. Aynı ana sürüm pg_dump kullanın ya da custom formatı seçin." (`PGREQUIREAUTH` kullanılmaz: trust/sertifika kimlik doğrulamasını bozabilir.)

### Araç seçimi

- `pg_dump`: sunucu ana sürümüne eşit ya da büyük; aynı ana sürüm tercih edilir, yoksa en küçük büyük.
- `pg_restore` (`tools::choose_restore(sets, serverVersionNum, archiveDumpVersion)`): sunucuyla aynı ana sürüm (arşivi okuyabiliyorsa, yani ≥ arşivi üreten pg_dump ana sürümü); yoksa arşivi okuyabilen en küçük ana sürüm; yoksa en yeni. Eşitlikte aday sırası. `restore_plan`/`restore_start` arşiv özetini bir kez okur (en yeni `pg_restore -l`) ve hem seçimde hem toplamda kullanır. `ToolDetection.chosenRestore` arşiv bilinmeden seçilir (sunucuyla aynı ana sürüm, yoksa en yeni).
- `psql`: en yeni (sürüm önemsiz).
- `ToolDetection.problem` metinleri: "PostgreSQL istemci araçları (pg_dump, pg_restore, psql) bulunamadı. Ayarlardan PostgreSQL bin klasörü seçin." | "Ayarlardaki bin klasöründe çalışan PostgreSQL aracı bulunamadı; diğer bulunan araçlar kullanılacak." | seçim iletisi (yukarıdaki `tool` örneği; sunucu bağlıyken `chosenDump` yoksa).
- `settings_set_pg_bin_dir` hataları: "Bin klasörü için tam (mutlak) yol gerekir." / "Seçilen bin klasörü bulunamadı." / "Seçilen klasörde pg_dump.exe yok."
- Her `backup_tools` çağrısı yeniden tarar; diğer komutlar önbelleği kullanır (ayar değişince geçersiz olur).

### İpuçları (`JobSummary.hint`; yalnız `failed` ve `completed_with_errors` işte)

1. Parola (`password authentication failed`, `no password supplied`, Türkçe `şifre doğrulaması başarısız`): "Parola reddedildi ya da verilmedi: parolayı kaydedin veya sunucuya yeniden bağlanın."
2. `server version mismatch`: "Sunucu bu pg_dump'tan yeni: daha yeni bir pg_dump gerekir; Ayarlardan PostgreSQL bin klasörü seçin."
3. `unsupported version`: "Arşiv bu pg_restore'dan yeni bir pg_dump ile alınmış: daha yeni bir pg_restore gerekir; Ayarlardan PostgreSQL bin klasörü seçin."
4. `unrecognized configuration parameter "transaction_timeout"`: `pg_restore` işinde araç sunucudan yeni; sunucuyla aynı ana sürümden araç kurun ("hatada dur" kapalıyken `pg_restore` satırı atlar). `psql` (düz SQL) işinde satır dosyanın kendisindedir: "Düz SQL yedeği bu sunucudan yeni bir pg_dump ile alınmış ve sunucunun tanımadığı bir ayar (transaction_timeout) içeriyor: yedeği sunucuyla aynı ana sürümden pg_dump ile yeniden alın ya da custom formatı kullanın."

Öncelik yukarıdaki sıradadır; satır sınırı aşıldığında son 100 satır taranır.

### Ortam ve süreç sözleşmesi

- Argümanlar dizi olarak verilir (kabuk yok); stdin/stdout `null`, yalnız stderr okunur (kayıplı UTF-8). Windows'ta konsol penceresi açılmaz (`CREATE_NO_WINDOW`).
- `env_clear` kullanılmaz (Windows `SystemRoot`/`PATH`). Miras alınan adı `PG` ile başlayan her değişken (harf duyarsız) ve `LC_ALL`, `LANGUAGE` kaldırılır; sonra `PGHOST`, `PGPORT`, `PGUSER`, `PGDATABASE`, `PGSSLMODE`, `PGSSLROOTCERT/CERT/KEY` (varsa), `PGAPPNAME` (`pg-tools backup|restore`), `PGCONNECT_TIMEOUT=15` ve `LC_MESSAGES=C` eklenir. **`PGPASSWORD` yalnız parola varsa çocuk ortamına girer**; önizlemede `PGPASSWORD=***`.
- `pg_restore -d` değeri `dbname='…'` biçimindedir (ad bağlantı dizesi sanılmasın).
- Kapsam desenleri `--schema`/`--table` için çift tırnaklı ad (içteki `"` ikilenir); ASCII dışı karakterler tırnak dışında `?` jokeri olur (bkz. bilinen sınırlar).
- Günlükte (`tracing`) yalnız iş türü, sonuç, çıkış kodu ve süre vardır.

### Atomik çıktı

- Araç `<hedef>.pgt-partial`'a yazar. Başlamadan önce (`check_output`) hedef denetlenir; önceki çöküşten kalan `.pgt-partial` silinir (ad bize aittir).
- Başarıda `rename` ile hedefe taşınır (dosyada varsa üzerine yazar, hedefin hiç olmadığı bir an yoktur; klasörde boş hedef önce silinir). Hata, iptal ya da alıcı gitmesinde geçici çıktı silinir (Windows'ta tutamaç gecikmesi için 10 deneme).
- **Son taşıma başarısızsa** kopyala+sil yapılmaz: geçici çıktı **korunur**, sonuç `failed` olur ve son `error` satırı yeri söyler ("…; çıktı geçici konumda bırakıldı: <yol> (aynı hedefe bir sonraki yedekleme bunu siler ve üzerine yazar)"). Araç başarıyla bitip çıktı oluşmadıysa: "Araç başarıyla bitti ama çıktı oluşmadı."
- Silinemeyen geçici çıktı için son satır: "Geçici çıktı silinemedi: <yol>".
- İş kaydı aynı hedefe (son yol ve geçici yol) ikinci yedeklemeyi `busy` ile reddeder.

### Bilinen sınırlar

- **(a) 17+ araçlar PG < 17 sunucuda:** `pg_dump` bu uyumsuzluktan etkilenmez (yedek alınır), ama 17+ `pg_dump`'ın **düz SQL** çıktısı `SET transaction_timeout = 0;` satırını içerir (hedef sunucu yedek alınırken bilinmez): böyle bir dosyanın 17'den eski sunucuya `psql` ile geri yüklenmesi ilk satırda durur (`ON_ERROR_STOP`) — psql sürümü önemsizdir, çözüm yedeği aynı ana sürüm `pg_dump` ile almak ya da custom format kullanmaktır (plan notu, ipucu 4'ün psql metni). Arşiv geri yüklemede 17+ `pg_restore` 17'den eski sunucuya `SET transaction_timeout` gönderir: "hatada dur" kapalıyken satırı atlar, veriyi yükler ve 1 ile çıkar (`completed_with_errors`); açıkken `failed`. `choose_restore` sunucuyla aynı ana sürümden `pg_restore` varsa onu seçerek bunu önler. Plan 27 diyalogta **sunucuyla aynı ana sürümden araç kurmayı önermeli ve araç ≥ 17, sunucu < 17 ise uyarmalıdır**; ipucu 4 sonuçta da gösterilir.
- **(b) Windows ANSI kod sayfası:** araçlar argv/ortamı ANSI kod sayfasında alır. ASCII dışı şema/tablo adı `?` joker deseniyle aktarılır; desen birden fazla nesneye uyarsa plan hatası (`scope.name`), tek nesneye uyarsa çalışır. ASCII dışı veritabanı ya da kullanıcı adı Windows'ta plan hatasıdır (`database`/`user`). **Best-fit eşlemesi:** araçlar ANSI `main` kullanır; CRT argv'yi `GetCommandLineA` ile en yakın eşlemeyle (ör. U+FF02 `＂` → `"`, U+FF0D `－` → `-`) yeniden kurar. Böyle karakterli bir yol tek argüman olarak verilse de araçta tırnağı kapatıp argüman ekleyebilir (`-h kötü.örnek` ile `PGPASSWORD` başka sunucuya gider; `--clean` ile kapı atlanır). Bu yüzden Windows'ta sabit ASCII olmayan her argv öğesi (yollar; desenler zaten ASCII'dir) ile `PGHOST` ve sertifika yolu ortam değerleri `CP_ACP`'de birebir gidiş-dönüş yapmalıdır (`WideCharToMultiByte(CP_ACP, WC_NO_BEST_FIT_CHARS)` varsayılan karakter kullanmamalı ve geri çözüm aynı olmalı; `pgt_backup::ansi`). Aksi plan hatasıdır (ANSI iletisi); arşivde `pg_restore -l` bile çalıştırılmaz. Kod sayfasında olmayan karakterli yol (ör. 1254'te Japonca) da reddedilir; Türkçe yol 1254'te çalışır. Sistem kod sayfası UTF-8 (65001) ise her yol geçer.
- **(c) `password_required` yalnız oturum parola kullandıysa** döner; parolasız (trust) oturumda araç da parolasız çalışır.
- **(d) İlerleme toplamı yalnız `relkind = 'r'`:** bölümlenmiş üst tablo, MV ve yapılandırma tablosu olmayan uzantı üyesi tablolar (`pg_depend.deptype = 'e'`, `pg_extension.extconfig` dışı) sayılmaz; yalnız şema yedeğinde `total` yoktur (`null`). Sayı 0 ya da sorgu hatalıysa `total = null` (belirsiz ilerleme). Şema/tablo kapsamında uzantı dökülmezse yapılandırma tablosu sayılıp dökülmeyebilir (toplam biraz fazla; ön yüz `done`'u sınırlar). Geri yüklemede toplam arşiv özetindeki öğe sayısıdır; yalnız şema/veri ya da oluştur seçiliyse ve düz SQL'de yoktur.
- **(e) Kilitte bekleyen `pg_dump` iptali:** süreç sonlandırılır ama sunucu oturumu kilit bırakılana dek kalır (takip işi: `pg_cancel_backend` ile `PGAPPNAME` eşlemesi).
- **(f) Geri yüklemeler birbirini engellemez;** yalnız aynı hedef yola yazan yedeklemeler (`busy`) engellenir. Aynı veritabanına eşzamanlı iki geri yükleme ön yüzce engellenmelidir.
- Ek: `restore_start` arşiv özetini plan hazırlığında bir kez okur (`pg_restore -l`, 60 sn / 64 MiB sınırı) ve araç seçimi ile ilerleme toplamında yeniden kullanır; özet okunamazsa plan hatasıdır (`path`).
