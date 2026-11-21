# Nesne Diyalogları — FDW / Publication / Subscription Explorer Düğümleri (Plan 38) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Grup C'nin ilk adımı: Explorer ağacına altı yeni nesne türü — **Foreign Data Wrapper**, **Foreign Server**, **User Mapping**, **Foreign Table** (+ kolonları), **Publication**, **Subscription** — listeleme, **Özellikler** (Inspector), **DDL** (Inspector / "DDL'i kopyala"), **nesne arama** (Plan 32) ve ikonlar. Gizli değerler (subscription bağlantı dizgesi, `password` seçeneği) hiçbir çıktıda ham görünmez. **Diyaloglar (oluştur/düzenle/sil) bu planda değil** (Plan 39 arka uç, Plan 40 ön yüz).

**Mimari:** `pgt-catalog`'un genel liste/özellik/DDL motoru (`kinds/engine.rs`, `kinds/registry.rs`: SQL dosyası + kayıt) aynen kullanılır; bağlantı dizgesi ya da seçenek maskesi gerektiren türlerin özellik/DDL'i Rust'ta elle yazılmış modüllerdedir (`kinds/database.rs` kalıbı) ve maskeleme **saf Rust işlevleriyle** yapılır (birim testli). Arayüz değişikliği yalnız ana paketteki ikon/menü kancasıdır ve **aynı görevde adı konmuş telafiyle** karşılanır.

**Teknoloji:** Rust 2024, tokio-postgres, ts-rs 12, insta, Docker matrisi (`scripts/test-matrix.ps1`), SolidJS, vitest.

**Spec:** `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md` — §1 (gizli değerler, sunucu kapsamı), **§4.1 (bağlayıcı)**, §6, §7. Temel: Explorer spec'leri ve Plan 32 nesne arama (`crates/pgt-catalog/src/search.rs`). Önceki planların Global Constraints maddeleri geçerlidir.

## Kararlar

1. **Yeni düğümler** (Rust `Coll`/`Seg`/`NodeKind`, TS bindings):

   | Koleksiyon (`Coll` etiketi, `NodeKind`) | Çocuk (`Seg` etiketi, `NodeKind`) | Konum | Etiket | Ayrıntı (`detail`) |
   |---|---|---|---|---|
   | `fdws` `foreignDataWrapperCollection` ("Foreign Data Wrappers") | `fdw` `foreignDataWrapper` | `db:D/fdws/fdw:N` | `fdwname` | handler adı (yoksa null) |
   | `fservers` `foreignServerCollection` ("Foreign Servers") | `fsrv` `foreignServer` | `…/fdw:N/fservers/fsrv:S` | `srvname` | `srvtype`/`srvversion` ("tür · sürüm", ikisi de yoksa null) |
   | `umaps` `userMappingCollection` ("User Mappings") | `um` `userMapping` | `…/fsrv:S/umaps/um:U` | rol adı ya da `PUBLIC` | null |
   | `ftables` `foreignTableCollection` ("Foreign Tables") | `ftbl` `foreignTable` (+ `columns`) | `…/sch:S/ftables/ftbl:T` | `relname` | sunucu adı |
   | `pubs` `publicationCollection` ("Publications") | `pub` `publication` | `db:D/pubs/pub:N` | `pubname` | "all tables" ya da "N tablo" |
   | `subs` `subscriptionCollection` ("Subscriptions") | `sub` `subscription` | `db:D/subs/sub:N` | `subname` | "etkin" / "devre dışı" |

   - Veritabanı çocuk sırası: Schemas, Extensions, **Foreign Data Wrappers, Publications, Subscriptions**. Şema çocukları: `Foreign Tables`, **Materialized Views'ün hemen ardından**.
   - FDW → `fservers` koleksiyonu; Foreign Server → `umaps` koleksiyonu; Foreign Table → `columns` koleksiyonu (mevcut kolon listesi; `relkind = 'f'`). User mapping, publication, subscription yapraktır.
   - `um:U` oid'i `pg_user_mapping.oid`'dir (`pg_user_mappings.umid`); `umuser = 0` → `PUBLIC`.
   - Eklentiye ait FDW/server/foreign table: `flags.fromExtension = true` (mevcut bayrak; liste SQL'i `pg_depend deptype = 'e'`).
   - Bütün yeni türler **veritabanı kapsamlıdır** (Subscription: yalnız `subdbid` = bağlı veritabanı).
   - **Tables koleksiyonu foreign table listelemez** (bugünkü davranış; `tables.sql` değişmez). Bölümlü tablonun yabancı bölümleri `partitions` altında kalır.
2. **Okuma izni ve gizlilik:**
   - **Subscription:** liste yalnız herkese açık sütunları okur (`subconninfo` **okunmaz**). Özellikler/DDL `subconninfo`'yu yalnız `has_column_privilege('pg_subscription', 'subconninfo', 'SELECT')` doğruysa okur; değilse özellik satırı "Bağlantı: görüntülemek için yetki gerekir" ve DDL'de `CONNECTION '********'`. Okunabilse bile **parola maskelenir**: `mask_conninfo(&str) -> String` (anahtar=değer biçimi: `password = değer` / `password='…'` (tırnak içi `\'` ve `\\` kaçışları) → `password=********`; URI biçimi `postgres(ql)://kullanıcı:parola@…` → `kullanıcı:********@…` ve sorgu dizgesindeki `password=` → `password=********`). Herhangi bir çözümleme belirsizliğinde **tüm dizge** `********` olur (güvenli varsayılan). Okuma hatası (42501) koleksiyonda boş liste, özelliklerde not; hata toast'u yok.
   - **User mapping / foreign server seçenekleri:** `password` anahtarlı seçeneğin (büyük/küçük harf duyarsız anahtar) değeri `********`: `mask_options(&[String]) -> Vec<String>` (`anahtar=değer` öğeleri; `=` yoksa öğe olduğu gibi). `pg_user_mappings.umoptions` NULL ise (yetki yok) özelliklerde "Seçenekler: görüntülemek için yetki gerekir", DDL'de `OPTIONS` yazılmaz + yorum satırı `-- seçenekler okunamadı (yetki)`.
   - Maskesiz değer hiçbir zaman `TreeNode`, `PropertySection`, DDL metni, arama sonucu, `tracing` ya da hata iletisine girmez (ham değer yalnız arka uç belleğinde, maskelemeden hemen önce).
3. **Özellikler (Inspector):** Genel (Ad, OID, Sahip, Yorum) + türe özgü bölümler:
   - FDW: Handler, Validator, Seçenekler, Eklenti (varsa).
   - Server: FDW, Tür, Sürüm, Seçenekler (maskeli), Eklenti.
   - User mapping: Rol (`PUBLIC`), Sunucu, Seçenekler (maskeli / yetki notu).
   - Foreign table: Şema, Sunucu, Seçenekler (tablo `ftoptions`), Kolon sayısı.
   - Publication: Tüm tablolar (evet/hayır), Yayınlanan işlemler (insert/update/delete/truncate (PG 11+)), `publish_via_partition_root` (PG 13+), `publish_generated_columns` (PG 18+), Tablo sayısı, Şemalar (PG 15+, `TABLES IN SCHEMA`).
   - Subscription: Etkin, Yayınlar, Slot adı, `synchronous_commit`, Bağlantı (maskeli ya da yetki notu), sürüme göre `binary` (14+), `streaming` (14+; 16+ `parallel`), `two_phase` (15+), `disable_on_error` (15+), `origin` (16+), `password_required`/`run_as_owner` (16+), `failover` (17+).
4. **DDL (Inspector, "DDL'i kopyala"):** mevcut `ddl::header`/`join_blocks`/`comment`/`grants` biçimi (başlık + `DROP … IF EXISTS` + gövde + yorum + yetkiler + sahip):
   - FDW: `CREATE FOREIGN DATA WRAPPER q [HANDLER f] [VALIDATOR f] [OPTIONS (…)]`, `ALTER … OWNER TO`, `GRANT USAGE ON FOREIGN DATA WRAPPER` (`fdwacl`).
   - Server: `CREATE SERVER q [TYPE '…'] [VERSION '…'] FOREIGN DATA WRAPPER f [OPTIONS (…maskeli…)]`, sahip, `GRANT USAGE ON FOREIGN SERVER` (`srvacl`).
   - User mapping: `CREATE USER MAPPING FOR {rol|PUBLIC} SERVER s [OPTIONS (…maskeli…)]` (sahip/yetki yok).
   - Foreign table: `CREATE FOREIGN TABLE s.t (kolon tip [OPTIONS (…)] [COLLATE] [NOT NULL] [DEFAULT …], …) SERVER s [OPTIONS (…)]`, sahip, yorum, kolon yorumları, `GRANT … ON TABLE` (`relacl`).
   - Publication: `CREATE PUBLICATION q FOR ALL TABLES | FOR TABLE s.t [(kolonlar)] [WHERE (…)], …[, TABLES IN SCHEMA s, …] WITH (publish = '…'[, publish_via_partition_root = …][, publish_generated_columns = …])` (sürüme göre), sahip, yorum. Tablosuz publication: `CREATE PUBLICATION q` + `WITH`.
   - Subscription: `CREATE SUBSCRIPTION q CONNECTION '<maskeli>' PUBLICATION p, … WITH (enabled = …, slot_name = …, …sürüme göre…)`, sahip, yorum. DDL başına not satırı: `-- Bağlantı dizgesindeki parola gizlenmiştir.`
   - Tanımlayıcılar `quote_ident`, değerler `quote_literal` (SQL'de `format('%I'/'%L')` ya da Rust `crate::quote`); maskeli metin de `quote_literal` ile yazılır.
5. **Nesne arama (Plan 32):** `SearchKind`'e `ForeignTable`, `ForeignServer`, `Publication`, `Subscription` (`FRAGMENTS`'e PG 10–99 parçaları; süzgeçler liste SQL'leriyle birebir). Sonucun `id`'si ağaçtaki düğüm kimliğidir (server için `db:D/fdws/fdw:F/fservers/fsrv:S` — FDW oid'i parçada taşınır); `path` etiket zinciri. Subscription parçası `subconninfo` okumaz ve `subdbid = current_database oid` süzgecini uygular. FDW ve user mapping aranmaz (spec §4.1 listesi).
6. **Ön yüz:**
   - `NodeIcon.tsx`: yeni türlere **mevcut içe aktarılmış** simgeler (yeni lucide simgesi yok): FDW → `Puzzle`, Server → `Server`, User mapping → `User`, Foreign table → `Table2`, Publication → `DatabaseZap`, Subscription → `Database`. Koleksiyonlar mevcut `Folder` (varsayılan).
   - `actions.ts`: `SCHEMA_QUALIFIED` += `"foreignTable"`; `SELF_ONLY` += `"foreignDataWrapper"`, `"foreignServer"`, `"publication"`, `"subscription"` (user mapping için nitelikli ad yok). Designer menüsü **yok** (`ddl-kinds.ts` değişmez).
   - Arama diyaloğu (tembel) yeni `SearchKind` etiketleri: "Foreign table", "Foreign server", "Publication", "Subscription" (`src/app/strings-search.ts`).
   - Kolon düğümü (foreign table kolonu): mevcut kolon eylemleri; "Düzenle…" **gösterilmez** (`ownedByTable` foreign table'da false kalır — mevcut kod; test ile sabitlenir).
7. **Ana paket telafisi (adı konmuş):** `NodeIcon.tsx`'in `switch`'i, davranışı birebir koruyan **tür → simge tablosu** (`Partial<Record<NodeKind, Component>>`) + yalnız bayrağa bağlı özel durumlar (`table` partitioned, `column`/`constraint` primaryKey, `role` group) biçimine yeniden yazılır; yeni türler tabloya eklenir. Görev sonunda ana paket ve başlangıç ≤ mevcut tavanlar (126,415 / 154,088); tavanlar ölçülen değere indirilir. Yetmezse **dur ve sor**.
8. **pgt-ddl hedefleri:** yeni düğümler designer hedefi **üretmez** (`target_of` → boş). `target.rs`'in "diğer düğümler boş" testine yeni parçalar eklenir.
9. **Kapsam dışı (Plan 39–40):** oluştur/düzenle/sil diyalogları, `IMPORT FOREIGN SCHEMA`, foreign table için "Verileri Görüntüle/Düzenle" ve Query Tool menüsü (backlog), subscription durum izleme.

## Global Constraints

- PG **10–18** vanilla; Docker yalnız `scripts/test-matrix.ps1` ile (`pgt-matrix-*` geçici kapsayıcılar, bitince silinir); başka kapsayıcı/işleme dokunulmaz; `PGT_TEST_URLS` yalnız test komutunun ortamında, hiçbir yere yazılmaz/yazdırılmaz. Test nesneleri benzersiz önekli (`pgt_cat_rt_<etiket>`) ve `guarded` temizlikle; postgres_fdw testleri `CREATE EXTENSION IF NOT EXISTS postgres_fdw` (resmî imajlarda vardır) ve kendi veritabanında.
- **Gizlilik (bağlayıcı):** Karar 2. Testlerde ayırt edici sır dizgesi (ör. `pgt_gizli_parola_7f3`) parola olarak kullanılır ve **liste, özellikler, DDL, arama sonucu ve yakalanan `tracing` çıktısında bulunmadığı** doğrulanır. Subscription testleri `WITH (connect = false)` ile (slot/çoğaltma gerekmez; `enabled = false`, `create_slot = false`, `slot_name = NONE`).
- SQL yalnız sabit dosyalardan/parçalardan; kullanıcı girdisi her zaman parametre. Liste SQL'leri `COLLATE "C"` sıralı. Sürüme bağlı sütunlar `VersionedSql` ile (`pg_publication.pubviaroot` 13+, `pubtruncate` 11+, `pg_publication_namespace` 15+, `pg_publication_rel.prqual/prattrs` 15+, `pubgencols` 18+, `subbinary`/`substream` 14+, `subtwophasestate`/`subdisableonerr` 15+, `suborigin`/`subpasswordrequired`/`subrunasowner` 16+, `subfailover` 17+; uygulayıcı her sütunu belgeyle ya da Docker'da doğrular, tahmin yazmaz).
- **Ana paket:** yalnız Task 4 dokunur (Karar 6–7); tavanlar yükseltilmez. Yeni arama etiketleri tembel modülde.
- `cargo test` `src/ipc/bindings/*.ts`'i LF ile yeniden yazar → `git checkout -- src/ipc/bindings/` ile gürültü geri alınır; **yeni ve içeriği değişen** binding dosyaları CRLF ile commit edilir. Dosyalar CRLF'dir; satır sonu korunur. Crate geneli `cargo fmt` yok. Yorumlar Türkçe, çevredeki yoğunlukta. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. `graphify-out/` ve `.superpowers/` commit edilmez. Git worktree kullanılmaz (dal `feat/fdw-nodes`). Ajanlar GUI'yi elle denemez: elle kontrol maddeleri `docs/smoke-test.md`'ye `[—]`.

## Review Focus

- **Parola sızıntısı:** `mask_conninfo` tüm biçimlerde (boşluklu `password = x`, tırnaklı ve kaçışlı değer, URI kullanıcı:parola, URI sorgu `password=`, büyük harf `PASSWORD`, birden çok parola, bozuk tırnak → tamamı maskeli); `mask_options` (`password=`, `PASSWORD=`, `=` yok); hiçbir çıktı yolunda (liste ayrıntısı, özellikler, DDL, arama, günlük) ham değer yok.
- **Yetkisiz okuma:** süper kullanıcı olmayan bir test rolüyle subscription listesi hatasız (conninfo okunmaz), özelliklerde yetki notu; user mapping seçenekleri NULL'da not.
- **Sürüm uyumu:** her liste/özellik/DDL sorgusu PG 10–18'de çalışır; sürüme özel sütunlar yalnız ilgili sürümde.
- **Ağaç yapısı:** yeni koleksiyonlar doğru yerde ve doğru sırada; foreign table Tables'ta yok; kolonlar açılır; arama sonuçlarının `id`'si ağaçta açılabilir (genişletme yolu geçerli).
- **Ana paket:** ikon tablosu yeniden yazımı mevcut türlerde aynı simgeyi verir (tablo testi); tavanlar yalnız düşer.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-catalog/src/node.rs` | `Coll`/`Seg`/`NodeKind` eklemeleri, etiketler |
| `crates/pgt-catalog/src/kinds/{mod.rs, registry.rs}` | Çocuk listeleri, kayıtlar |
| `crates/pgt-catalog/src/kinds/foreign.rs` (yeni) | FDW/server/user mapping/foreign table özellik ve DDL (maskeli seçenekler) |
| `crates/pgt-catalog/src/kinds/replication.rs` (yeni) | Publication/subscription özellik ve DDL (maskeli bağlantı) |
| `crates/pgt-catalog/src/mask.rs` (yeni) | `mask_conninfo`, `mask_options` (saf, birim testli) |
| `crates/pgt-catalog/src/sql/*.sql` (yeni) | `list_fdws`, `list_foreign_servers`, `list_user_mappings`, `list_foreign_tables`, `list_publications`, `list_subscriptions` + özellik/DDL sorguları (sürüm dosyalarıyla) |
| `crates/pgt-catalog/src/search.rs` | Dört yeni `SearchKind` |
| `crates/pgt-catalog/tests/*` | Docker liste/özellik/DDL/arama/gizlilik testleri |
| `crates/pgt-ddl/src/target.rs` | Boş hedef testi |
| `src/features/explorer/{NodeIcon.tsx, actions.ts}` (+ testler), `src/app/strings-search.ts`, `src/features/search/*`, `scripts/bundle-budget.json` | Ön yüz (Task 4) |
| `docs/smoke-test.md`, spec | Belgeler (Task 5) |

---

### Task 1: Düğüm modeli ve listeleme

**Dosyalar:** `node.rs`, `kinds/mod.rs`, `kinds/registry.rs`, yeni liste SQL'leri, `crates/pgt-catalog/tests/listing.rs` (ya da yeni `tests/foreign_listing.rs`), `crates/pgt-ddl/src/target.rs` (test), derleme uyumu gereken TS dosyaları (yalnız tip; ana paket baytı değişmemeli — `NodeKind` birleşimi büyür ama çalışma zamanı kodu değişmez).

- [ ] **Adım 1: Başarısız testler.** Birim: yeni `Coll`/`Seg` etiketlerinin gidiş-dönüşü (`fdws`, `fdw:1`, `fservers`, `fsrv:2`, `umaps`, `um:3`, `ftables`, `ftbl:4`, `pubs`, `pub:5`, `subs`, `sub:6`), `NodeKind` serde adları; `target_of` yeni düğümlerde boş. Docker (her sürümde): veritabanı çocuk sırası; postgres_fdw + server (`OPTIONS (host 'x', dbname 'y')`) + user mapping (`OPTIONS (user 'u', password '<sır>')`) + PUBLIC mapping + foreign table (iki kolon) + publication (tablo ile ve FOR ALL TABLES) + subscription (`connect = false`, sır parolalı bağlantı) → her koleksiyon doğru etiket/ayrıntı/yaprak bayrağıyla; FDW `fromExtension`; foreign table Tables'ta yok, `ftables`'ta var ve kolonları listelenir; ham sır hiçbir `TreeNode` alanında yok; süper kullanıcı olmayan rolle subscription listesi hatasız.
- [ ] **Adım 2–4:** uygula; `cargo test --workspace` (Docker'sız), clippy, matris (9 sürüm); `npm run typecheck` + `npm run bundle:check` (değişmemeli); bindings gürültüsü geri.
- [ ] **Adım 5: Commit** (`feat(catalog): FDW/server/user mapping/foreign table/publication/subscription düğümleri ve listeleme`).

### Task 2: Maskeleme, özellikler ve DDL

**Dosyalar:** yeni `mask.rs`, `kinds/foreign.rs`, `kinds/replication.rs`, özellik/DDL SQL'leri, `kinds/mod.rs` dağıtımı, testler (`tests/objects_properties.rs`, `tests/objects_ddl.rs` ya da yeni dosya; insta anlık görüntüleri).

- [ ] **Adım 1: Başarısız testler.** Birim (`mask.rs`): Review Focus'taki tüm `mask_conninfo`/`mask_options` biçimleri (tablo testi; çıktıda sır yok, diğer anahtarlar aynen). Docker: altı türün özellikleri (Karar 3 satırları, sürüme göre) ve DDL'i (Karar 4; insta anlık görüntüsünde oid/sürüm farkı normalize) — sır dizgesi hiçbir çıktıda yok; DDL'de `password '********'` ve `CONNECTION '…password=********…'`; yetkisiz rolle subscription özelliklerinde yetki notu ve DDL'de `CONNECTION '********'`; user mapping seçenekleri okunamayınca not; publication PG 15+ şema/kolon listesi/WHERE ve PG 13+ `publish_via_partition_root` DDL'de; DDL çıktısı PG'de çalıştırılabilir (maskesiz türlerde: FDW hariç — eklentiye ait — server/foreign table/publication DDL'i yeni bir veritabanında `DROP`+yeniden oluşturma ile doğrulanır, parola içermeyen örnekte).
- [ ] **Adım 2–4:** uygula; test; clippy; matris. **Adım 5: Commit** (`feat(catalog): FDW/replikasyon nesnelerinin özellikleri ve DDL'i; bağlantı dizgesi ve parola seçeneği maskelenir`).

### Task 3: Nesne arama

**Dosyalar:** `search.rs` (+ `tests/search.rs`), bindings (`SearchKind`).

- [ ] **Adım 1: Başarısız testler.** Docker: dört türün aranması (ad parçasıyla), sonucun `id`'si `list_children` ile adım adım genişletilerek bulunur (mevcut arama testi kalıbı), subscription başka veritabanındakini göstermez, sonuçta sır yok, PG 10–18.
- [ ] **Adım 2–4:** uygula; test; clippy; matris. **Adım 5: Commit** (`feat(catalog): nesne aramasına foreign table/server, publication ve subscription`).

### Task 4: Ön yüz — ikonlar, nitelikli ad, arama etiketleri ve telafi

**Dosyalar:** `NodeIcon.tsx` (+ yeni ya da mevcut test), `actions.ts` (+ `actions.test.ts`), `src/app/strings-search.ts`, arama modeli/diyaloğu (gerekirse), `scripts/bundle-budget.json`.

- [ ] **Adım 1: Başlangıç ölçümü** (126415 / 154088; değilse dur).
- [ ] **Adım 2: Başarısız testler.** `iconFor` tablo testi: **mevcut her tür** yeniden yazımdan önceki simgeyi verir (partitioned tablo, PK kolon/kısıt, group rol dahil) ve yeni altı tür Karar 6 simgesini; `actions.test.ts`: foreign table'da `copyQualifiedName` (şema.ad), FDW/server/publication/subscription'da `copyQualifiedName` (yalnız ad), user mapping'de yok, yeni düğümlerde designer eylemi yok, foreign table kolonunda `editObject` yok; arama modeli yeni türlerin etiketini döndürür.
- [ ] **Adım 3–4:** uygula (Karar 6–7).
- [ ] **Adım 5: Doğrulama.** `npm run test`, `npm run typecheck`, `npm run build`, `npm run bundle:check`; tavanları ölçülen değere indir (yalnız düşürme); arama etiketleri ana pakette yok.
- [ ] **Adım 6: Commit** (`feat(explorer): yeni FDW/replikasyon düğümlerinin ikonları ve nitelikli adları; ikon tablosu (ana paket telafisi, index N B)`).

### Task 5: Belgeler ve bütünleşik doğrulama

- [ ] **Adım 1:** Tam matris (9 sürüm) yeşil.
- [ ] **Adım 2: `docs/smoke-test.md`** — "## Explorer: FDW / Publication / Subscription düğümleri (Plan 38)" (`[—]` maddeler: ağaç yerleşimi ve sırası, ikonlar, özellikler ve DDL'de parola maskesi, yetkisiz kullanıcıyla subscription notu, foreign table kolonları, arama sonuçlarından düğüme gitme, eklentiye ait FDW işareti, koyu/açık tema) + ölçümler.
- [ ] **Adım 3: Spec** — durum satırı "Plan 34–38 uygulandı"; **§13 Plan 38 notları (Plan 39–40 için)**: düğüm kimlikleri ve etiketleri, maskeleme kuralları (`mask.rs`), okuma izni davranışı, sürüm sütunları, arama türleri, ikon tablosu telafisi ve ölçümler.
- [ ] **Adım 4: Bütünleşik doğrulama** (`cargo test --workspace`, clippy, `npm run test`, typecheck, build, bundle:check). Kapanış grep'i: sır test dizgesi `crates/` dışında yok; `tracing::` yeni dosyalarda değer yazmıyor.
- [ ] **Adım 5: Commit** (`docs(catalog): Plan 38 notları, smoke ve bütünleşik doğrulama`).

---

## Kapanış kontrolü

- Altı tür ağaçta, Inspector'da ve aramada; PG 10–18 yeşil; parola hiçbir çıktıda yok.
- Ana paket tavanları aşılmadı (yalnız düşmüş olabilir).
- `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme

- **Spec kapsamı:** §4.1 tablosu → Karar 1 (Task 1); "Özellikler/DDL/arama/ikon" → Karar 3–6 (Task 2–4); subscription okuma izni ve maske → Karar 2 (Task 1–2); user mapping maske → Karar 2; Tables foreign table listelemez → Karar 1; §1 gizli değerler → Global Constraints; §5 ana paket → Karar 7.
- **Bilinçli kararlar:** yeni lucide simgesi yok (bayt); foreign table için veri/Query Tool menüsü yok (backlog); belirsiz bağlantı dizgesi tamamen maskelenir.
- **Tip tutarlılığı:** `NodeKind` yeni adları (Task 1) → `NodeIcon`/`actions` (Task 4); `SearchKind` (Task 3) → arama etiketleri (Task 4); `mask_*` (Task 2) → arama (Task 3) yalnız adları döndürür, maskeye gerek yok.
