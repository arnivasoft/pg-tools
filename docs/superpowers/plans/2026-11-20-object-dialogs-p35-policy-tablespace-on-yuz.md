# Nesne Diyalogları — Policy / Tablespace Arayüzü (Plan 35) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 34 arka ucunun üstüne Grup A'nın **arayüzü**: Explorer'dan **Policy** (RLS politikası; `policies` koleksiyonu ve `pol:` düğümü) ve **Tablespace** (`tablespaces` koleksiyonu ve `ts:` düğümü) için **Oluştur… / Düzenle… / Yeniden adlandır… / Sil…**, tablespace için **Yetkiler…**; tembel formlar (`PolicyForm`, `TablespaceForm`), tablo diyaloğuna **Satır güvenliği (RLS)** bayrakları, tablo RLS/ad değişiminden sonra o tablonun açık politika sekmelerinin yeniden yüklenmesi ve elle kontrol listesi. Rust değişikliği **yoktur** (sözleşme boşluğu bulunursa düzeltilmez, raporlanır).

**Mimari:** Plan 11/13/15/19 kalıbı aynen: istemci SQL üretmez, `ObjectDef` düzenler; mantık saf modüllerde (`policy-model.ts`, `tablespace-model.ts`), Solid bileşenleri ince; metinler tembel modüllerde (`src/app/strings-policy.ts`, `src/app/strings-tablespace.ts`; tablo RLS metinleri mevcut tembel `strings-table.ts`'e). Ana pakete yalnız menü görünürlüğü (`ddl-kinds.ts`: `policy`, `tablespace` nesne türleri, `tablespace` ACL türü) girer ve **aynı görevde adı konmuş telafiyle** karşılanır.

**Teknoloji:** SolidJS, Tailwind 4, vitest (env `node`), mevcut designer çerçevesi (`designer-controller`, `SqlPreview`, `ApplyBar`, `FormParts` — `FieldRow`, `OwnerSelect`, `CommentField`, `FreeSqlNote`, `CheckRow`, `createFocusOnRequest` —, `ReadOnlyReasons`, `DropDialog`, `PrivilegesForm`).

**Spec:** `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md` — §1 kararlar, §2.1–2.3 Grup A, §5 arayüz, §6 güvenlik, **§9 "Plan 34 sözleşme notları" bağlayıcıdır**. Temel spec `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§3 risk, §4 arayüz) ve önceki arayüz planı `docs/superpowers/plans/2026-10-17-object-dialogs-p19-routines-types-on-yuz.md` — burada tekrarlanmayan Global Constraints maddeleri geçerlidir.

## Kararlar

1. **Menü:** `DDL_OBJECT_KINDS`'e `"policy"`, `"tablespace"` (NodeKind adları bindings'te doğrulandı: `policy`, `policyCollection`, `tablespace`, `tablespaceCollection`); `DDL_CREATE_COLLECTIONS` türetilmiş kalır. `ACL_KINDS`'e `"tablespace"` (Yetkiler…). Varsayılan yetkiler tablespace'te **yok** (spec §9.2). Sistem tablespace'leri (`pg_default`, `pg_global`) için menü gizlenmez; salt okunur kararını arka uç verir (form salt okunur açılır, Yeniden adlandır/Sil açık ret iletisi). Kilitli sunucuda mevcut kural (designer eylemleri kapalı) aynen.
2. **Politika komut kuralları formda aynalanır:** `insert` → `USING` alanı kapalı; `select`/`delete` → `WITH CHECK` kapalı (spec §2.1). Komut değişince yasak hâle gelen ifade **temizlenir** (`policyCommandPatch`; Plan 19 `normalizeTrigger` kalıbı). Diğer her kural arka ucun alan hatasıyla gösterilir.
3. **Yeniden kurma bilgisi istemcide hesaplanır:** `policyRebuildReasons(original, draft): string[]` spec §9.3 kuralını **aynalar** (komut değişimi, PERMISSIVE/RESTRICTIVE değişimi, `using` ya da `withCheck` dolu → boş). Liste doluysa formda bilgi notu "Politika aynı transaction içinde yeniden kurulur (Yıkıcı)" + nedenler. Seçim kutusu **yoktur** (arka uç otomatik yeniden kurar; risk rozeti ve onay kutusu mevcut kapıdan gelir). Arka uç yetkilidir.
4. **Boş ifade = yok:** `using`/`withCheck`/`comment` metin alanlarında boş ya da yalnız boşluk → `null` (`normalizeEmpty`). İfade metni başka biçimde değiştirilmez (kırpılmaz, `E'…'` eklenmez).
5. **Roller:** `roles` lookup'ından (`{ kind: "roles" }`) seçilir: "Rol ekle" `select` (listede olmayanlar) + eklenen her rol için kaldırılabilir çip; boş liste "PUBLIC (herkes)" olarak açıklanır. `public` öğe olarak sunulmaz. Yüklü rol lookup'ta yoksa çip olarak kalır. Çip sırası taslaktaki sıradır (yükleyici ada göre sıralı verir; sıra farkı arka uçta değişiklik sayılmaz). Çip hatası (`roles[i]`) rol adıyla gösterilir (TriggerForm `events.updateColumns[i]` kalıbı).
6. **RLS durumu:** politika formunda salt okunur satır "Tablo satır güvenliği: açık/kapalı, zorlanmış/zorlanmamış" (`tableRls`; `null` ise gösterilmez); kapalıysa uyarı notu (spec §2.1 metni). Önizleme uyarıları zaten arka uçtan gelir; form ayrıca uyarı üretmez.
7. **Tablespace formu:** `location` yalnız create'te düzenlenir (alter'da salt okunur metin; sistem tablespace'inde `—`); create'te "süper kullanıcı gerekir, dizin sunucuda var olmalı, boş ve PostgreSQL hizmet kullanıcısına ait olmalı" ipucu. Seçenekler dört metin alanı (boş → `null`, kırpılır); `maintenanceIoConcurrency` yanında "PG 13+" ipucu (sürüm kapısı arka uçta). İstemci yalnız **biçim uyarısı** gösterir (`tablespaceOptionWarnings`: sayı değil / negatif / tamsayı değil / 0–1000 dışı), gönderimi engellemez; asıl doğrulama arka uç alan hatasıdır.
8. **Tablo RLS bayrakları:** `TableGeneral.tsx`'e "Satır güvenliği (RLS)" bölümü: iki onay kutusu (`rowSecurity.enabled`, `rowSecurity.forced`). Alter notları `rowSecurityNotes(original, draft)`: `false → true` (ENABLE ya da FORCE) → "Politikası olmayan tabloda sahibi dışındakiler hiçbir satır görmez (Yıkıcı)"; `true → false` → "Tablonun politikaları devre dışı kalır". Create'te not yok. FORCE, ENABLE'dan bağımsızdır (PG izin verir); ipucu "FORCE yalnız RLS açıkken etkilidir".
9. **Politika sekmelerinin yenilenmesi (spec §9.3):** başarılı bir **tablo alter**'ından sonra (`kind = "table"`, `mode = "alter"`), `before`'a göre `rowSecurity`, `name` ya da `schema` değiştiyse: aynı `serverId` + `databaseOid`'deki, yüklü tanımı (`original`) `schema`/`table` ile o tabloya (`before.schema`, `before.name`) ait **politika** sekmeleri için: kirli değil ve uygulamada değilse `reload`; aksi hâlde `needsReload = true` (taslak ezilmez — mevcut kural). Saf yardımcı `policyTabsForTable(...)` `designer-model.ts`'te; denetleyici `applied` içinde çağırır.
10. **Silme:** politika ve tablespace için CASCADE/FORCE kutusu yok (`dropOptionsFor` zaten `{false,false}`); Destructive → yalnız onay kutusu, üretimde ad yazma **yok** (spec §9.3 kapısı; mevcut `applyGate` davranışı — değiştirilmez).
11. **Kapsam dışı:** politika ifadeleri için CodeMirror (küçük `textarea`, font-mono), varsayılan yetkilerde tablespace, tablespace'teki nesneleri taşıma, yerinde tablespace, şema karşılaştırmada politikalar (spec §8).

## Global Constraints

- **Paket bütçesi (bağlayıcı):** `scripts/bundle-budget.json` tavanları **yükseltilmez** (`mainMaxBytes 126582`, `startupMaxBytes 154255`). Ana pakete yalnız Task 1'in `ddl-kinds.ts` değişikliği girer; **aynı görevde** şu adı konmuş telafiyle karşılanır: yalnız tembel parçaların kullandığı `qtMore` komutu `src/ipc/commands.ts`'ten tembel bir modüle taşınır (tüketiciler: `src/features/query/QueryTab.tsx`, `src/features/data/data-commands.ts`, `src/features/data/data-controller.ts`; emsal `src/ipc/ddl-commands.ts`, `backup-commands.ts`, `dash-commands.ts`). Yetmezse sıradaki adaylar — **her biri `grep` ile "ana pakette tüketici yok" doğrulanarak**; ana pakette tüketicisi olan taşınmaz: `qtCancel` (QueryTab, ExportUi, data-commands), `settingsGet` (backup-controller, ai-ipc, AiPane, QueryTab), `getProperties` (yalnız Inspector — **Inspector `App.tsx`'te statik içe aktarılır, ana pakettedir: taşınmaz**). Telafi yetmezse **dur ve sor**. Görev sonunda tavanlar **ölçülen değere indirilir** (yalnız düşürme) ve ölçüm ledger'a + commit mesajına yazılır.
- **Boyut doğrulaması (her UI görevinin son adımı):** `bun run build` ve `bun run bundle:check` geçer; yeni form metinlerinden en az iki benzersiz dizge ana `index-*.js`'te **bulunmaz** (`Select-String -Path dist/assets/index-*.js -SimpleMatch "<dizge>"` boş). Tembel form parçaları (`PolicyForm-*.js`, `TablespaceForm-*.js`) ayrı dosyadır.
- **Tembel kural:** ana paket modülleri (`src/app/*` — `strings-*.ts` tembel modüller hariç —, `src/features/explorer/*`, `src/features/query/{QueryTabs,tabs-store,query-controller}.ts(x)`, `src/ipc/commands.ts`, `ddl-kinds.ts`) yeni form/model/metin modüllerini **statik içe aktarmaz**. Tembel formlarda Kobalte yok; lucide simgelerinden yalnız mevcut designer formlarının kullandıkları (`Trash2`, `Undo`, `X` yoksa metin düğme). Formlara yalnız `DesignerView`'ın `lazy(() => import(…))` girdileriyle erişilir.
- **İstemci SQL üretmez:** `using`/`withCheck` olduğu gibi gönderilir (Karar 4); altında `FreeSqlNote`. Tablespace `location` olduğu gibi gönderilir (kırpılmaz).
- **Sözleşme (spec §9):** yeni tanımlar `editable: true, unsupported: []` ile gider (`ddl_defaults` doldurur; form alanları düşürmez); `tableRls` salt okunurdur, form **yazmaz** (gönderilen taslakta yükleyicinin/varsayılanın değeri aynen kalır); politika `schema`/`table` değiştirilemez (salt okunur metin); tablespace `location` alter'da değiştirilemez.
- **FieldError yolları** spec §9.2: Policy `name, schema, table, roles[i], using, withCheck, comment, object` (+ `old.*` genel şeride); Tablespace `name, location, options.seqPageCost, options.randomPageCost, options.effectiveIoConcurrency, options.maintenanceIoConcurrency, owner, comment, object`. `*_PATHS` kümeleri kök anahtarlarla (`roles`, `options`) yazılır; `groupIssues` alt yolları kapsar. Tablo `rowSecurity.*` yolu üretilmez (TABLE_PATHS'e eklenmez).
- **Uygulama güvenliği** Plan 11 ile aynı ve **değişmez** (`applyGate`, onay kutusu, kilit, tek uçuş, `plan_changed` sonrası otomatik yükleme yok — Karar 9'daki yeniden yükleme yalnız **kirli olmayan** sekmede yapılır).
- **Tepkisellik dersleri:** denetleyicide okunan her yeni yapı mevcut `registry` sinyaline bağlanır; Solid sinyali kullanan her test dosyası başında `vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));`; saf model testleri Solid içe aktarmaz.
- **Gizlilik/erişilebilirlik/tema:** politika ifadeleri, adlar, konum ve SQL `console.*`'a ve depolamaya (`localStorage`/`sessionStorage`) yazılmaz; arayüz metinleri Türkçe, PostgreSQL terimleri İngilizce (`PERMISSIVE`, `RESTRICTIVE`, `USING`, `WITH CHECK`, `PUBLIC`, `LOCATION`); renkler yalnız CSS değişkenleri; her alan etiketli, hatalar `aria-describedby`; çip kaldırma düğmeleri klavyeyle erişilir ve `aria-label`'ı rol adını içerir; radio grupları `fieldset`/`legend`.
- **Test:** `bun run test` (vitest), `bun run typecheck`, `bun run build`, `bun run bundle:check`; `cargo test --workspace` (Docker'sız) yeşil kalır (Rust değişmez). `cargo test` `src/ipc/bindings/*.ts`'i LF ile yeniden yazarsa `git checkout -- src/ipc/bindings/`. Git worktree **kullanılmaz** (dal `feat/policy-tablespace`). Ajanlar GUI'yi elle denemez: elle kontrol maddeleri `docs/smoke-test.md`'ye `[—]` olarak yazılır. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. `graphify-out/` ve `.superpowers/` commit edilmez. Crate genelinde `cargo fmt` yok.

## Review Focus

- **Paket bütçesi:** Task 1 sonrası ve her UI görevinde `bundle:check` geçer; telafi taşıması davranışı değiştirmez (QueryTab "daha fazla satır" ve veri düzenleyicisi sayfalaması aynı komutu çağırır); tavanlar yalnız düşer.
- **Komut/ifade kuralları:** `insert`'te `USING`, `select`/`delete`'te `WITH CHECK` kapalı ve komut değişince temizlenir; `all`/`update` ikisini de kabul eder; boş metin `null`; yeniden kurma notu yalnız alter'da ve kural tetiklenince.
- **Roller:** boş liste PUBLIC açıklaması; `public` öğe değil; eklenen/kaldırılan çipler taslağa doğru yazılır; `roles[i]` hatası doğru çipe düşer (ekleme/silme sonrası kayma yok).
- **Salt okunurluk ve sözleşme alanları:** `editable = false` (sistem tablespace'i, eklentiye ait tablo politikası) formda hiçbir denetim düzenlenemez, gerekçeler listelenir; `editable`/`unsupported`/`tableRls` alanları gönderilen taslakta korunur; tablespace `location` alter'da salt okunur.
- **Politika sekmesi yenileme:** tablo RLS/ad/şema değişiminde yalnız o tablonun (aynı sunucu + veritabanı) politika sekmeleri; kirli sekme ezilmez (`needsReload`); başka tablo/veritabanı etkilenmez; tablo create ya da yalnız yorum değişimi tetiklemez.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `src/features/designer/ddl-kinds.ts`, `src/ipc/commands.ts`, yeni tembel komut modülü (ör. `src/ipc/qt-more-commands.ts` ya da mevcut tembel bir `*-commands.ts`), `QueryTab.tsx`, `data-commands.ts`, `data-controller.ts`, `src/features/explorer/actions.test.ts`, `scripts/bundle-budget.json` | **Ana paket (Task 1):** menü kancası, telafi, tavan düşürme |
| `src/features/designer/policy-model.ts` (+ test), `PolicyForm.tsx`, `src/app/strings-policy.ts` | Policy formu (Task 2) |
| `src/features/designer/tablespace-model.ts` (+ test), `TablespaceForm.tsx`, `src/app/strings-tablespace.ts` | Tablespace formu (Task 3) |
| `src/features/designer/{forms-model.ts, DesignerView.tsx}`, `src/app/strings-designer.ts` | `POLICY_PATHS`/`TABLESPACE_PATHS`, `FORMS` girdileri, geçici `formMissing` korumasının kaldırılması (Task 2–3) |
| `src/features/designer/TableGeneral.tsx`, `table-model.ts` ya da `table-form-state.ts` (+ test), `src/app/strings-table.ts`, `designer-model.ts` (+ test), `designer-controller.ts` (+ test) | Tablo RLS bayrakları ve politika sekmesi yenileme (Task 4) |
| `docs/smoke-test.md`, spec | Elle kontrol listesi, durum (Task 5) |

---

### Task 1: Ana paket — menü kancası ve adı konmuş telafi

**Dosyalar:** Değiştir `src/features/designer/ddl-kinds.ts`, `src/ipc/commands.ts`, `src/features/query/QueryTab.tsx`, `src/features/data/data-commands.ts`, `src/features/data/data-controller.ts` (gerekirse), `src/features/explorer/actions.test.ts`, `scripts/bundle-budget.json` (yalnız düşürme); Oluştur (gerekirse) tembel komut modülü.

**Arayüzler:**
- `ddl-kinds.ts`: `DDL_OBJECT_KINDS` sonuna `"policy", "tablespace"`; `ACL_KINDS` sonuna `"tablespace"`; dosya bağımlılıksız kalır; yorumlar güncellenir (Policy/Tablespace Plan 35; tablespace ACL yalnız `CREATE`, varsayılan yetki yok).
- Telafi: `qtMore` `commands.ts`'teki `api` nesnesinden çıkar; aynı imzayla (`(session: string, stmtIndex: number, onEvent: (ev: QueryEvent) => void) => …` — mevcut tanım birebir) tembel bir modüle taşınır; `QueryTab.tsx` ve veri düzenleyicisi yeni modülden alır (`DataBaseApi` tipi `Pick<Api, …>`'tan `qtMore` çıkar, tembel modülün tipiyle birleşir). Davranış aynı.

- [ ] **Adım 1: Başlangıç ölçümü.** `bun run build` + `bun run bundle:check`: ana **126582**, başlangıç **154255** (değilse dur ve raporla).
- [ ] **Adım 2: Başarısız testler.** `src/features/explorer/actions.test.ts`: `policyCollection` ve `tablespaceCollection`'da `createObject`; `policy` düğümünde `editObject`, `renameObject`, `dropObject` var, `privilegesObject` ve `defaultPrivilegesObject` **yok**; `tablespace` düğümünde `editObject`, `renameObject`, `privilegesObject`, `dropObject` var, `defaultPrivilegesObject`, `maintenance`, `backup` **yok**; kilitli sunucuda designer eylemleri mevcut kurala göre kapalı (mevcut kilit testinin kalıbıyla). Var olan veri düzenleyicisi/QueryTab testleri `qtMore` sahtesini nereden alıyorsa yeni konuma uyarlanır.
- [ ] **Adım 3: Başarısızlığı gör** (`bun run test -- actions`).
- [ ] **Adım 4: Uygula** (ddl-kinds + telafi).
- [ ] **Adım 5: Doğrulama.** `bun run test`, `bun run typecheck`, `bun run build`, `bun run bundle:check`. Ana ≤ 126582 ve başlangıç ≤ 154255 olmalı; olmazsa Global Constraints'teki sıradaki aday. Geçince `scripts/bundle-budget.json` tavanlarını ölçülen değerlere **indir** (eşitse dokunma). `grep -rn "qtMore" src --include=*.ts --include=*.tsx` çıktısında `src/ipc/commands.ts` yok.
- [ ] **Adım 6: Commit** (`feat(explorer): Policy/Tablespace menü kancası ve tablespace yetkileri; qtMore tembel modüle (ana paket telafisi, index N B)`).

---

### Task 2: Policy formu

**Dosyalar:** Oluştur `src/features/designer/policy-model.ts`, `src/features/designer/policy-model.test.ts`, `src/features/designer/PolicyForm.tsx`, `src/app/strings-policy.ts`; Değiştir `src/features/designer/forms-model.ts` (`POLICY_PATHS`), `src/features/designer/forms-model.test.ts`, `src/features/designer/DesignerView.tsx` (`policy` → `lazy(() => import("./PolicyForm"))`).

**Arayüzler (`policy-model.ts`, saf; Solid içe aktarmaz):**

```ts
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PolicyCommand } from "../../ipc/bindings/PolicyCommand";
import type { PolicyDef } from "../../ipc/bindings/PolicyDef";

export function asPolicy(d: ObjectDef | null): (PolicyDef & { kind: "policy" }) | null;
/** Tür uyuşmazlığında aynı nesneyi döndürür. */
export function policyPatch(d: ObjectDef, patch: Partial<PolicyDef>): ObjectDef;
/** Karar 2: hangi ifade alanı bu komutta izinli. */
export function expressionAvailability(command: PolicyCommand): { using: boolean; withCheck: boolean };
/** Komutu yazar ve yasak hâle gelen ifadeyi `null` yapar (insert → using; select/delete → withCheck). */
export function policyCommandPatch(d: PolicyDef, command: PolicyCommand): PolicyDef;
/** Karar 3 — alter'da DROP+CREATE gerektiren nedenler (metin anahtarları); create (`original === null`) → []. */
export type RebuildReason = "command" | "permissive" | "usingRemoved" | "withCheckRemoved";
export function policyRebuildReasons(original: PolicyDef | null, d: PolicyDef): RebuildReason[];
/** Karar 5: rol ekleme (yinelenen ve boş ad yok sayılır), kaldırma (indeks), seçilebilirler (lookup − mevcut, `public` hariç, ad sırasıyla). */
export function addRole(d: PolicyDef, name: string): PolicyDef;
export function removeRole(d: PolicyDef, i: number): PolicyDef;
export function roleChoices(lookup: readonly string[], current: readonly string[]): string[];
```

- `policyRebuildReasons`: `command` farklı → `"command"`; `permissive` farklı → `"permissive"`; `original.using !== null && d.using === null` → `"usingRemoved"`; aynısı `withCheck` için. Boş metin `null` sayılır (taslakta `normalizeEmpty` sonrası zaten `null`).
- `POLICY_PATHS = new Set(["schema", "table", "name", "roles", "using", "withCheck", "comment", "object"])`.
- **Form (`PolicyForm.tsx`):** `ReadOnlyReasons` (üstte); **Genel**: tablo (salt okunur metin `schema.table`), ad (create: "Ad"; alter: "Yeniden adlandır" + `createFocusOnRequest(props, "name", …)`), yorum (`CommentField`); **Komut ve tür**: komut `select` (`ALL`, `SELECT`, `INSERT`, `UPDATE`, `DELETE`; `policyCommandPatch`), tür `fieldset` radio (`PERMISSIVE` — "izin veren; politikalar VEYA ile birleşir", `RESTRICTIVE` — "kısıtlayan; VE ile birleşir"); **Roller** (Karar 5; boş listede "PUBLIC (herkes)" metni; lookup hatasında yalnız mevcut çipler); **İfadeler**: `USING` ve `WITH CHECK` font-mono `textarea` (3–6 satır), `normalizeEmpty`, kapalıyken `disabled` + neden ipucu ("INSERT politikasında USING kullanılmaz" / "SELECT ve DELETE politikasında WITH CHECK kullanılmaz"), altında `FreeSqlNote`; **RLS durumu** (Karar 6); **Yeniden kurma notu** (Karar 3; `role="note"`). `props.readOnly || !d.editable` iken tüm denetimler `disabled`.
- **Metinler** `src/app/strings-policy.ts` (`export const tpol = { … } as const;`): yukarıdaki etiketler, ipuçları, RLS satırı, yeniden kurma notu ve neden metinleri (`command`: "Komut değişti", `permissive`: "PERMISSIVE/RESTRICTIVE değişti", `usingRemoved`: "USING ifadesi kaldırıldı", `withCheckRemoved`: "WITH CHECK ifadesi kaldırıldı"), rol çipi `aria-label` (`removeRole: (name) => \`${name} rolünü kaldır\``), RLS kapalı uyarısı "Tabloda satır güvenliği kapalı; politika etkili olmaz.".

- [ ] **Adım 1: Başarısız testler** (`policy-model.test.ts`): `expressionAvailability` beş komut tablosu; `policyCommandPatch` (insert'e geçiş `using`'i siler `withCheck`'i korur; select/delete'e geçiş `withCheck`'i siler; all/update hiçbirini silmez; girdi değişmez — yeni nesne); `policyRebuildReasons` (create → []; aynı → []; komut; permissive; using dolu→null; withCheck dolu→null; null→dolu yeniden kurma değil; ad/rol/yorum değişimi yeniden kurma değil; birden çok neden); `addRole` (yinelenen, boş ve yalnız boşluk yok sayılır; sona ekler), `removeRole` (indeks dışı no-op), `roleChoices` (mevcutlar ve `public` çıkar, sıralı); `policyPatch` tür uyuşmazlığında aynı nesne; `asPolicy` başka türde `null`. `forms-model.test.ts`: `FORM_PATHS.policy === POLICY_PATHS`; `groupIssues` örneği: `roles[1]` ve `withCheck` alana, `old.using` genel şeride.
- [ ] **Adım 2: Başarısızlığı gör.**
- [ ] **Adım 3: Uygula** (model, metinler, form, `POLICY_PATHS`, `DesignerView` girdisi).
- [ ] **Adım 4: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (`PolicyForm-*.js` ayrı parça; ör. "RESTRICTIVE; VE ile birleşir" benzeri iki benzersiz dizge ana pakette yok; `grep -rn "strings-policy" src` yalnız form/model).
- [ ] **Adım 5: Commit** (`feat(designer): Policy formu (komut/tür, roller, USING/WITH CHECK, RLS durumu, yeniden kurma notu)`).

---

### Task 3: Tablespace formu ve geçici korumanın kaldırılması

**Dosyalar:** Oluştur `src/features/designer/tablespace-model.ts`, `src/features/designer/tablespace-model.test.ts`, `src/features/designer/TablespaceForm.tsx`, `src/app/strings-tablespace.ts`; Değiştir `src/features/designer/forms-model.ts` (`TABLESPACE_PATHS`; geçici yorum kaldırılır), `src/features/designer/forms-model.test.ts`, `src/features/designer/DesignerView.tsx` (`tablespace` → `lazy(() => import("./TablespaceForm"))`; `formMissing` bileşeni ve yorumu **silinir**), `src/app/strings-designer.ts` (`view.formMissing` **silinir**).

**Arayüzler (`tablespace-model.ts`, saf):**

```ts
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { TablespaceDef } from "../../ipc/bindings/TablespaceDef";
import type { TablespaceOptions } from "../../ipc/bindings/TablespaceOptions";

export function asTablespace(d: ObjectDef | null): (TablespaceDef & { kind: "tablespace" }) | null;
export function tablespacePatch(d: ObjectDef, patch: Partial<TablespaceDef>): ObjectDef;
export type TablespaceOptionKey = keyof TablespaceOptions;
/** Metni kırpar; boş → null. Diğer seçenekler değişmez (yeni nesne). */
export function tablespaceOptionPatch(d: TablespaceDef, key: TablespaceOptionKey, text: string): TablespaceDef;
/** Karar 7 — engellemeyen biçim uyarıları (metin anahtarları); boş/null alan uyarı üretmez. */
export type OptionWarning = "notNumber" | "negative" | "notInteger" | "outOfRange";
export function tablespaceOptionWarnings(o: TablespaceOptions): Partial<Record<TablespaceOptionKey, OptionWarning>>;
```

- Uyarı kuralları: `seqPageCost`/`randomPageCost` → `Number(text)` sonlu değilse `notNumber`, < 0 ise `negative`; `effectiveIoConcurrency`/`maintenanceIoConcurrency` → `/^\d+$/` değilse (`-` ile başlıyorsa `negative`, aksi hâlde sayıysa `notInteger`, değilse `notNumber`), sayı > 1000 ise `outOfRange`.
- `TABLESPACE_PATHS = new Set(["name", "owner", "location", "options", "comment", "object"])`.
- **Form (`TablespaceForm.tsx`):** `ReadOnlyReasons`; ad (create/alter etiketleri + `createFocusOnRequest(props, "name", …)`; "pg_ ile başlayamaz" ipucu), sahip (`OwnerSelect`), **Konum** (create: zorunlu metin, `spellcheck={false}`, `autocomplete="off"`, ipucu Karar 7 + "örn. D:\\pgdata\\ts1 ya da /var/lib/postgresql/ts1"; alter: salt okunur metin, boşsa `—`, ipucu "Konum oluşturulduktan sonra değiştirilemez"), create'te üstte bilgi notu "Tablespace oluşturmak süper kullanıcı yetkisi ister.", **Seçenekler** (dört alan: `seq_page_cost`, `random_page_cost`, `effective_io_concurrency`, `maintenance_io_concurrency` — etiketler PostgreSQL adlarıyla, boş = "ayarsız (sunucu varsayılanı)" yer tutucusu, `inputmode="decimal"`/`"numeric"`, uyarı satırı `tablespaceOptionWarnings`'ten, `maintenance_io_concurrency` "PG 13+" ipucu; hata yolu `options.<camelKey>` her alanın `FieldRow path`'idir), yorum. `props.readOnly || !d.editable` → hepsi `disabled`.
- **Metinler** `src/app/strings-tablespace.ts` (`export const ttsp = { … } as const;`).

- [ ] **Adım 1: Başarısız testler** (`tablespace-model.test.ts`): `tablespaceOptionPatch` (kırpma, boş → null, diğer anahtarlar korunur, girdi değişmez); `tablespaceOptionWarnings` tablosu (`"1.5"` yok, `"abc"` notNumber, `"-1"` negative, `"1.5"` io için notInteger, `"1001"` outOfRange, `"1000"` yok, `"0"` yok, `null` yok, `"1e3"` cost için geçerli); `tablespacePatch` tür uyuşmazlığında aynı nesne. `forms-model.test.ts`: `FORM_PATHS.tablespace === TABLESPACE_PATHS`; `options.maintenanceIoConcurrency` alana, `old.location` genel şeride; **her `ObjectKind` için `FORM_PATHS` girdisi boş olmayan bir küme** (16 tür).
- [ ] **Adım 2: Başarısızlığı gör.**
- [ ] **Adım 3: Uygula.** `grep -rn "formMissing\|FORM_MISSING" src` → 0.
- [ ] **Adım 4: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (`TablespaceForm-*.js` ayrı parça; iki benzersiz dizge ana pakette yok).
- [ ] **Adım 5: Commit** (`feat(designer): Tablespace formu (konum, seçenekler, biçim uyarıları); form yok koruması kaldırıldı`).

---

### Task 4: Tablo RLS bayrakları ve politika sekmelerinin yenilenmesi

**Dosyalar:** Değiştir `src/features/designer/TableGeneral.tsx`, `src/app/strings-table.ts`, tablo durum/model modülü (`table-form-state.ts` ya da `table-model.ts` — mevcut not yardımcılarının bulunduğu dosya; uygulayıcı `persistenceChanged` benzeri yardımcıların yerini izler) + testi, `src/features/designer/designer-model.ts` (+ `designer-model.test.ts`), `src/features/designer/designer-controller.ts` (+ `designer-controller.test.ts`).

**Arayüzler:**

```ts
// tablo modeli (saf)
export type RowSecurityNote = "enableDestructive" | "disableWarning";
/** Karar 8; create (`original === null`) → []. Her bayrak için ayrı değerlendirilir; tekrar eden not bir kez. */
export function rowSecurityNotes(original: TableDef | null, d: TableDef): RowSecurityNote[];

// designer-model.ts (saf)
export interface PolicyTabProbe { tabId: string; serverId: string; databaseOid: number | null; kind: ObjectKind; variant: DesignerVariant; original: DesignerDef | null }
/** Karar 9: tablo alter'ı sonrası yenilenecek politika sekmeleri (tetik koşulu sağlanmıyorsa []). */
export function policyTabsForTable(
  tabs: readonly PolicyTabProbe[],
  serverId: string,
  databaseOid: number | null,
  before: TableDef,
  after: TableDef,
): string[];
```

- `policyTabsForTable`: tetik = `before.rowSecurity.enabled/forced` farklı **ya da** `before.name !== after.name` **ya da** `before.schema !== after.schema`; değilse `[]`. Eşleşme: `variant === "object"`, `kind === "policy"`, aynı `serverId` ve `databaseOid`, `original` bir `PolicyDef` ve `original.schema === before.schema && original.table === before.name`.
- Denetleyici `applied`: `variant === "object"`, `m.kind === "table"`, `mode === "alter"` ve `before` varsa `policyTabsForTable(...)` → her sekme için: kirli değil ve uygulamada değilse `void reload(id)`; aksi hâlde `needsReload: true` yaması. Uygulanan sekmenin kendi akışı değişmez; hata yutulur (uygulama sonucunu etkilemez).
- **Form:** `TableGeneral.tsx`'e "Satır güvenliği (RLS)" bölümü (`fieldset`/`legend`): `CheckRow` "Satır güvenliğini etkinleştir (ENABLE ROW LEVEL SECURITY)" ve "Sahibine de uygula (FORCE ROW LEVEL SECURITY)" + Karar 8 ipucu ve notlar (`rowSecurityNotes`; `enableDestructive` uyarı rengiyle). Taslakta `rowSecurity` yoksa (eski yükleme — serde varsayılanı vardır, olmamalı) `{ enabled: false, forced: false }` varsayılır.

- [ ] **Adım 1: Başarısız testler.** Tablo modeli testi: `rowSecurityNotes` (create → []; değişmedi → []; enable false→true → `enableDestructive`; forced false→true → `enableDestructive` (bir kez); enabled true→false → `disableWarning`; biri açılıp biri kapanınca iki not). `designer-model.test.ts`: `policyTabsForTable` (RLS değişimi eşleşen sekmeyi döndürür; farklı tablo/şema/sunucu/veritabanı/tür/varyant dönmez; ad değişimi eski adla eşleşir; yalnız yorum değişimi → []). `designer-controller.test.ts` (Solid mock satırıyla): tablo alter (RLS değişimi) uygulanınca aynı tablonun kirli olmayan politika sekmesi `ddl_load`'u yeniden çağırır; kirli politika sekmesi yeniden yüklenmez ve `needsReload = true`; başka tablonun politika sekmesine dokunulmaz; RLS değişmeyen tablo alter'ı hiçbir politika sekmesini yüklemez.
- [ ] **Adım 2: Başarısızlığı gör.**
- [ ] **Adım 3: Uygula.**
- [ ] **Adım 4: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (RLS metinleri ana pakette yok; ana paket değişmedi — denetleyici tembel parçadır).
- [ ] **Adım 5: Commit** (`feat(designer): tablo satır güvenliği (RLS) bayrakları; tablo değişince politika sekmeleri yenilenir`).

---

### Task 5: Elle kontrol listesi, spec durumu ve bütünleşik doğrulama

**Dosyalar:** Değiştir `docs/smoke-test.md`, `docs/superpowers/specs/2026-11-19-object-dialogs-remaining-design.md`.

- [ ] **Adım 1: `docs/smoke-test.md`** — Plan 34 bölümündeki "GUI maddeleri Plan 35'te" cümlesini "Plan 35 bölümündedir" olarak güncelle; ardına **"## Nesne diyalogları: Policy / Tablespace arayüzü (Plan 35)"** (ön koşul: Docker PG 10, 13 ve 15+; süper kullanıcı ve süper kullanıcı olmayan iki bağlantı; konteynerde `postgres`'e ait boş bir dizin (`docker exec … mkdir -p … && chown postgres …`); politikası olan ve olmayan tablolar; üretim etiketli sunucu). Maddeler (hepsi `- [ ] … **[—]**`): menü (Policies koleksiyonunda Oluştur, politika düğümünde Düzenle/Yeniden adlandır/Sil, Yetkiler yok; Tablespaces koleksiyonunda Oluştur, tablespace düğümünde Düzenle/Yeniden adlandır/Yetkiler/Sil, Varsayılan yetkiler yok; kilitli sunucuda kapalı); politika oluştur (komut değişince USING/WITH CHECK kapanır ve temizlenir; RESTRICTIVE; roller ekle/kaldır, boş = PUBLIC; ifadesiz politika önizleme uyarısı; RLS kapalı tabloda uyarı); politika düzenle (yalnız ifade/rol → Güvenli tek ALTER; komut/tür değişimi ya da ifade kaldırma → yeniden kurma notu + Yıkıcı onay; üretimde ad yazma **istenmez**; yeniden adlandırma sonrası sekme başlığı ve ağaç); politika sil (Yıkıcı onay, CASCADE kutusu yok); hatalı ifade → alan hatası `USING` altında; tablo diyaloğunda RLS aç/zorla (Yıkıcı notu) ve kapat (uyarı); açık politika sekmesi tablo RLS değişince yenilenir, kirliyse "yeniden yükle" ipucu; tablespace oluştur (süper kullanıcıyla başarı + yorum; süper kullanıcı olmayanla 42501 ipucu; olmayan dizin → konum ipucu; kullanılan dizin → 55006 ipucu; `pg_` öneki alan hatası; göreli yol alan hatası); tablespace düzenle (ad, sahip, seçenek SET/RESET, yorum; konum salt okunur; PG 10–12'de `maintenance_io_concurrency` alan hatası; biçim uyarıları gönderimi engellemez); `pg_default`/`pg_global` salt okunur gerekçeli, Yeniden adlandır/Sil açık ret; boş olmayan tablespace silme → 55000 ipucu, hiçbir şey değişmez; tablespace yetkileri (CREATE ver/geri al; `pg_global` salt okunur); tablespace hata toast'unda konum görünmez (yalnız sekmedeki hata alanında PG iletisi); klavye/erişilebilirlik/tema (çip kaldırma klavyeyle, radio grupları, açık/koyu tema, dar pencere ≈1100 px); "Ölçümler" altına son paket ölçümü.
- [ ] **Adım 2: Spec** — durum satırı "Plan 34–35 uygulandı (Grup A)"; §9'un ardına kısa **"§10 Plan 35 notları"**: telafi kaydı ve ölçümler, komut kuralı aynalaması (temizleme), yeniden kurma notu (seçim kutusu yok), politika sekmesi yenileme kuralı, biçim uyarıları engellemez, backlog (politika ifadesi için CodeMirror, şema karşılaştırmada politikalar).
- [ ] **Adım 3: Bütünleşik doğrulama.** `bun run test`, `bun run typecheck`, `bun run build`, `bun run bundle:check`; `cargo test --workspace` (Docker'sız; bindings gürültüsü geri alınır), `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Adım 4: Kapanış grep'leri:** `console\.` / `localStorage|sessionStorage` / `innerHTML` `src/features/designer` → yeni dosyalarda boş; `strings-(policy|tablespace)` yalnız ilgili form/model; `formMissing` → 0; `qtMore` `src/ipc/commands.ts`'te yok.
- [ ] **Adım 5: Commit** (`docs(designer): Policy/Tablespace arayüzü elle kontrol listesi, spec durumu; bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 35 bitişi)

- İki tür için tam akış: menü → açma → düzenleme → önizleme → kapılı uygulama → başlık + ağaç yenileme; tablespace yetkileri; tablo RLS bayrakları ve politika sekmesi yenileme; salt okunur nesneler gerekçeli.
- `bundle:check` geçer; tavanlar yalnız düşmüş olabilir; yeni kod/metin yalnız tembel parçalarda.
- Elle kontrol maddeleri `[—]`; final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla** ("main'e yerel birleştir").

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §2.1 Explorer/alanlar/doğrulama/düzenleme/RLS durumu/tablo ekleri/silme/salt okunur/kimlik/roller → Task 1, 2, 4; §2.2 Explorer/alanlar/doğrulama/salt okunur/izinler/ACL → Task 1, 3; §2.3 risk → mevcut kapı (Karar 10); §5 tembel formlar, serbest SQL `textarea`, ana paket telafisi → Task 1–4; §6 güvenlik (günlüğe yazmama, konum toast'ta değil) → Global Constraints + smoke; §7 ön yüz testleri → her görevin Adım 1'i; §9.1 (`editable`/`unsupported`/`tableRls` korunur) → Global Constraints + Review Focus; §9.2 yollar → Task 2–3 `*_PATHS`; §9.3 kurallar/yenileme → Karar 3, 9, Task 4; §9.4 hedefler → mevcut `designer-open` (sunucu kapsamı `isServerScoped` Plan 34'te); §9.5 ipuçları → mevcut hata şeridi (yeni metin yok); §9.7 bütçe → Task 1.
- **Yer tutucu taraması:** "TBD/TODO" yok; geçici `formMissing` koruması Task 3'te kalkar.
- **Tip tutarlılığı:** `PolicyDef`/`PolicyCommand`/`RowSecurity`/`TablespaceDef`/`TablespaceOptions` (Plan 34 bindings) → Task 2–4; `policyTabsForTable` (Task 4, `designer-model.ts`) → denetleyici (Task 4); `POLICY_PATHS`/`TABLESPACE_PATHS` → `FORM_PATHS` (Task 2–3).
