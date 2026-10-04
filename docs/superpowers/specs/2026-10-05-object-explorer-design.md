# pg-tools — Alt Proje 1: Object Explorer Tasarımı

- **Tarih:** 2026-10-05
- **Durum:** İnceleme bekliyor
- **Kapsam:** Sunucu kayıt defteri + bağlantı yönetimi + salt okunur Object Explorer (ağaç, Properties, DDL)

---

## 1. Amaç ve bağlam

pg-tools, şirket içinde pgAdmin4'ün yerini alacak masaüstü PostgreSQL yönetim aracıdır. pgAdmin4'te yaşanan üç sorun projenin varlık nedenidir:

1. **Yavaş açılış** (Python sunucusu + tarayıcı/Electron katmanı).
2. **Yüksek bellek** kullanımı.
3. **Windows uyumsuzluğu:** güncel pgAdmin bazı Windows sürümlerine kurulamıyor; eski pgAdmin ise yeni PostgreSQL sürümlerini desteklemiyor.

pgAdmin4'ün tamamı tek bir spec'e sığmayacağından proje alt projelere bölünmüştür. Bu belge **ilk alt projeyi** tanımlar. Sonraki adaylar (ayrı spec'ler): Query Tool (CodeMirror 6), veri ızgarası (TanStack Virtual), nesne oluşturma/düzenleme diyalogları, Dashboard.

### 1.1 Kullanıcıdan gelen kararlar

| Konu | Karar |
|---|---|
| Kullanım | Şirket içi; ürün düzeyinde paketleme/i18n önceliği yok |
| Teknoloji | Rust, Tauri 2, SolidJS, Kobalte (`@kobalte/core`), Tailwind CSS 4, CodeMirror 6, TanStack Virtual; paket yöneticisi bun |
| İstemci işletim sistemi | Windows 10 / 11, Windows Server 2019 / 2022 |
| Kapsam dışı OS | Windows 7, Windows Server 2012 (Rust ≥ 1.78 ve Tauri ≥ 2.12 desteklemiyor). Bu makinelerdeki PostgreSQL sunucularına desteklenen bir istemciden ağ üzerinden bağlanılır. |
| PostgreSQL | 10 – 18, yalnızca vanilla PostgreSQL (türev yok). Şirket eski sürümleri 18'e taşıyor. |
| Object Explorer kapsamı | Salt okunur: göz atma, Yenile, filtre, Properties, DDL. Değişiklik yapan işlem yok. |
| Nesne türleri | Çekirdek set (bkz. §5.1) |
| Bağlantı özellikleri | Temel bağlantı + Credential Manager, SSL/TLS, pgAdmin `servers.json` içe aktarma |
| Arayüz dili | Türkçe menü/komutlar, PostgreSQL nesne terimleri İngilizce (Tables, Indexes, …) |

### 1.2 Varsayımlar

- Ölçek "orta": ~50 kayıtlı sunucu, şema başına birkaç bin tablo. Mimari bunun 10 katında bozulmamalı.
- Server 2019/2022 makinelerinde WebView2 kurulu olmayabilir ve internet erişimi olmayabilir.

### 1.3 Başarı ölçütleri

Referans makine: SSD'li ortalama iş dizüstü bilgisayarı, Windows 11, sunucular LAN üzerinde.

| Ölçüt | Hedef |
|---|---|
| Soğuk açılış → pencere kullanılabilir | < 1 sn |
| Boşta toplam bellek (uygulama + WebView2 süreçleri) | < 150 MB; Rust süreci < 30 MB |
| Kurulum paketi | < 15 MB (WebView2 gömülü Server varyantı ~150 MB) |
| 5.000 tablolu "Tables" düğümünü açma | < 300 ms |
| Ağaç kaydırma | 60 fps, düğüm sayısından bağımsız |
| 10.000 yüklü düğümde filtreleme | < 50 ms |
| Seçili tablonun DDL'i | < 200 ms |

### 1.4 Kapsam dışı (bu alt proje)

- Nesne oluşturma, düzenleme, silme, yeniden adlandırma, truncate.
- Query Tool, veri görüntüleme.
- SSH tüneli, `pgpass.conf` desteği.
- Çekirdek set dışındaki nesne türleri (Aggregates, Foreign Tables, FDW/Foreign Servers, Publications/Subscriptions, Event Triggers, Casts, Languages, Collations, FTS, Operators, Rules).
- Açık düğümlerin oturumlar arası hatırlanması, otomatik yenileme/değişiklik izleme.
- Uçtan uca (WebDriver) arayüz testleri.
- Kurulum paketinin üretimi ve dağıtımı (ayrı iş; WebView2 stratejisi §9'da not edildi).

---

## 2. Mimari

**Yaklaşım:** İş Rust'ta yapılır, arayüz yalnızca çizer. Bağlantılar, sürüme duyarlı katalog sorguları ve DDL üretimi Rust'tadır; arayüze tipli Tauri komutlarıyla hazır veri gider. WebView'e rastgele SQL çalıştırma yetkisi verilmez.

### 2.1 Proje yapısı

```
pg-tools/
├─ Cargo.toml                 Rust workspace
├─ package.json · vite.config.ts · bun.lock
├─ src/                       SolidJS ön yüz
│  ├─ app/                    Kabuk, bölünmüş panel düzeni, tema, strings.ts
│  ├─ features/
│  │  ├─ servers/             Sunucu diyaloğu, servers.json içe aktarma
│  │  ├─ explorer/            Ağaç store'u, sanal ağaç, filtre, sağ tık menüsü
│  │  └─ inspector/           Properties ve DDL sekmeleri
│  ├─ ipc/                    tauri-specta ile üretilmiş tipli komutlar
│  └─ ui/                     Kobalte üzerine ince, stillenmiş bileşenler
├─ src-tauri/                 İnce Tauri kabuğu: komutlar, durum, olaylar
└─ crates/
   ├─ pgt-core/               Sunucu kayıt defteri, kimlik bilgileri, bağlantı havuzu, TLS
   └─ pgt-catalog/            Düğüm modeli, katalog sorguları, Properties, DDL
```

### 2.2 İlkeler

- `src-tauri` yalnızca yönlendirme yapar; mantık crate'lerdedir ve Tauri olmadan `cargo test` ile test edilir.
- Rust ↔ TypeScript tipleri **tauri-specta** ile tek kaynaktan üretilir; tipler elle iki yerde yazılmaz.
- Ön yüz özellik bazlı düzenlenir; ortak parçalar yalnızca `ui/` ve `ipc/` altındadır.
- Açılış hızı: pencere boş durumla hemen çizilir; sunucu listesi ilk çizimden sonra yüklenir; hiçbir sunucuya otomatik bağlanılmaz; CodeMirror ve diyaloglar dinamik `import()` ile ilk kullanımda yüklenir; web fontu yoktur.
- Sonraki alt projeler (ör. `pgt-query`) aynı bağlantı katmanını kullanır.

---

## 3. Veri modeli

### 3.1 Düğüm kimliği

Nesneler isimle değil **OID** ile tanımlanır (yeniden adlandırmada kimlik değişmez). Klasör düğümleri sanal koleksiyonlardır:

```
srv:<uuid>                                       sunucu
srv:<uuid>/dbs                                   "Databases" koleksiyonu
srv:<uuid>/db:16384                              veritabanı
srv:<uuid>/db:16384/sch:2200/tables              "Tables" koleksiyonu
srv:<uuid>/db:16384/sch:2200/tbl:24576           tablo
srv:<uuid>/db:16384/sch:2200/tbl:24576/col:3     kolon (attnum)
```

- Rust'ta tipli `enum NodeRef`; arayüze opak string olarak gider ve arayüz onu ayrıştırmaz.
- Sunucu grupları ayrı nesne değildir; sunucu yapılandırmasındaki `group` alanından türetilir. Grup düğümünün kimliği `grp:<grup-adı>`.

### 3.2 Arayüze giden tipler

```ts
TreeNode {
  id: string;
  kind: NodeKind;
  label: string;
  detail?: string;        // ör. kolon tipi, tablo için tahmini satır
  hasChildren: boolean;
  flags: { partitioned?, system?, inherited?, connected?, … };
}

PropertySection { title: string; rows: { label: string; value: string }[] }
```

### 3.3 Komutlar

| Komut | Dönüş | Not |
|---|---|---|
| `list_children(id)` | `TreeNode[]` | Tek seviye, tembel yükleme. Koleksiyon etiketine sayı eklenir: "Tables (1.243)". |
| `get_properties(id)` | `PropertySection[]` | Genel özellik tablosu; tür başına ayrı arayüz bileşeni yok. |
| `get_ddl(id)` | `string` | |
| `list_servers()` / `save_server(cfg, password?)` / `delete_server(id)` | | Kayıt defteri |
| `test_connection(cfg, password?)` | sürüm bilgisi | Diyalogdaki "Bağlantıyı test et" |
| `connect(id, password?)` / `disconnect(id)` | | |
| `preview_pgadmin_import(path)` / `apply_pgadmin_import(ids)` | | İçe aktarma |

### 3.4 Arayüz ağaç durumu

- `nodes: Map<id, TreeNode>`
- `children: Map<id, ChildState>` — `yüklenmedi | yükleniyor | id[] | hata`
- `expanded: Set<id>`, `selected: id | null`
- Görünür düğümlerin düz listesi `createMemo` ile türetilir ve TanStack Virtual ile çizilir; DOM'da yalnızca görünen satırlar bulunur.
- **Yenile:** çocuklar yeniden çekilir, id'ye göre eşleştirilir; mevcut düğümlerin genişletme/seçim durumu korunur, silinenler alt ağaçlarıyla temizlenir.

---

## 4. Bağlantı yönetimi (`pgt-core`)

### 4.1 Sunucu kayıt defteri

- Dosya: `%APPDATA%\pg-tools\servers.json`.
- Alanlar: `id` (UUID), `name`, `group`, `host`, `port`, `maintenance_db` (varsayılan `postgres`), `username`, `save_password`, `ssl { mode, root_cert, client_cert, client_key }`.
- Atomik yazma (geçici dosya + yeniden adlandırma).
- Parola bu dosyada **asla** bulunmaz.

### 4.2 Kimlik bilgileri

- `keyring` crate'i → Windows Credential Manager, anahtar `pg-tools:<sunucu-uuid>`.
- `save_password = false` ise bağlanırken parola sorulur ve yalnızca oturum boyunca bellekte tutulur.
- İstemci anahtarı parolası (passphrase) aynı yöntemle saklanır.
- Sunucu silindiğinde ilgili Credential Manager kayıtları da silinir.

### 4.3 Sürücü ve TLS

- Sürücü: `tokio-postgres` (SCRAM-SHA-256 ve md5).
- TLS: `rustls`. `sslmode` libpq anlamlarıyla:

| sslmode | Davranış |
|---|---|
| `disable` | TLS yok |
| `prefer` | TLS dene, olmazsa düz bağlantı; sertifika doğrulanmaz |
| `require` | TLS zorunlu; sertifika doğrulanmaz |
| `verify-ca` | Zincir doğrulanır, host adı doğrulanmaz |
| `verify-full` | Zincir ve host adı doğrulanır |

- `root_cert` verilmemişse Windows sertifika deposu kullanılır (`rustls-platform-verifier`); GPO ile dağıtılan şirket kök sertifikası ek ayarsız çalışır.
- İstemci sertifikası ve anahtarı PEM dosyalarından okunur.

### 4.4 Bağlantı modeli

- Bağlantılar **(sunucu, veritabanı)** çifti başına tutulur; PostgreSQL başka veritabanının kataloğunu sorgulatmaz.
- Sunucuya bağlanmak = bakım veritabanına bağlanmak (veritabanı listesi, roller, tablespace'ler için). Bir veritabanı genişletildiğinde o veritabanına ayrı bağlantı açılır.
- (sunucu, veritabanı) başına en fazla 2 bağlantılık havuz; 10 dk boşta kalan bağlantılar kapatılır.
- Her bağlantıda:
  - `application_name = 'pg-tools'`
  - `default_transaction_read_only = on`
  - `statement_timeout = 30s`
- Sürüm, bağlantıda gelen `server_version_num` parametresinden okunur.
- Bağlantı zaman aşımı: 10 sn.
- **Kopma:** işlem bir kez sessizce yeniden bağlanarak tekrar denenir; yine başarısızsa sunucu "bağlı değil" durumuna geçer ve hata rozeti gösterilir. Ağacın geri kalanı etkilenmez.
- Bekleyen bir yanıt artık geçerli değilse (ör. kullanıcı başka düğüm seçti) sonucu arayüzde yok sayılır.

### 4.5 pgAdmin içe aktarma

- pgAdmin `servers.json` export dosyası okunur; hem eski düz alan biçimi (`SSLMode`, `SSLRootCert`, `SSLCert`, `SSLKey`) hem pgAdmin 7+ `ConnectionParameters` biçimi desteklenir.
- Önizleme listesi gösterilir; aynı host + port + kullanıcı + isme sahip kayıtlar "zaten var" olarak işaretlenip varsayılan olarak seçilmez.
- `Group` alanı korunur. Parolalar pgAdmin tarafından dışa aktarılmadığından ilk bağlantıda sorulur.
- Desteklenmeyen alanlar (ör. SSH tüneli ayarları) yok sayılır ve önizlemede uyarı olarak gösterilir.

---

## 5. Katalog ve DDL katmanı (`pgt-catalog`)

### 5.1 Ağaç hiyerarşisi (çekirdek set)

```
Sunucu Grubu
└─ Sunucu
   ├─ Databases
   │  └─ Database
   │     ├─ Schemas
   │     │  └─ Schema
   │     │     ├─ Tables ─ Table
   │     │     │           ├─ Columns
   │     │     │           ├─ Constraints (PK / FK / Unique / Check / Exclusion)
   │     │     │           ├─ Indexes
   │     │     │           ├─ Triggers
   │     │     │           ├─ RLS Policies
   │     │     │           └─ Partitions
   │     │     ├─ Views ─ View ─ Columns
   │     │     ├─ Materialized Views ─ MatView ─ Columns, Indexes
   │     │     ├─ Functions / Procedures / Trigger Functions
   │     │     ├─ Sequences
   │     │     └─ Types / Domains
   │     └─ Extensions
   ├─ Login/Group Roles
   └─ Tablespaces
```

- Sistem şemaları (`pg_catalog`, `information_schema`, `pg_toast`, `pg_temp_*`) varsayılan olarak gizlidir; bir ayarla gösterilir.
- Partition'lar "Tables" listesinde görünmez; yalnızca ana tablonun "Partitions" düğümü altında listelenir.
- PG 10'da "Procedures" koleksiyonu gösterilmez.
- Extension'lara ait nesneler (ör. `pg_stat_statements` fonksiyonları) normal listelerde gösterilir ve `flags` ile işaretlenir.

### 5.2 Modül düzeni

```
crates/pgt-catalog/src/
├─ node.rs        NodeRef, NodeKind, TreeNode
├─ version.rs     PgVersion ve yetenek sorguları (has_procedures(), …)
├─ kinds/         Her nesne türü bir modül; trait ObjectKind { children(), properties(), ddl() }
├─ sql/           Her sorgu ayrı .sql dosyası, include_str! ile gömülü
└─ ddl/           Ortak yardımcılar: OWNER, COMMENT, GRANT (aclexplode), başlık bloğu
```

### 5.3 Sürüm stratejisi

- Varsayılan: sorgu tüm sürümlerde (10–18) çalışacak şekilde yazılır.
- Katalog farklıysa ayrı dosya: `functions.sql` (11+) ve `functions.pg10.sql`; seçim `PgVersion` ile yapılır. Eski sürüme özel dosyalar `.pg10` / `.pg11` sonekleriyle ayrışır ve ileride silinebilir.
- Bilinen kırılma noktaları (kapsamlı değil; uygulama sırasında her tür için doğrulanacak):
  - PG 11: `pg_proc.prokind`, procedure'lar.
  - PG 12: `pg_attribute.attgenerated`; `relhasoids` kaldırıldı.
  - PG 15 / 17: `pg_database` locale alanları (`daticulocale`, `datlocale`, `datlocprovider`).
  - PG 16: `pg_auth_members` yeni alanları (`inherit_option`, `set_option`).
  - PG 18: NOT NULL kısıtları `pg_constraint` içinde (`contype = 'n'`); virtual generated column'lar (`attgenerated = 'v'`).
- 18'den yeni sürümlerde en güncel dal kullanılır; PG 10'dan eski sürümlerde bağlantı engellenmez, uyarı gösterilir.

### 5.4 Tırnaklama

İsimler sorgu içinde `quote_ident()` / `format('%I')` / `::regclass::text` ile sunucuda hazırlanır. Rust'ta rezerve kelime listesi tutulmaz.

### 5.5 DDL üretimi

Çıktı biçimi pgAdmin'e benzer: başlık yorumu (`-- Table: public.orders`) ve yorum satırı olarak `-- DROP TABLE IF EXISTS public.orders;`.

| Nesne | Kaynak |
|---|---|
| Table | Elle birleştirme: kolonlar (`format_type`, `pg_get_expr` ile default / generated / identity, collation), kısıtlar (`pg_get_constraintdef`), `PARTITION BY` (`pg_get_partkeydef`), `PARTITION OF … FOR VALUES` (`relpartbound`), `INHERITS`, `UNLOGGED`, `WITH (reloptions)`, tablespace; ardından OWNER, COMMENT (tablo + kolonlar), GRANT, kısıta bağlı olmayan index'ler, trigger'lar, `ENABLE ROW LEVEL SECURITY` + policy'ler |
| View | `pg_get_viewdef` + `security_barrier`, `check_option` |
| Materialized View | `pg_get_viewdef` + `WITH [NO] DATA` + index'ler |
| Function / Procedure / Trigger Function | `pg_get_functiondef` |
| Index | `pg_get_indexdef` |
| Constraint | `ALTER TABLE … ADD CONSTRAINT … ` + `pg_get_constraintdef` |
| Trigger | `pg_get_triggerdef` |
| Policy | `pg_policy` + `pg_get_expr` ile elle birleştirme |
| Column | `ALTER TABLE … ADD COLUMN …` biçiminde parça |
| Sequence | `pg_sequence` + `OWNED BY` |
| Type | enum (`pg_enum`), composite, range (`pg_range`); base type için yalnızca bilgi yorumu |
| Domain | `format_type` + kısıtlar |
| Schema / Extension / Database / Tablespace | Katalog alanlarından birleştirme |
| Role | Rol nitelikleri + üyelikler; **parola hiçbir zaman yer almaz** |

OWNER, COMMENT ve GRANT blokları tüm türler için ortak yardımcılardan üretilir.

### 5.6 Properties

Seçilen düğüm için tek sorgu; bölümler:

- **Genel:** ad, OID, sahip, yorum
- **Tanım:** türe özgü alanlar
- **İstatistik** (tablo/matview/index): tahmini satır, `pg_total_relation_size`, son vacuum/analyze

Boyut hesaplamaları yalnızca seçimde yapılır, liste sorgularında asla.

### 5.7 Performans kuralları

- Liste sorguları yalnızca ağaçta gösterilecek kolonları çeker.
- Koleksiyon başına tek sorgu (N+1 yok).
- `hasChildren` sorgu içinde hesaplanır veya tür gereği sabittir (ör. tablo her zaman `true`); çocuklar önceden çekilmez.

---

## 6. Arayüz

### 6.1 Düzen

```
┌──────────────────────────────────────────────────────────────────────┐
│ [+ Sunucu] [İçe aktar] [⟳]            [ Filtre…  Ctrl+F ]       [◐]  │
├─────────────────────────┬────────────────────────────────────────────┤
│ ▾ Üretim                │  public.orders · Table    [Properties][DDL] │
│   ▾ ● pg-prod-01  16.4  │ ────────────────────────────────────────── │
│     ▾ Databases (4)     │  Genel / Tanım / İstatistik                │
│       ▾ app             │                                            │
│         ▾ Schemas (3)   │                                            │
│           ▾ public      │                                            │
│             ▾ Tables (1.243)                                         │
│ ▸ Test                  │                                            │
├─────────────────────────┴────────────────────────────────────────────┤
│ pg-prod-01 · PostgreSQL 16.4 · app · bağlı                           │
└──────────────────────────────────────────────────────────────────────┘
```

- Sol/sağ panel arası sürüklenebilir ayırıcı; genişlik `localStorage`'da saklanır.
- Gruba atanmamış sunucular kök seviyede listelenir.

### 6.2 Ağaç etkileşimleri

- **Tek tık:** seçer; sağ panel 150 ms debounce ile yüklenir, eskimiş yanıtlar yok sayılır.
- **Çift tık / Enter / →:** genişletir. Bağlı olmayan sunucuda genişletme bağlanmayı başlatır (gerekirse parola sorar).
- **Klavye:** ↑↓ gezinme; ← daraltır, kapalıysa üst düğüme çıkar; Home/End; F5 seçili düğümü yeniler; Ctrl+F filtreye odaklanır; Ctrl+C adı kopyalar.
- **Erişilebilirlik:** `role="tree"` / `treeitem`, `aria-expanded`, `aria-level`, `aria-setsize`, `aria-posinset`.
- **Filtre:** yalnızca yüklenmiş düğümlerde, büyük/küçük harf duyarsız alt dize; eşleşenler üst düğümleriyle gösterilir; sunucuya sorgu gitmez.
- **Sağ tık menüsü** (Kobalte ContextMenu, yalnızca okuma işlemleri): Yenile; Bağlan / Bağlantıyı kes; Sunucuyu düzenle / sil (onaylı); Adı kopyala; Nitelikli adı kopyala; DDL'i kopyala.

### 6.3 Diyaloglar

- **Sunucu diyaloğu** (Kobalte Dialog + Tabs):
  - Genel: ad, grup (mevcut gruplardan seç veya yeni yaz).
  - Bağlantı: host, port, bakım veritabanı, kullanıcı, parola, "Parolayı kaydet".
  - SSL: `sslmode`, CA / istemci sertifikası / istemci anahtarı (Tauri dialog eklentisiyle dosya seçici).
  - "Bağlantıyı test et" düğmesi.
- **İçe aktarma diyaloğu:** dosya seç → onay kutulu önizleme listesi → içe aktar.
- **Parola istemi:** kayıtlı parola yoksa bağlanırken açılır; "Parolayı kaydet" seçeneği içerir.

### 6.4 Sağ panel

- **Properties:** bölüm başlıklı iki kolonlu tablo.
- **DDL:** salt okunur CodeMirror 6, PostgreSQL vurgulaması, "Kopyala" düğmesi; CodeMirror bu sekme ilk açıldığında yüklenir.

### 6.5 Görünüm

- Sistem temasını izleyen açık/koyu tema, elle değiştirilebilir; renkler Tailwind 4 `@theme` değişkenleri.
- Sıkı yoğunluk: ağaç satırı 24 px.
- Fontlar: Segoe UI Variable (arayüz), Cascadia Mono → Consolas (kod).
- İkonlar: `lucide-solid`.
- Metinler `src/app/strings.ts` içinde toplanır; Türkçe komutlar, İngilizce PostgreSQL terimleri. i18n kütüphanesi yok.

---

## 7. Hata yönetimi

### 7.1 Hata modeli

```rust
enum AppError {
    Config(..),
    Credential(..),
    Connect { kind: Auth | Network | Tls | Timeout, .. },
    Query { sqlstate, message, detail, hint },
    NotFound(NodeRef),
}
```

Arayüze `{ code, message, detail?, hint? }` olarak gider. Anlaşılır Türkçe mesaja çevrilen durumlar: hatalı parola (`28P01`), veritabanı yok (`3D000`), bağlantı reddedildi, sertifika doğrulanamadı, zaman aşımı, yetki yok (`42501`).

### 7.2 Gösterim

- Düğüm hatası: kırmızı rozet + üzerine gelince detay + "Tekrar dene".
- Properties/DDL hatası: sağ panel içinde.
- Eylem sonuçları: kısa süreli bildirim (toast).
- `NotFound`: düğüm "nesne artık mevcut değil" olarak işaretlenir, üst düğümü yenileme önerilir.
- Rutin hatalar için modal diyalog açılmaz.

### 7.3 Dayanıklılık ve loglama

- `servers.json` okunamazsa `servers.json.bak-<zaman>` olarak yedeklenir, uygulama boş listeyle açılır ve kullanıcı uyarılır.
- `tracing` ile `%LOCALAPPDATA%\pg-tools\logs` altına dönen log; son 5 dosya. Panik kancası log'a yazar.
- Parolalar ve bağlantı dizgileri asla loglanmaz.

---

## 8. Test stratejisi

| Katman | Kapsam | Araç |
|---|---|---|
| Rust birim | `NodeRef` gidiş-dönüş, sürüme göre sorgu seçimi, `sslmode` eşleme, ACL/GRANT yardımcıları, pgAdmin `servers.json` ayrıştırma (iki biçim için fixture) | `cargo test` |
| Katalog entegrasyon | Tüm çekirdek nesne türlerini içeren fixture şeması (partition, RLS, generated/identity, enum/composite/range/domain, procedure, index'li matview, yorumlar, yetkiler); her koleksiyonun listelenmesi ve her nesnenin DDL'i; DDL snapshot'ları | gerçek PostgreSQL + `insta` |
| DDL gidiş-dönüş | Üretilen DDL boş veritabanına uygulanır, iki veritabanının katalogları karşılaştırılır | gerçek PostgreSQL |
| Sürüm matrisi | Aynı entegrasyon testleri PG 10, 12, 14, 16, 17, 18 üzerinde | Docker |
| Performans | 5.000 tablolu fixture ile `list_children` < 300 ms | entegrasyon testi |
| Ön yüz | Ağaç store'u: düzleştirme, yenileme eşleştirmesi, filtre; IPC mock'lu | Vitest + `@solidjs/testing-library` |
| Açılış / bellek / paket boyutu | Yayın öncesi ölçüm kontrol listesi | elle |
| Duman testi | Bağlan, ağaçta gez, DDL kopyala, içe aktar, kopma senaryosu | elle kontrol listesi |

- Bağlantı adresleri `PGT_TEST_URLS` ortam değişkeninden okunur; tanımlı değilse entegrasyon testleri atlanır ve `cargo test` yine geçer.
- Günlük geliştirme: yerel PostgreSQL 18. Sürüm matrisi: Docker Desktop açıkken tek komut.

---

## 9. Paketleme notları (bu alt projede uygulanmaz)

- Windows 10/11 varyantı: NSIS kurulum paketi, WebView2 `embedBootstrapper`.
- Server 2019/2022 varyantı: WebView2 `offlineInstaller` gömülü (internetsiz makineler için).
- Tauri `minimumWebview2Version` ayarı belirlenecek.

---

## 10. Açık riskler

| Risk | Etki | Azaltma |
|---|---|---|
| tauri-specta'nın Tauri 2 sürümüyle uyumu | Tip üretimi elle yapılmak zorunda kalabilir | Plan aşamasında doğrulanacak; gerekirse `specta` + elle yazılmış ince komut sarmalayıcıları |
| `rustls-platform-verifier` ile `verify-ca` (host adı kontrolsüz) birleşimi | Özel doğrulayıcı gerekir | `verify-ca` için özel `ServerCertVerifier`; testte kendi imzalı CA ile doğrulanır |
| Table DDL'inin elle birleştirilmesi | Kenar durumlarda eksik/yanlış DDL | DDL gidiş-dönüş testi + sürüm matrisi |
| WebView2 bellek tüketimi | Bellek hedefini zorlayabilir | Ölçüm kontrol listesi; hedef aşılırsa WebView2 tarayıcı argümanlarıyla ince ayar |
