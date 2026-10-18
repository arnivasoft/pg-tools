# pg-tools — Yedekleme / Geri Yükleme Tasarımı (pg_dump / pg_restore / psql)

- **Tarih:** 2026-10-18
- **Durum:** Taslak (Plan 26 arka ucu yazıldı; Plan 27 ön yüzü kalan)
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
  - `pg_restore` arşivi okuyabilmelidir: arşivi üretenle aynı ya da daha yeni sürüm gerekir. Sürüm bilinmiyorsa en yeni araç seçilir; uyumsuzluğu PG kendi hatasıyla bildirir.
  - `psql` için sürüm şartı yoktur, en yeni seçilir.
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
  - Genel: yalnız şema (`-s`), yalnız veri (`-a`) (bu ikisi birbirini dışlar), sahiplik hariç (`-O`), yetkiler hariç (`-x`), `--if-exists`.
  - Yalnız plain: `--clean` (`-c`), `--create` (`-C`; plain'de veritabanını oluşturur, diğer formatlarda arşive yazılır ve geri yüklemede kullanılır), `--inserts` / `--column-inserts` (birbirini dışlar).
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
- **Düz SQL (`psql`):** `psql -X -v ON_ERROR_STOP=1 -f <dosya>`, isteğe bağlı `-1` (tek transaction). Yalnız "mevcut veritabanına" yüklenir; temizle ve oluştur seçenekleri yoktur (SQL içeriğine bağlıdır). İçerik taranmaz.
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
    - Geri yüklemede `done`, "processing item"/"creating"/"restoring data" satırlarının sayısıdır. `total`, arşiv özetindeki öğe sayısıdır. Plain'de `total` yoktur.
  - `Finished { outcome: success|failed|cancelled, exit_code, elapsed_ms, warnings, errors, output_path? }`.
- **Hata eşleme:** çıkış kodu sıfır değilse sonuç failed olur. Son `error:` satırları özetlenir.
  - "password authentication failed" ve "no password supplied" için ipucu: parolayı kaydedin ya da yeniden bağlanın.
  - "server version mismatch" / "aborting because of server version mismatch" için ipucu: daha yeni pg_dump gerekir, bin klasörü seçin.
  - `pg_restore` "unsupported version" için ipucu: daha yeni pg_restore gerekir.
  - Uyarılı ama sıfır çıkış kodlu geri yüklemede (`-e` olmadan hatalar atlanır; pg_restore hata sayısını bildirir) sonuç "tamamlandı, N hata" olur ve vurgulanır.
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
