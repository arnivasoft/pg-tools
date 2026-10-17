# Yetkiler Ön Yüzü (Plan 22) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 20 arka ucunu arayüze bağlamak. Explorer'a "Yetkiler…" ve "Varsayılan yetkiler…" menüleri, designer sekmesinde yetki ızgarası, alıcı ekleme/kaldırma, CASCADE, önizleme ve kapılı uygulama eklenir.

**Mimari:** Mevcut designer sekmesi ve denetleyicisi `variant` ile genelleştirilir (`object` | `acl` | `default_acl`). Önizleme, uygulama, onay, üretim onayı ve kilit akışı aynen kullanılır. Kullanılabilir yetki sütunları arka uçtan gelir (`available`). Izgara saf bir modelle (`acl-grid-model.ts`) yönetilir. Ana pakete yalnız menü kancası ve iki komut sarıcısı girer; geri kalanı tembel parçalardadır.

**Teknoloji:** Rust (`pgt-ddl`, küçük ek), SolidJS, Kobalte (yalnız tembel), vitest, Docker matrisi.

**Spec:** `docs/superpowers/specs/2026-10-17-privileges-design.md` §7 (Plan 20 sözleşme notları) ve **§8 (Plan 22 kararları — bağlayıcı)**. Paket kuralları: `docs/superpowers/specs/2026-10-17-main-bundle-split-design.md` ve `docs/testing.md` "Ana paket bütçesi".

## Global Constraints

- **Dal:** `feat/privileges-ui`. Bu dal `feat/main-bundle-split` + `feat/privileges-backend` birleşimidir (31a0dc6). main'e birleştirme yalnız kullanıcı onayıyla yapılır.
- **Paket bütçesi:** her görevde `bun run bundle:check` geçmelidir. Ana paket ≤ `mainMaxBytes`, açılış kümesi ≤ `startupMaxBytes`, preload tam olarak `["solid","web"]` ve yasak paket yok. Ana pakete giren her bayt aynı görevde geri ödenir. Geri ödenemezse tavan, görev raporunda gerekçe ve ölçülen değerle yükseltilir; plan boyunca toplam en çok +600 B. Tembel görünümler ana modüllerden (strings, actions, tree-model, Button…) içe aktarmaz; gerekenleri props ile alır (Rolldown paylaşılan parça dersi). İstisna: zaten tembel olan designer modülleri, ki bunlar `strings-designer.ts`'i ve designer bileşenlerini zaten kullanıyor.
- **Arka uç değişikliği (yalnız Task 1):**
  - `AclDef.available` ve `DefaultAclSection.available` eklenir.
  - Reload equality ve sabit nokta korunur.
  - Değişiklik sonrası tam Docker matrisi yalnızca `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf` ile çalıştırılır. Kapsayıcılar geçicidir ve silinir; başka kapsayıcılara dokunulmaz. URL/parola yazdırılmaz.
- **Metin:** menü metinleri `src/app/strings.ts` `t.menu` içindedir (ana). Diğer tüm metinler yeni `src/app/strings-privileges.ts` dosyasındadır ve yalnız yetki formlarınca içe aktarılır. Metinler Türkçedir.
- **Erişilebilirlik:**
  - Izgara hücreleri tek tek erişilebilir adlı düğmelerdir. Izgara içinde ok tuşlarıyla gezinilir (gezgin tabindex).
  - Space/Enter durum değiştirir.
  - IME koruması (`ev.isComposing || ev.keyCode === 229`).
  - Odak yönetimi mevcut formlarla aynıdır (yükselen kenarda odak).
  - Salt okunur durumda kontroller devre dışıdır.
- **vitest:** ortam `node`. Mantık saf modellerde test edilir. GUI ajanlarca test edilemez; elle kontrol maddeleri `docs/smoke-test.md` "Yetkiler arayüzü (Plan 22)" bölümüne `[—]` olarak eklenir.
- **Değişmezler:**
  - Mevcut nesne formları ve designer davranışı değişmez; mevcut testler geçer.
  - `src/ipc/bindings` yalnız `cargo test` ile üretilir. Satır sonu gürültüsü `git checkout -- src/ipc/bindings` ile atılır; yeni ya da değişen binding'ler commit edilir.
  - Yorumlar Türkçedir. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Doğrulama:**
  - Ön yüz: `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`.
  - Rust değişen görevde ayrıca: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.

## Review Focus

- **Hata yolu eşlemesi:**
  - `grants[i].privileges[j]` arka uçta `draft.grants[i].privileges[j]` konumunu gösterir. Izgara modeli `privileges` dizisini kanonik (`available`) sırada tutmalı ki hata doğru hücreye düşsün.
  - Yeni satırlar ve satır silme indeksleri kaydırır. Bu durumda eski hatalar önizleme gelene kadar bayat kalabilir (bilinen pürüz). Yanlış hücreye kalıcı hata düşmemelidir.
- **Sabit nokta ve kirli durum:**
  - Hiç değişmeyen ızgara boş plan üretir. Bir hücreyi değiştirip geri almak kirli durumu temizler.
  - Varsayılan ACL'de (`explicit = false`) gösterilen satırlar düzenlenmeden `dirty = false` kalır.
  - Rol değişiminde kirli taslak için onay istenir.
- **Kural düğmeleri:**
  - PUBLIC satırında ve sahip satırında GRANT OPTION seçilemez.
  - Yinelenen alıcı eklenemez.
  - `available` dışı yetki gösterilmez. Arka uç gönderirse satır "bilinmeyen yetki" olarak salt okunur görünür.
  - Salt okunur tanımda (`editable = false`) hiçbir kontrol çalışmaz.
- **Kapılar ve sonuç:**
  - REVOKE planında onay istenir. CASCADE seçilirse üretimde ad yazma istenir.
  - Bayat tanım `needsReload` üretir. Kilitli sunucuda menü devre dışıdır, sekme de salt okunurdur.
  - Uygulama sonrası `ACL_WARNING` görünür ve tanım yeniden yüklenir.
- **Paket:** "Yetkiler…" menüsü ilk kez açıldığında yalnız designer parçaları iner. Açılış kümesi büyümez; olası büyüme yalnız menü kancası bayt kadardır ve gerekçesi rapora yazılır.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `crates/pgt-ddl/src/{acl.rs, load_acl.rs, build/acl.rs, build/default_acl.rs}` (+ testler) | `available` alanları (Task 1) |
| `src/ipc/commands.ts` | `ddlLoadAcl`, `ddlLoadDefaultAcl` (ana; Task 2) |
| `src/features/designer/designer-model.ts`, `designer-controller.ts`, `designer-open.ts`, `refresh-model.ts`, `DesignerView.tsx` | `variant` genelleştirmesi (Task 2) |
| `src/features/designer/acl-grid-model.ts` (+ test), `AclGrid.tsx`, `AclReadOnlyLists.tsx` | Izgara modeli ve bileşeni (Task 3) |
| `src/features/designer/PrivilegesForm.tsx`, `privileges-paths.ts`, `src/app/strings-privileges.ts` | "Yetkiler…" formu (Task 4) |
| `src/features/explorer/actions.ts`, `src/features/designer/ddl-kinds.ts`, `src/app/strings.ts` (`t.menu`), `src/app/App.tsx` (gerekirse) | Menü kancası (Task 4) |
| `src/features/designer/DefaultPrivilegesForm.tsx` | "Varsayılan yetkiler…" formu (Task 5) |
| `docs/smoke-test.md`, spec §8, `docs/testing.md` | Belgeler (Task 6) |

---

### Task 1: Arka uç — kullanılabilir yetkiler (`available`)

**Dosyalar:** `crates/pgt-ddl/src/acl.rs` (alanlar), `load_acl.rs` (doldurma), `build/acl.rs`, `build/default_acl.rs` (kimlik karşılaştırması ve doğrulama), `manager.rs` (gerekirse), testler, bindingler.

**Arayüzler — Üretir:**
- `AclDef { …, available: Vec<Privilege> }`: yükleyici `privileges_for(kind, version_num)` ile, kanonik sırada doldurur.
- `DefaultAclSection { …, available: Vec<Privilege> }`: `default_privileges_for(object_type, version_num)`.
- Üreticiler `available`'ı kimlik alanı sayar: eski ≠ yeni → `object` (ya da `sections`) alan hatası. Yetki kümesi doğrulaması mevcut sürüm kuralıyla aynı kalır.

- [ ] **Adım 1:** Başarısız testler:
  - Birim: `available` eşitsizliği → hata.
  - Docker: yüklenen `available` PG 16'da `MAINTAIN` içermez, PG 17+'da içerir. Sabit nokta korunur.
- [ ] **Adım 2–3:** uygula. **Adım 4:** `cargo test --workspace`, clippy, tam matris (Global Constraints), bindingler (`AclDef.ts`, `DefaultAclSection.ts`), `bun run typecheck` (TS kullanan yer yok).
- [ ] **Adım 5: Commit** (`feat(ddl): yetki tanımlarına kullanılabilir yetkiler (available)`).

---

### Task 2: Designer denetleyicisinin `variant` ile genelleştirilmesi ve komut sarıcıları

**Dosyalar:** `src/ipc/commands.ts`, `designer-model.ts`, `designer-controller.ts`, `designer-open.ts`, `refresh-model.ts`, `DesignerView.tsx` (+ ilgili testler).

**Arayüzler — Üretir:**

```ts
// commands.ts (ana)
ddlLoadAcl: (serverId: string, databaseOid: number | null, object: ObjectRef) => call<AclDef>("ddl_load_acl", { serverId, databaseOid, object }),
ddlLoadDefaultAcl: (serverId: string, databaseOid: number, scope: DefaultAclScope) => call<DefaultAclDef>("ddl_load_default_acl", { serverId, databaseOid, scope }),
// designer-model.ts
export type DesignerVariant = "object" | "acl" | "default_acl";
export type DesignerDef = ObjectDef | AclDef | DefaultAclDef;
// DesignerModel'e: variant: DesignerVariant; aclScope: DefaultAclScope | null; cascade (mevcut) yetki isteğinde de kullanılır
// designer-controller.ts
openAcl(tabId: string, ctx: OpenContext, object: ObjectRef): void;                  // ddlLoadAcl
openDefaultAcl(tabId: string, ctx: OpenContext, scope: DefaultAclScope): void;      // ddlLoadDefaultAcl (databaseOid zorunlu)
setAclRole(tabId: string, role: string): void;                                       // default_acl: kapsamı değiştir + yeniden yükle (kirliyse çağıran onay almış olmalı)
export function aclTabKey(kind: ObjectKind, serverId: string, databaseOid: number | null, oid: number): string;   // "acl:…"
export function defaultAclTabKey(serverId: string, databaseOid: number, schemaOid: number | null): string;       // "defacl:…"
```
- `planRequest()` variant'a göre şu istekleri üretir:
  - `acl` → `{op:"privileges", object, old, new, cascade}`
  - `default_acl` → `{op:"default_privileges", scope, old, new}`
- `loadObject`/yeniden yükleme variant'a göre doğru komutu çağırır. `needsReload` ve uygulama sonrası yeniden yükleme de aynı yolu izler.
- `cloneDef`, `defsEqual`, `dirtyOf` ve `defName` birleşim tipiyle çalışır. `defName`:
  - `acl` için nesne adı (`schema.name`, imza varsa `name(signature)`);
  - `default_acl` için `role` + (şema varsa) ` / schema`.
- `DesignerView`, `variant !== "object"` iken form olarak `FORMS` yerine iki yeni tembel girişten birini seçer:
  - `ACL_FORMS.acl` = `PrivilegesForm`
  - `ACL_FORMS.default_acl` = `DefaultPrivilegesForm`
  - Bu görevde ikisi de geçici yer tutucu bileşendir ("Hazırlanıyor"; Task 4/5 değiştirir). Yer tutucu dosya oluşturulmaz, giriş ikinci parça olarak eklenir. Başlık rozetinde `kindLabel` yerine "Yetkiler" / "Varsayılan yetkiler" metni gösterilir (`strings-designer.ts`'e).
- `designer-open.ts`:
  - `privileges` / `defaultPrivileges` kiplerini işler.
  - `ddlTargetOf(nodeId)` ile `object` alınır.
  - `defaultPrivileges` için `databaseOid` şöyle belirlenir: `db` düğümünde `object.oid`, `sch` düğümünde `target.databaseOid`. `schemaOid` ise `sch` düğümünde `object.oid`, `db` düğümünde `null` olur.
  - Sekme başlığı: `tdes.tab.privileges(label)` / `tdes.tab.defaultPrivileges(label)`.
  - Kilit ve bağlantı kontrolleri mevcut akışla aynıdır.
- `AppliedInfo`/`onApplied` yetki varyantlarında ad değişimi ya da yenileme düğümü üretmez; yalnız yeniden yükler. Inspector yenilemesi Task 4'tedir.
- **Paket:** `commands.ts` eklemesi ana pakete bayt ekler. Aynı görevde geri ödenir: `strings.ts`'ten tembel-yalnız bir metin bloğu (ör. yalnız tembel parçalarca kullanılan `t.query.strip` ya da `t.import` alt alanları) ilgili tembel `strings-*.ts`'e taşınır ve kullanımlar güncellenir. Taşınan bloğun ana pakette kullanıcısı kalmadığını `rg` ile doğrulayın.

- [ ] **Adım 1:** Başarısız testler (designer-model / controller birim testleri, mevcut test kalıbıyla):
  - `planRequest` iki yeni biçimi üretir.
  - `defName` iki varyantta doğru değeri verir.
  - `aclTabKey` ve `defaultAclTabKey` doğru anahtarı üretir.
  - `openAcl`, `ddlLoadAcl` çağırır (sahte api).
  - `setAclRole` kapsamı değiştirip yeniden yükler.
  - Değişmeyen taslakta `dirty` false olur.
  - Mevcut nesne testleri değişmeden geçer.
- [ ] **Adım 2–3:** uygula. **Adım 4:** typecheck, test, build, `bundle:check` (ana paket büyümemeli; geri ödeme rapora).
- [ ] **Adım 5: Commit** (`feat(designer): yetki sekmeleri için variant'lı designer modeli ve komut sarıcıları`).

---

### Task 3: Yetki ızgarası modeli ve bileşeni

**Dosyalar:** Oluştur `src/features/designer/acl-grid-model.ts` (+ `acl-grid-model.test.ts`), `AclGrid.tsx`, `AclReadOnlyLists.tsx`.

**Arayüzler — Üretir:**

```ts
// acl-grid-model.ts (saf)
export type CellState = "none" | "granted" | "grantable";
export function cellState(g: AclGrant, p: Privilege): CellState;
export function nextCellState(s: CellState, allowGrantOption: boolean): CellState;            // none→granted→(grantable)→none
export function setCell(grants: AclGrant[], row: number, p: Privilege, s: CellState, available: Privilege[]): AclGrant[];  // privileges kanonik (available) sırada; değişmez kopya
export function addGrantee(grants: AclGrant[], grantee: string | null): AclGrant[] | null;    // yinelenen → null
export function removeGrantee(grants: AclGrant[], row: number): AclGrant[];
export function allowGrantOption(g: AclGrant, owner: string): boolean;                        // PUBLIC ve sahip → false
export function unknownPrivileges(g: AclGrant, available: Privilege[]): Privilege[];          // available dışı (salt okunur gösterim)
export function cellPath(base: string, row: number, grants: AclGrant[], p: Privilege): string | null;   // "grants[i].privileges[j]" ya da null
export function nextCellFocus(row: number, col: number, rows: number, cols: number, key: "ArrowUp"|"ArrowDown"|"ArrowLeft"|"ArrowRight"|"Home"|"End"): [number, number];
```
- `AclGrid.tsx` props: `grants`, `available`, `owner`, `basePath` (`"grants"` ya da `"sections[i].grants"`), `issues`, `readOnly`, `labels` (yetki adları, durum metinleri), `onChange(grants)`, `onAdd`/`onRemove` (alıcı ekleme kontrolünü form sağlar).
- Kurallar spec §8'den. Hücre düğmesinin `aria-label`'ı "<alıcı> — <yetki>: <durum>" biçimindedir; `aria-pressed` kullanılmaz (üç durum), durum metinle verilir. Hücre hataları `cellPath` ile `issues`'tan okunur, hücre `aria-invalid` alır ve hata metni satır altında gösterilir.
- `AclReadOnlyLists.tsx`: `foreign` ve `columns` salt okunur tabloları (spec §8).

- [ ] **Adım 1:** Başarısız model testleri:
  - döngü (GRANT OPTION izinli/değil);
  - `setCell` kanonik sıra ve değişmezlik;
  - boşaltılan satırın kalması (tüm yetkiler kaldırıldı);
  - yinelenen alıcı (PUBLIC dahil);
  - `cellPath` indeksleri;
  - `unknownPrivileges`;
  - `nextCellFocus` kenarlar.
- [ ] **Adım 2–3:** uygula (bileşen saf modeli kullanır). **Adım 4:** typecheck, test, build, `bundle:check` (bileşenler henüz kullanılmıyorsa paket değişmez).
- [ ] **Adım 5: Commit** (`feat(designer): yetki ızgarası modeli ve bileşeni`).

---

### Task 4: "Yetkiler…" formu ve Explorer menü kancası

**Dosyalar:** Oluştur `PrivilegesForm.tsx`, `privileges-paths.ts`, `src/app/strings-privileges.ts`; değiştir `DesignerView.tsx` (`ACL_FORMS.acl`), `src/features/explorer/actions.ts`, `src/features/designer/ddl-kinds.ts`, `src/app/strings.ts` (`t.menu.privilegesObject`, `t.menu.defaultPrivilegesObject`), testler.

- **Form içeriği:**
  - Üst bilgi: sahip, bağlı rol, `explicit = false` notu.
  - `ReadOnlyReasons` (`def`).
  - `AclGrid`.
  - "Alıcı ekle" alanı: rol seçicisi `designer.lookup(tabId, {kind:"roles"})` datalist'iyle ve "PUBLIC" seçeneğiyle gelir; Enter ya da Ekle düğmesi alıcıyı ekler; geçersiz ya da yinelenen girişte satır içi ileti gösterilir.
  - CASCADE onay kutusu: plan görünümünde REVOKE yoksa devre dışıdır; mevcut `cascade` alanı kullanılır.
  - `AclReadOnlyLists`.
- `privileges-paths.ts`: üst şerit yolları (`object`, `schema`, `name`, `signature`, `foreign`, `columns`, `old.*`) ve ızgara yolları. `groupIssues` kalıbıyla çalışır.
- **Menü (ana):**
  - `ddl-kinds.ts`: `ACL_KINDS` (spec §8 düğüm türleri; `NodeKind` adlarıyla) ve `DEFAULT_ACL_KINDS` (`database`, `schema`).
  - `actions.ts`: `privilegesObject` / `defaultPrivilegesObject` eylemleri `editObject`'in yanında, aynı koşulla eklenir (sistem rolü kuralı gibi istisnalar yetkiye uygulanmaz; salt okunur kararını arka uç verir). Sıra: `editObject`, `renameObject`, `privilegesObject`, `defaultPrivilegesObject`, `dropObject`.
  - `Object` soneki sayesinde grup 3, kilit gerekçesi ve `openDesigner(id, mode)` mevcut mekanizmadan gelir. `DesignerOpenMode` iki kipe genişletilir.
- Inspector: uygulama sonrası, seçili düğüm sekmenin düğümüyse DDL sekmesi yenilenir. Mevcut yenileme kancası varsa kullanılır, yoksa rapora yazılır.
- **Paket:** menü kancası ana pakete bayt ekler. Geri ödeme Task 2'deki yöntemle yapılır; geri ödenemezse Global Constraints sınırı içinde gerekçeli tavan yükseltmesi.

- [ ] **Adım 1:** Başarısız testler:
  - `actionsFor`: ACL düğümlerinde `privilegesObject` var; `db`/`sch` düğümlerinde `defaultPrivilegesObject` var; `idx`, `trg`, `ext`, `role`, `con`, kolon ve koleksiyonlarda yok; sıralama ve grup doğru.
  - `disabledReason`: kilitli sunucuda gerekçe dolu.
  - `executeAction` doğru kiple `openDesigner` çağırır.
  - `privileges-paths` eşlemesi doğru.
  - Form alıcı ekleme mantığı doğru (saf yardımcıya ayrılır).
- [ ] **Adım 2–3:** uygula. **Adım 4:** typecheck, test, build, `bundle:check`.
- [ ] **Adım 5:** smoke maddeleri:
  - menü görünürlüğü ve kilit;
  - ızgara fare/klavye;
  - PUBLIC ve sahip GRANT OPTION kapalı;
  - alıcı ekle/kaldır;
  - CASCADE ve üretimde ad yazma;
  - `explicit` notu;
  - başka verenler ve kolon listeleri;
  - `ACL_WARNING`;
  - bayat tanım;
  - salt okunur nesne (aggregate, sistem şeması).

  **Commit** (`feat(designer): "Yetkiler…" sekmesi ve Explorer menüsü`).

---

### Task 5: "Varsayılan yetkiler…" formu

**Dosyalar:** Oluştur `DefaultPrivilegesForm.tsx`; değiştir `DesignerView.tsx` (`ACL_FORMS.default_acl`), `privileges-paths.ts` (bölüm yolları), `strings-privileges.ts`, testler.

- Kapsam satırı:
  - Rol seçici: `roles` lookup, varsayılan `viewer`.
  - Değişimde taslak kirliyse mevcut "değişiklikleri at" onayı (ConfirmDialogView, tembel) gösterilir, sonra `setAclRole`.
  - Rol yoksa / NotFound hatası satır içinde gösterilir.
  - Kapsam etiketi "Veritabanı geneli" ya da "Şema: <ad>".
- Her bölüm için başlık (Tablolar / Diziler / Fonksiyonlar / Tipler / Şemalar) ve `AclGrid` (`basePath = sections[i].grants`, sahip = `role`) gösterilir. Şema kapsamında altında "Genel varsayılan (salt okunur)" `AclGrid readOnly` (`baseline`) yer alır.
- Not: "Yalnızca bundan sonra oluşturulan nesneleri etkiler." (plan uyarısı zaten gelir; formda da kalıcı açıklama). PG 18 LARGE OBJECTS notu `ReadOnlyReasons` ya da bilgi satırı olarak gösterilir.

- [ ] **Adım 1:** Başarısız testler:
  - bölüm yolu eşlemesi (`sections[i].grants[j].privileges[k]` → doğru bölüm/hücre);
  - kirli taslakta rol değişimi onay ister (saf karar fonksiyonu);
  - şema kapsamında baseline salt okunur.
- [ ] **Adım 2–4:** uygula, doğrulamalar. **Adım 5:** smoke maddeleri (veritabanı ve şema düğümünden açma, rol değişimi + onay, baseline, yeni tabloda varsayılan yetkinin etkisi, `FOR ROLE` yetkisizlikte ipucu). **Commit** (`feat(designer): "Varsayılan yetkiler…" sekmesi`).

---

### Task 6: Belgeler ve bütünleşik doğrulama

**Dosyalar:** `docs/smoke-test.md` ("Yetkiler arayüzü (Plan 22)" bölümünü toparla; Plan 20 bölümündeki "GUI maddeleri Plan 22'de" notunu bu bölüme bağla), spec §8 durum + ölçümler, `docs/testing.md` (yeni test dosyaları).

- [ ] **Adım 1: Bütünleşik doğrulama:**
  - `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`;
  - `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo build -p pg-tools`.

  Ana paket ve açılış kümesi baytları rapora ve spec §8'e yazılır.
- [ ] **Adım 2: Kapanış taramaları:**
  - `rg "strings-privileges" src` → yalnız tembel yetki formları.
  - `rg "@kobalte" src/app src/features/explorer src/ipc` → yok.
  - `rg "ddlLoadAcl|ddlLoadDefaultAcl" src` → commands.ts + designer.
- [ ] **Adım 3: Commit** (`docs(designer): Plan 22 smoke, ölçümler ve bütünleşik doğrulama`).

---

## Kapanış kontrolü

- Yetkiler ve varsayılan yetkiler arayüzden uçtan uca kullanılabilir. Mevcut designer davranışı değişmez. Bütçe denetimi geçer.
- Elle kontrol `docs/smoke-test.md` "Yetkiler arayüzü (Plan 22)" bölümündedir. main'e birleştirme (üç dal birlikte) yalnız kullanıcı onayıyla yapılır.

## Öz inceleme

- **Spec §8 kapsamı:**

  | Spec §8 maddesi | Task |
  |---|---|
  | Giriş noktaları | T4 |
  | Sekme / variant / anahtarlar | T2 |
  | `available` | T1 |
  | Izgara | T3 |
  | Alıcı ekle/kaldır | T3/T4 |
  | Salt okunur bölümler | T3/T4 |
  | CASCADE | T4 |
  | Varsayılan yetkiler | T5 |
  | Hata yolları | T3/T4/T5 |
  | Uygulama sonrası | T2/T4 |
  | Paket | Global Constraints + T2/T4 |

- **Tip tutarlılığı:**
  - `DesignerVariant` / `DesignerDef` (T2) → formlar (T4/T5).
  - `acl-grid-model` imzaları (T3) → T4/T5.
  - `available` (T1) → `AclGrid.available` (T3).
  - `aclTabKey` / `defaultAclTabKey` (T2) → `designer-open` (T2).
  - Eylem kimlikleri ve kipler (T4) ↔ `DesignerOpenMode` (T2'de tip, T4'te menü).
- **Yer tutucu:** T2'deki geçici form girişleri T4/T5'te değiştirilir. Bu açıkça belirtilmiş bir ara durumdur.
