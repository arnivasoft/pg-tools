# Nesne Diyalogları — View / Materialized View / Database / Role Arayüzü (Plan 15) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Plan 14 arka ucunun üstüne dört türün **arayüzü**: Explorer'dan **View**, **Materialized View**, **Database**, **Role** için **Oluştur… / Düzenle… / Yeniden adlandır… / Sil…**; tembel formlar (`ViewForm` — view + MV tek parça, `DatabaseForm`, `RoleForm`, ortak `SettingsEditor`), sunucu kapsamlı sekmeler (`databaseOid: null`), **yalnızca yazılır parola** alanı (değer önizlemeye hiç gitmez, yalnızca uygulamada `ApplyOptions.password` ile), gizlenmiş ifadeli planda "Query Tool'da aç"ın kapatılması, silme diyaloğunda veritabanı için **WITH (FORCE)** seçeneği ve elle kontrol listesi.

**Bağlantı:** Bu plan **Plan 14'ün** (`docs/superpowers/plans/2026-10-07-object-dialogs-p14-view-database-role.md`) `main`'e birleşmesinden sonra başlar ve onun "Plan 15 için notlar (sözleşme)" bölümünü bağlayıcı sayar. Plan 14 arka ucu, bindingleri, `formMissing` geçici korumasını ve tembel derleme uyumunu (`password: null`, `force: false`, `redacted: false` fikstürleri) bırakmıştır; bu plan korumayı kaldırır ve dört formu ekler. Rust değişikliği **yoktur** (sözleşme boşluğu bulunursa düzeltilmez, raporlanır).

**Mimari:** Plan 11/13 kalıbı aynen: istemci SQL üretmez, `ObjectDef` düzenler; tüm mantık saf modüllerde (`view-model.ts`, `database-model.ts`, `role-model.ts`, `settings-model.ts`), Solid bileşenleri ince. Her formun metni kendi tembel modülündedir (`strings-view.ts`, `strings-database.ts`, `strings-role.ts`, `strings-settings.ts`; yalnızca ilgili form + modeli içe aktarır). Parola değeri denetleyicide **tepkisel olmayan, kalıcı olmayan** bir haritada tutulur (`secrets: Map<tabId, string>`), modelde yalnızca `secretReady: boolean` görünür; `defsEqual`/kirli bayrağı/önizleme isteği/günlük/`localStorage` hiçbir zaman değeri görmez. Ana pakete yalnızca menü görünürlüğü kancası girer (Task 1) ve **aynı görevde adı konmuş azaltmayla** telafi edilir.

**Teknoloji:** SolidJS, Tailwind 4, vitest (env `node`), mevcut designer çerçevesi (`designer-controller`, `SqlPreview`, `ApplyBar`, `FormParts`, `DropDialog`).

**Spec:** `docs/superpowers/specs/2026-10-06-object-dialogs-design.md` (§3 risk modeli, §4 arayüz, §5 View/MatView/Database/Role, §6 gizlilik; "View/MatView/Database/Role arka ucu (Plan 14) sözleşme notları"). Önceki arayüz planları: `2026-10-06-object-dialogs-p11-on-yuz.md`, `2026-10-07-object-dialogs-p13-tablo-on-yuz.md` — burada tekrarlanmayan tüm Global Constraints ve Review Focus maddeleri geçerlidir. Bağlamalar: `src/ipc/bindings/{ViewDef,ViewColumn,CheckOption,MatViewDef,MatViewRefresh,DatabaseDef,LocaleProvider,ConfigSetting,RoleDef,RoleMembership,PasswordChange,Statement,ApplyOptions,PlanRequest,DdlTarget,LookupRequest,LookupItem,ObjectDef,ObjectKind,DdlErrorCode}.ts`.

## Kararlar

1. **Sorgu düzenleyicisi `textarea`'dır (CodeMirror değil):** spec §5 "sorgu gövdesi (SQL editörü)" der. CodeMirror'u tembel formdan içe aktarmak `SqlPreview`/QueryTab parçalarıyla paylaşılan kodu yeniden böler ve ana paketin `__vite__mapDeps` listesini büyütme riski taşır (Plan 13 dersi). Tek satır aralıklı, `font-mono`, `spellcheck=false`, `wrap="off"`, yatay kaydırmalı `textarea` + `FreeSqlNote` + `E'…'` ipucu kullanılır; CodeMirror'lu düzenleyici backlog.
2. **Parola alanı hiçbir zaman gösterilmez:** iki `type="password"` girdi (yeni + tekrar, `autocomplete="new-password"`), göster/gizle düğmesi yok (spec: "asla gösterilmez"; ayrıca `Eye`/`EyeOff` ana paketle paylaşılan simgeler). Eşleşmeyen/boş parola uygulama kapısını yerel olarak kapatır (`secretMissing`), önizleme serbesttir (önizleme parolayı kullanmaz).
3. **Kopyala/Query Tool:** planda `redacted` ifade varsa ApplyBar'daki "Query Tool'da aç" kapalıdır (gerekçe `title`); SqlPreview'deki kopyalama metni zaten maskeli ve tırnaksızdır (çalıştırılırsa sözdizimi hatası — Plan 14 Karar 6), ek önlem gerekmez.
4. **Görünüm "Yeniden oluştur" yalnızca açık seçimdir:** onay kutusu yalnızca alter'da ve sorgu yüklenenden farklıyken görünür; işaretlenince kaybolacaklar listesi (`unsupported` + "GRANT yetkileri") ve DataLoss notu gösterilir. Uygulama `CREATE OR REPLACE` uyumsuzluk hatasıyla dönerse arka ucun ipucu hata şeridinde görünür (yeni istemci mantığı yok).
5. **Sürüm bağımlı alanlar gizlenmez** (Plan 13 kararı): `security_invoker` (PG 15+), `WITH (FORCE)` (PG 13+), ICU (PG 15+), builtin (PG 17+) ipucuyla sunulur; eski sunucuda arka uç alan hatası gösterilir.

## Global Constraints

- **Paket bütçesi (bağlayıcı):** ana `index-*.js` bu planın başında **tam 233,088 B**'dir (Plan 13 tavanı; pay yok). Plan 15 **yalnızca tembel**dir; tek istisna Task 1'deki menü görünürlüğü kancası (`ddl-kinds.ts` + `actions.ts`) — **en çok +0,5 KB** olabilir ve **aynı görevde** şu adı konmuş azaltmayla telafi edilir: `t.query.editorSettings` metin bloğu (`src/app/strings.ts`; yalnızca QueryTab parçasındaki `EditorSettings.tsx` kullanır) yeni tembel `src/features/query/strings-editor.ts`'e (`tedit`) taşınır. **Kabul ölçütü:** Task 1 sonunda ve sonraki **her** UI görevinde ana `index-*.js` ≤ **233,088 B** (Task 1'de ölçülen değer sonraki görevlerin tavanıdır); `index.html` yalnızca solid/web/Icon parçalarını önyükler; ana `__vite__mapDeps` listesinde **yeni parça adı yok**.
- **Boyut doğrulaması (her UI görevinin son adımı):**
  ```powershell
  bun run build
  (Get-Item dist/assets/index-*.js).Length                       # ≤ Task 1 değeri (≤ 233,088)
  Select-String -Path dist/index.html -Pattern 'modulepreload'   # yalnızca solid / web / Icon
  Select-String -Path dist/assets/index-*.js -SimpleMatch "Yeniden oluştur","Parola (tekrar)","Şablon veritabanı","Satır sonu"   # BOŞ
  Get-ChildItem dist/assets -Filter "*Form-*.js" | Select-Object Name, Length
  ```
  (`"Satır sonu"` `tedit` metinlerinden biridir: Task 1'den itibaren ana pakette olmamalı — uygulayıcı `EditorSettings` bloğundan gerçekten taşınan, başka yerde geçmeyen bir dizgeyi seçip burada kullanır ve plan ledger'ına yazar.)
- **Tembel kural:** ana paket modülleri (`src/app/*` — `strings-*.ts` tembel modüller hariç —, `src/features/explorer/*`, `src/features/query/{QueryTabs,tabs-store,query-controller}.ts(x)`, `src/ipc/commands.ts`, `ddl-kinds.ts`) yeni form/model/metin modüllerini **statik içe aktarmaz**; tip değişiklikleri (`databaseOid: number | null`) yalnızca **tip** düzeyindedir (bayt değişmez — `git diff` ile ana dosyalarda yalnız tip satırları). Tembel formlarda **Kobalte içe aktarılmaz** (yerel `select`/`input`/`radio`/`checkbox`), `preview.ts`'in `createPreviewer`'ı içe aktarılmaz (gerekirse `type-suggestions.ts`'teki debounce kalıbı kopyalanır), lucide simgelerinden yalnızca `Trash2` ve `Undo` kullanılır (`Plus`, `X`, `Pencil`, `Lock`, `RefreshCw`, `Copy`, `Eye`, `EyeOff`, `ChevronDown`, `ChevronRight`, `Undo2`, `Columns3`, `KeyRound` vb. **yasak**).
- **İstemci SQL üretmez:** görünüm/MV sorgusu ve ayar değerleri **olduğu gibi** gönderilir (yalnızca boş → `null` normalleştirmesi isteğe bağlı alanlarda; `query` boş dizge olarak kalır, alan hatası arka uçtan). GUC liste tırnaklaması arka uçtadır; arayüz yalnızca `search_path` vb. için ipucu verir.
- **Parola gizliliği:** değer yalnızca denetleyicinin `secrets` haritasında ve ApplyOptions'ın gönderildiği tek çağrıda bulunur; `DesignerModel`'e, `PlanRequest`'e, `defsEqual`/`cloneDef`'e, sekme kaydına, `localStorage`/`sessionStorage`'a, `console.*`'a, toast/hata metnine, `structuredClone` edilen hiçbir nesneye girmez. Harita girdisi: başarılı uygulamada, `reload`'da, sekme kapanışında, parola niyeti `set` dışına çıkınca ve sunucu bağlantısı kesilince **silinir**. Girdi alanları `autocomplete="new-password"`, `spellcheck=false`.
- **Sözleşme (Plan 14 notları):** Database/Role sekmelerinde `databaseOid = null` (sekme anahtarı `alter:role:<sunucu>:null:<oid>`); `query`/`columns`/`unsupported`/`inheritOption`/`setOption` istemcide değiştirilmez (salt okunur gösterim); create-only Database alanları alter'da salt okunur; `validUntil` ve tablespace temizlenemez (alter'da yüklü değer varken boş seçenek yok; `infinity`/`pg_default` açık seçimdir); `recreate`/`refresh` alter-only ve yüklemede sıfır; Database/Role için `cascade` gönderilmez (`false`), `force` yalnızca Database.
- **FieldError yolları** Plan 14 "Plan 15 için notlar"daki listedir; satır indeksleri taslaktaki sıradır; `old.` önekliler ve bilinmeyen yollar genel şeride (`groupIssues`).
- **Uygulama güvenliği** Plan 11 ile aynı ve **değişmez** (`applyGate`, onay kutusu, üretimde ad yazma, kilit, tek uçuş, `plan_changed` sonrası otomatik yükleme yok); tek ek: `GateInput.secretMissing` (aşağıda), önceliği `empty`'den sonra, risk'ten önce.
- **Düğüm kimlikleri ayrıştırılmaz:** hedef `ddl_target_of`; ek kararlar ağaç yapısı/türü/bayraklarıyla (`flags.system`).
- **Tepkisellik dersleri:** denetleyicide okunan her yeni `Map` (ör. `secretReady` için) mevcut `registry` sinyaline bağlanır; Solid sinyali kullanan her test dosyası başında `vi.mock("solid-js", async () => await import("solid-js/dist/solid.js" as string));`; saf model testleri Solid içe aktarmaz.
- **Gizlilik/erişilebilirlik/tema:** SQL/tanım/yorum/parola `console.*`'a ve depolamaya yazılmaz; arayüz metinleri Türkçe, PostgreSQL terimleri İngilizce; renkler yalnızca CSS değişkenleri; her alan etiketli, hatalar `aria-describedby`, klavyeyle tam kullanım; seçili/durum rozetleri zemin rengiyle (tek taraflı kalın kenarlık yok).
- Test: `bun run test`, `bun run typecheck`, `bun run build`; `cargo test --workspace` (Docker'sız) yeşil kalır; git worktree **kullanılmaz**; `src/ipc/bindings` satır sonu gürültüsü `git checkout -- src/ipc/bindings`. Ajanlar GUI'yi elle denemez: elle kontrol maddeleri `docs/smoke-test.md`'ye `[—]` olarak yazılır. Commit eki: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

## Review Focus

- **Paket bütçesi:** her görevde ana `index-*.js` ≤ Task 1 değeri; yeni metinler ana pakette yok; `__vite__mapDeps` yeni parça adı içermez; `tedit` taşınması davranışı değiştirmez.
- **Parola:** değer yalnızca `secrets` haritasında ve tek `ddlApply` çağrısının `options.password` alanında; kirli bayrağı/önizleme isteği/model/depolama/günlükte yok; silinme anları (başarı, reload, kapanış, niyet değişimi, bağlantı kaybı) testli; eşleşmeyen parola uygulamayı kapatır ama önizlemeyi değil; `role_password_missing` kodu anlaşılır metne eşlenir.
- **Sunucu kapsamı:** Database/Role açılışı `databaseOid = null` ile; sekme başlığında/`where`'de veritabanı adı yok; Query Tool'da aç bakım veritabanına; veritabanı düğümünden açılan Database sekmesi bile `null` kullanır.
- **Sözleşme sınırları:** create-only Database alanları kilitli; `recreate` yalnızca sorgu değişmişken; `refresh` seçenekleri `withData` tablosuna uyar; üyelik seçenekleri (PG 16+) salt okunur; `validUntil`/tablespace temizlenemez; sistem rolleri menü almaz.
- **Silme:** Database için FORCE kutusu (CASCADE yok), Role için ne CASCADE ne FORCE; DROP DATABASE DataLoss → üretimde ad yazma; 2BP01 (rol nesne sahibi / bağımlı görünüm) ayrıntısı gösterilir; 55006 ipucu gösterilir.

---

## Dosya Yapısı

| Dosya | Sorumluluk |
|---|---|
| `src/features/designer/ddl-kinds.ts`, `src/features/explorer/actions.ts` (+ `actions.test.ts`) | **Ana paket:** dört tür + koleksiyonları; sistem rolü hariç |
| `src/app/strings.ts`, `src/features/query/strings-editor.ts` (yeni), `src/features/query/EditorSettings.tsx` | **Ana paket telafisi** |
| `src/features/designer/designer-model.ts`, `designer-controller.ts`, `designer-open.ts`, `drop-model.ts`, `drop-controller.ts`, `DropDialog.tsx`, `ApplyBar.tsx`, `src/app/strings-designer.ts` (+ testler) | Sunucu kapsamı, parola haritası ve kapısı, gizlenmiş plan, FORCE |
| `src/features/query/tabs-store.ts`, `src/ipc/commands.ts` | Yalnızca tip: `databaseOid: number | null` |
| `src/features/designer/{view-model,database-model,role-model,settings-model}.ts` (+ testler) | Saf modeller |
| `src/app/{strings-view,strings-database,strings-role,strings-settings}.ts` | Tembel metin modülleri |
| `src/features/designer/{ViewForm,DatabaseForm,RoleForm,SettingsEditor}.tsx`, `DesignerView.tsx`, `forms-model.ts` | Formlar ve kayıt (`FORMS` tam `Record`, `formMissing` kaldırılır) |
| `docs/smoke-test.md`, spec | Elle kontrol listesi, durum |

---

### Task 1: Ana paket — menü kancası ve adı konmuş telafi

**Dosyalar:**
- Değiştir: `src/features/designer/ddl-kinds.ts`, `src/features/explorer/actions.ts`, `src/features/explorer/actions.test.ts`, `src/app/strings.ts`, `src/features/query/EditorSettings.tsx`
- Oluştur: `src/features/query/strings-editor.ts`

**Arayüzler:**
- Tüketir: `NodeKind`, `TreeNode.flags.system`, mevcut `actionsFor`.
- Üretir (`ddl-kinds.ts`, bağımlılıksız kalır):

```ts
export const DDL_OBJECT_KINDS: NodeKind[] = [
  "schema", "sequence", "extension", "index", "table", "view", "materializedView", "database", "role",
];
/** …Collection: schema, sequence, extension, index, table, view, materializedView, database, role. */
export const DDL_CREATE_COLLECTIONS = DDL_OBJECT_KINDS.map((k) => `${k}Collection` as NodeKind);
```

- Üretir (`actions.ts`, `actionsFor`): `if (canEditObject(node.kind) && !(node.kind === "role" && node.flags.system)) out.push("editObject", "renameObject", "dropObject");` (önceden tanımlı `pg_*` rolleri arka uçta da reddedilir; menü göstermez). `groupOf`/`disabledReason`/`executeAction` değişmez.
- Üretir (`strings-editor.ts`): `export const tedit = { …t.query.editorSettings bloğunun birebir kopyası… } as const;` — `strings.ts`'ten `query.editorSettings` silinir; `EditorSettings.tsx`'te `t.query.editorSettings` → `tedit` (`import { tedit } from "./strings-editor";`).

- [ ] **Adım 1: Başlangıç ölçümü.** `bun run build`; ana `index-*.js` **233088** olmalı (değilse dur ve raporla).
- [ ] **Adım 2: Başarısız testler** (`actions.test.ts`, designer bloğu; `ddlTree()`'ye düğümler: `add("coll-dbs", srv, "databaseCollection", "Databases"); add("db-app", "coll-dbs", "database", "app"); add("coll-roles", srv, "roleCollection", "Login/Group Roles"); add("role-app", "coll-roles", "role", "app"); add("role-sys", "coll-roles", "role", "pg_monitor", { flags: { system: true } }); add("coll-views", "sch", "viewCollection", "Views"); add("view-1", "coll-views", "view", "v"); add("coll-mvs", "sch", "materializedViewCollection", "Materialized Views"); add("mv-1", "coll-mvs", "materializedView", "m");`):

```ts
it("view/MV/database/role koleksiyonlarında createObject", () => {
  for (const id of ["coll-dbs", "coll-roles", "coll-views", "coll-mvs"]) expect(actionsFor(s, node(s, id))).toContain("createObject");
});
it("view, MV, database ve kullanıcı rolü düğümünde edit/rename/drop", () => {
  for (const id of ["view-1", "mv-1", "db-app", "role-app"]) {
    expect(actionsFor(s, node(s, id)).filter((x) => DES.includes(x))).toEqual(["editObject", "renameObject", "dropObject"]);
  }
});
it("sistem rolü (pg_*) ve görünüm kolonu designer eylemi almaz", () => {
  for (const id of ["role-sys", "vcol-1"]) expect(actionsFor(s, node(s, id)).filter((x) => DES.includes(x))).toEqual([]);
});
```

  Mevcut "diğer düğümlerde designer eylemi yok" testinden artık eylem alan düğümler (database, role, view) çıkarılır; tablespace düğümleri listede kalır. `disabledReason`: kilitli sunucuda `db-app` + `dropObject` → gerekçe.
- [ ] **Adım 3:** `bun run test src/features/explorer` — başarısızlığı gör. **Adım 4: Uygula** (`ddl-kinds.ts`, `actions.ts`), ardından telafi (`strings-editor.ts`, `strings.ts`, `EditorSettings.tsx`); `rg "t\.query\.editorSettings" src` → 0.
- [ ] **Adım 5: Doğrulama.** `bun run test`, `bun run typecheck`, **Boyut doğrulaması**: ana ≤ 233,088 B; ölçülen değeri commit mesajına ve `docs/smoke-test.md` "Ölçümler" altına yaz ("Plan 15 Task 1: index-*.js = N B"). Aşıyorsa **dur ve raporla** (başka azaltma bu planda kararlaştırılmadı).
- [ ] **Adım 6: Commit** (`feat(explorer): View/MatView/Database/Role menü kancası; editör ayarı metinleri tembel parçaya (ana paket telafisi, index N B)`).

  Not: Task 2–6 boyunca dal içinde yeni türler `formMissing` şeridini gösterir; dal `main`'e yalnızca Task 7 sonrası birleşir.

---

### Task 2: Designer çekirdeği — sunucu kapsamı, parola haritası ve kapısı, gizlenmiş plan, FORCE

**Dosyalar:**
- Değiştir: `src/features/designer/designer-model.ts` (+ test), `designer-controller.ts` (+ `designer-controller.test.ts`, `designer-reactivity.test.ts`), `designer-open.ts` (+ test), `drop-model.ts` (+ test), `drop-controller.ts` (+ test), `DropDialog.tsx`, `ApplyBar.tsx`, `src/app/strings-designer.ts`, `src/ipc/errors.ts` gerekirse (yalnızca tembel `errorText` eşlemesi designer'daysa orada), `src/features/query/tabs-store.ts` ve `src/ipc/commands.ts` (**yalnız tip**)

**Arayüzler:**
- Tüketir: `ObjectKind`, `Statement.redacted`, `ApplyOptions.password`, `PlanRequest.drop.force`, `DdlErrorCode` (`role_password_missing`).
- Üretir (`designer-model.ts`):

```ts
/** Plan 14: sunucu kapsamlı türler (bakım veritabanında; databaseOid her zaman null). */
export function isServerScoped(kind: ObjectKind): boolean { return kind === "database" || kind === "role"; }
/** Planda gizlenmiş (parola) ifade var mı → Query Tool'a aktarılmaz, parola gerekir. */
export function planHasRedacted(view: PlanView | null): boolean { return !!view && view.statements.some((s) => s.redacted); }
// GateInput'a: secretMissing: boolean   (planHasRedacted(view) && !secretReady)
// ApplyGate blocked reason'a: "password"
// applyGate önceliği: … > invalid > empty > password > risk
export function applyOptionsFor(applyId: string, lockTimeoutMs: number, view: PlanView, gate: ApplyGate, c: ConfirmState, password: string | null = null): ApplyOptions | null;
//   → { …, password: planHasRedacted(view) ? password : null }  (gizlenmiş ifade yoksa parola ASLA gönderilmez)
export type DesignerTarget = { serverId: string; databaseOid: number | null } & (/* aynı */);
```

- Üretir (`designer-controller.ts`):
  - `databaseOid: number | null` (model, `OpenCreateInput`, `OpenAlterInput`, `lookupKey`, `objectDropped`, `findAlter`).
  - Parola: `const secrets = new Map<string, string>()` (tepkisel değil); `setSecret(tabId: string, value: string | null): void` (null/boş → sil; her çağrıda `registry` sinyali dürtülür ki `secretReady` tepkisel olsun), `secretReady(tabId): boolean` (registry'ye bağlı okuma). `apply()`: `applyOptionsFor(…, secrets.get(tabId) ?? null)`; **başarılı** sonuçta, `reload`'da, `close`'da, `serverSessionsClosed`'da ve taslak `password` alanı `"set"` dışına çıkan `edit`'te girdi silinir. Gate girdisi `secretMissing: planHasRedacted(m.view) && !secretReady(tabId)`. `SESSION_LOOKUPS`'a `"grantable_roles"`, `"encodings"`, `"template_databases"`, `"locales"`, `"settings"` eklenir (anahtar isteğin tamamını içerdiğinden `locales{provider}` ayrı tutulur).
  - Hata metni: `role_password_missing` kodu → `tdes.gate.passwordMissing` (gateError yolu; mevcut kod eşleme tablosuna bir satır).
- Üretir (`designer-open.ts`): `kind = create?.kind ?? object?.kind`; `if ((!create && !object) || (databaseOid === null && !isServerScoped(kind)))` → `tdes.open.none`; sunucu kapsamlıda `databaseOid` **her zaman `null`** gönderilir (hedef zaten null döner; savunma) ve `database = null` (`where` = sunucu adı; başlık `tdes.tab.create(kindLabel, config.name, null)`); `dbNames` yalnız veritabanı kapsamlıda; `openDrop({ serverId, databaseOid: null, … })`.
- Üretir (`drop-model.ts` / `drop-controller.ts` / `DropDialog.tsx`): `dropRequest(object, def, cascade, force)`; `DropState.force: boolean` + `setForce(v)` (cascade kalıbı: sürüm ilerler, onay sıfırlanır, önizleme yeniden); `dropOptionsFor(kind): { cascade: boolean; force: boolean }` (database → `{cascade:false, force:true}`, role → `{false,false}`, diğerleri `{true,false}`) — diyalog yalnızca sunulan kutuları çizer; FORCE metni `tdes.drop.force` = "Bağlı oturumları sonlandır (WITH (FORCE), PG 13+)", ipucu `tdes.drop.forceHint` = "Query Tool sekmeleri dahil tüm oturumlar kesilir."; 55006 hatasında arka ucun ipucu gösterilir (mevcut "other" durumu `hint`'i de yazar — gerekirse `dropStatusText` hint'i ekleyecek şekilde genişletilir, testli).
- Üretir (`ApplyBar.tsx`): `planHasRedacted(view)` iken "Query Tool'da aç" `disabled` + `title={tdes.bar.openScriptRedacted}` ("Parola içeren plan Query Tool'a aktarılamaz."); `gate.reason === "password"` iken uygulama düğmesi gerekçesi `tdes.gate.password` ("Parolayı iki alana aynı girin.").
- `strings-designer.ts`'e yalnızca: `gate.password`, `gate.passwordMissing`, `bar.openScriptRedacted`, `drop.force`, `drop.forceHint`; `view.formMissing` **bu görevde kalır** (Task 4–6 kaldırır).

- [ ] **Adım 1: Başarısız testler.**
  - `designer-model.test.ts`: `isServerScoped` tablosu; `planHasRedacted`; `applyGate` önceliği (`secretMissing` + `empty` → empty; `secretMissing` + destructive → `password`); `applyOptionsFor` — gizlenmiş ifadeli planda parola geçer, gizlenmemişte verilen parola **null'a çekilir**; `designerTabKey` `databaseOid: null` → `alter:role:s1:null:16384`.
  - `designer-controller.test.ts` (mevcut sahte `Api` kalıbı):

```ts
it("parola yalnızca ddlApply options.password'da gider; ddlPreview isteğinde ve modelde yok", async () => {
  // role create taslağı password="set"; setSecret(tab, "Gizli-1"); önizleme + uygulama
  // expect(JSON.stringify(api.ddlPreview.mock.calls)).not.toContain("Gizli-1");
  // expect(api.ddlApply.mock.calls[0][3].password).toBe("Gizli-1");
  // expect(JSON.stringify(ctl.model(tab))).not.toContain("Gizli-1");
});
it("başarılı uygulama, reload, close ve password≠set düzenlemesi parolayı siler; secretReady false olur", async () => {});
it("gizlenmiş planda parola yokken gate 'password'; parola girilince needsConfirm/ready; önizleme her durumda istenir", async () => {});
it("role_password_missing hata kodu gateError metnine eşlenir ve uygulama yeniden denenebilir", async () => {});
it("sunucu kapsamlı sekme: ddlLoad/ddlPreview/ddlApply/ddlLookup databaseOid=null ile çağrılır", async () => {});
it("SESSION_LOOKUPS: grantable_roles/encodings/template_databases/locales/settings TTL sonrası yeniden istenmez", async () => {});
```

  - `designer-reactivity.test.ts`: `secretReady` `setSecret` sonrası memo'da güncellenir.
  - `designer-open.test.ts`: rol düğümünden edit → `openDesigner({ databaseOid: null, where: "<sunucu>" })`, başlıkta veritabanı adı yok; veritabanı düğümünden Database edit → `databaseOid: null`; `databaseOid === null` + view hedefi → `tdes.open.none`; drop → `openDrop({ databaseOid: null })`.
  - `drop-model.test.ts` / `drop-controller.test.ts`: `dropRequest(…, force)` şekli; `dropOptionsFor` tablosu; `setForce` sürüm/onay/önizleme davranışı (cascade testinin kopyası); 55006 + ipucu durum metni.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula. `tabs-store.ts` (`DesignerTab.databaseOid`, `OpenDesignerInput.databaseOid`) ve `commands.ts` (`ddl*` imzaları) yalnızca tip olarak `number | null`; `git diff -U0 -- src/features/query/tabs-store.ts src/ipc/commands.ts` yalnız tip satırları gösterir.
- [ ] **Adım 4:** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (ana ≤ Task 1 değeri; değişmemesi beklenir).
- [ ] **Adım 5: Commit** (`feat(designer): sunucu kapsamlı sekmeler, yalnızca uygulamada giden parola ve kapısı, gizlenmiş planda Query Tool kapalı, DROP DATABASE FORCE`).

---

### Task 3: View/MatView saf modeli, metin modülü ve formu

**Dosyalar:**
- Oluştur: `src/app/strings-view.ts`, `src/features/designer/view-model.ts` (+ `view-model.test.ts`), `src/features/designer/ViewForm.tsx`
- Değiştir: `src/features/designer/forms-model.ts` (+ test: `VIEW_PATHS`, `MATVIEW_PATHS`), `DesignerView.tsx` (`view`, `materialized_view` → aynı `lazy(() => import("./ViewForm"))`)

**Arayüzler:**
- Tüketir: `ViewDef`, `MatViewDef`, `CheckOption`, `MatViewRefresh`, `FormProps`, `FieldRow`, `OwnerSelect`, `CommentField`, `FreeSqlNote`, `createFocusOnRequest`, `tablespaceChoices`, `normalizeTablespaceEdit`, `fillfactorFromInput`.
- Üretir (`forms-model.ts`):

```ts
export const VIEW_PATHS: ReadonlySet<string> = new Set(["schema", "name", "owner", "query", "checkOption", "securityBarrier", "securityInvoker", "comment", "recreate"]);
export const MATVIEW_PATHS: ReadonlySet<string> = new Set(["schema", "name", "owner", "query", "tablespace", "fillfactor", "withData", "refresh", "comment"]);
```

- Üretir (`view-model.ts`, saf, değişmez):

```ts
export type ViewDraft = ViewDef & { kind: "view" };
export type MatViewDraft = MatViewDef & { kind: "materialized_view" };
export function asView(d: ObjectDef | null): ViewDraft | null;
export function asMatView(d: ObjectDef | null): MatViewDraft | null;
export function viewEditPatch(d: ObjectDef, patch: Partial<ViewDef>): ObjectDef;        // tür uyuşmazsa d
export function matviewEditPatch(d: ObjectDef, patch: Partial<MatViewDef>): ObjectDef;
/** Alter: sorgu yüklenenden farklı mı (metin eşitliği; boşluk dahil — arka uç da metin karşılaştırır). */
export function queryChanged(original: ViewDef | MatViewDef | null, d: ViewDef | MatViewDef): boolean;
/** "Yeniden oluştur" kutusu görünür mü (yalnız alter + view + sorgu değişmiş). Sorgu özgüne dönünce recreate false'a çekilir. */
export function recreateVisible(mode: "create" | "alter", original: ViewDef | null, d: ViewDef): boolean;
export function withQuery<T extends ViewDef | MatViewDef>(d: T, original: T | null, query: string): T;   // view: sorgu özgüne dönerse recreate=false
/** Yeniden kurmada kaybolacaklar (yüklenen `unsupported` + her zaman "GRANT yetkileri"); create'te []. */
export function lossList(original: ViewDef | MatViewDef | null): string[];
/** MV yenileme seçenekleri (Plan 14 Task 3 tablosu): sunulan `refresh` değerleri. */
export function refreshChoices(original: MatViewDef | null, d: MatViewDef): Array<MatViewRefresh | null>;
//   create → [null]; tanım değişti → [null]; old=false,new=true → [null, "normal"]; old=true,new=false → [null];
//   true,true → [null, "normal", "concurrently"]; false,false → [null]
/** withData değişimi: refresh yeni seçeneklerde yoksa null'a çekilir. */
export function withDataPatch(original: MatViewDef | null, d: MatViewDef, withData: boolean): MatViewDef;
/** MV tanım değişimi (sorgu) → "yeniden kurulur" uyarısı ve refresh kilitli (null). */
export function matviewRebuild(original: MatViewDef | null, d: MatViewDef): boolean;
export function checkOptionFromSelect(v: string): CheckOption | null;   // "" → null, "local"/"cascaded"
```

- Üretir (`strings-view.ts`, `tview`): bölüm başlıkları (Genel, Sorgu, Seçenekler/Depolama), alan etiketleri ve ipuçları — şema/ad (alter'da "değişirse yeniden adlandırılır"), sorgu ("SELECT … sorgusu; sonda `;` olmadan"), sorgu ipucu (`E'…'`), C-O-R notu ("Mevcut kolonlar aynı ad, tip ve sırada kalmalı; yeni kolonlar yalnız sona eklenebilir"), "Kolonlar (salt okunur)" başlığı, `check_option` seçenekleri ("Yok", "LOCAL", "CASCADED"), `security_barrier`, `security_invoker` ("PG 15+"), **"Yeniden oluştur (DROP + CREATE)"** + not ("Görünüm silinip yeniden oluşturulur; aşağıdakiler kaybolur ve veri kaybı onayı gerekir"), MV: tablespace (`pg_default` kuralı), fillfactor, "Veriyle doldur (WITH DATA)", yenileme seçenekleri ("Yenileme yok", "Şimdi yenile (REFRESH)", "Okumaları kesmeden yenile (CONCURRENTLY — benzersiz indeks gerekir)"), WITH NO DATA notu, "Tanım değişti: MV silinip yeniden kurulur; indeksler ve yetkiler kaybolur", `lossTitle` ("Kaybolacaklar"), `grants` ("GRANT yetkileri").
- Üretir (`ViewForm.tsx`, varsayılan dışa aktarım; `model.kind`'e göre view ya da MV): **Genel** (Şema — create'te yazılabilir + `schemas` datalist, alter'da yazılabilir (SET SCHEMA); Ad + `createFocusOnRequest("name")`; Sahip; Yorum), **Sorgu** (`textarea` Karar 1, `aria-describedby` hata + `FreeSqlNote` + kaçış ipucu; alter'da sorgu değişmişse view için C-O-R notu, MV için yeniden kurma uyarısı), **Kolonlar** (alter'da `columns` salt okunur liste: ad + tip; boşsa gizli), **Seçenekler** (view: check_option `select`, iki onay kutusu, `recreateVisible` iken "Yeniden oluştur" kutusu + `lossList` listesi `role="note"`), **Depolama** (MV: tablespace `select` (boş yalnızca `original?.tablespace == null`), fillfactor `number`, "Veriyle doldur" kutusu (`withDataPatch`), yenileme `select` (`refreshChoices`; tek seçenekse gizli), `matviewRebuild` iken uyarı + `lossList`). Bölümler basit başlıklı `section`'lardır (sekme çubuğu yok). Tüm denetimler `readOnly` iken `disabled`.

- [ ] **Adım 1: Başarısız testler** (`view-model.test.ts`; Solid içe aktarmaz):

```ts
it("asView/asMatView tür denetimi; edit patch tür uyuşmazlığında aynı nesne", () => {});
it("queryChanged: create (original null) → false; metin farkı → true; aynı metin → false", () => {});
it("recreateVisible: yalnız alter + view + sorgu değişmiş; withQuery sorgu özgüne dönünce recreate=false", () => {});
it("lossList: unsupported + 'GRANT yetkileri'; create → []", () => {});
it("refreshChoices: Plan 14 tablosunun her satırı; tanım değişince yalnız [null]", () => {});
it("withDataPatch: refresh yeni seçeneklerde yoksa null'a çekilir (true,true,concurrently → false: null)", () => {});
it("matviewRebuild: yalnız alter + sorgu farkı", () => {});
it("checkOptionFromSelect: '' → null; local/cascaded", () => {});
```

  `forms-model.test.ts`: `FORM_PATHS.view === VIEW_PATHS`, `FORM_PATHS.materialized_view === MATVIEW_PATHS`; `groupIssues([{path:"query"},{path:"old.name"},{path:"columns"}], VIEW_PATHS)` → `query` alan, diğerleri genel.
- [ ] **Adım 2–3:** Başarısızlığı gör; uygula (model + form + `FORMS` iki girdi + `FORM_PATHS` iki girdi).
- [ ] **Adım 4:** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (ana ≤ Task 1; `ViewForm-*.js` ayrı parça; `"Yeniden oluştur"` ana pakette yok; `rg "strings-view" src` yalnız `ViewForm.tsx`, `view-model.ts`).
- [ ] **Adım 5: Commit** (`feat(designer): View/Materialized View formu (sorgu, seçenekler, açık yeniden oluşturma, REFRESH) ve saf model`).

---

### Task 4: Ortak ayar düzenleyicisi ve Database formu

**Dosyalar:**
- Oluştur: `src/app/strings-settings.ts`, `src/app/strings-database.ts`, `src/features/designer/settings-model.ts` (+ test), `src/features/designer/database-model.ts` (+ test), `src/features/designer/SettingsEditor.tsx`, `src/features/designer/DatabaseForm.tsx`
- Değiştir: `forms-model.ts` (+ test: `DATABASE_PATHS`), `DesignerView.tsx` (`database` girdisi)

**Arayüzler:**
- Üretir (`settings-model.ts`, saf):

```ts
export function addSetting(list: readonly ConfigSetting[]): ConfigSetting[];                  // { name: "", value: "" } sona
export function updateSetting(list: readonly ConfigSetting[], i: number, patch: Partial<ConfigSetting>): ConfigSetting[];
export function removeSetting(list: readonly ConfigSetting[], i: number): ConfigSetting[];
/** Taslakta olmayan yüklü ayarlar (ad ile, özgün sırayla) — "Kaldırılacaklar" + Geri al. */
export function removedSettings(original: readonly ConfigSetting[] | null, list: readonly ConfigSetting[]): ConfigSetting[];
export function restoreSetting(list: readonly ConfigSetting[], original: readonly ConfigSetting[], name: string): ConfigSetting[];
/** İstemci ön uyarısı (hata değil): boş ad, büyük harf, izin verilmeyen karakter, yinelenen ad. */
export function settingNameWarning(list: readonly ConfigSetting[], i: number): string | null;
/** search_path vb. liste ayarı mı (ipucu: "virgülle ayırın; özel adları çift tırnaklayın"). Arka uçla aynı küme. */
export function isListSetting(name: string): boolean;
```

- Üretir (`SettingsEditor.tsx`, `export function SettingsEditor(props: FormProps & { path: "settings"; list: ConfigSetting[]; original: ConfigSetting[] | null; onChange: (l: ConfigSetting[]) => void })`): satır listesi (ad `input` + `settings` lookup datalist (`value` = ad, etiket `detail`), değer `input`, `isListSetting` ipucu, Sil `Trash2`), "Ayar ekle" (simgesiz `Button`), kaldırılacaklar listesi + `Undo` Geri al, satır hataları `settings[i].name|value` ve `orphanRowIssues(issues, "settings", len)`; not: "Ayarlar yeni oturumlarda etkili olur.".
- Üretir (`database-model.ts`, saf):

```ts
export type DatabaseDraft = DatabaseDef & { kind: "database" };
export function asDatabase(d: ObjectDef | null): DatabaseDraft | null;
export function databaseEditPatch(d: ObjectDef, patch: Partial<DatabaseDef>): ObjectDef;
/** Create-only alanlar alter'da salt okunur. */
export function createOnlyLocked(mode: "create" | "alter"): boolean;      // mode === "alter"
/** Sağlayıcıya göre görünen yerel ayar alanları. */
export function localeFields(provider: LocaleProvider | null): { libc: boolean; providerLocale: "icu" | "builtin" | null };
/** Sağlayıcı değişimi: libc/null'a dönünce providerLocale null'a çekilir. */
export function withProvider(d: DatabaseDef, p: LocaleProvider | null): DatabaseDef;
/** Kodlama/yerel ayar dolu ve şablon template0 değilse ipucu gösterilir. */
export function template0Hint(d: DatabaseDef): boolean;
/** Bağlantı sınırı: "Sınırsız" kutusu ↔ sayı (-1). Girdi: boş/negatif/kesirli → undefined (olay yok sayılır). */
export function connectionLimitFromInput(s: string, badInput?: boolean): number | undefined;
/** Alter'da yeniden adlandırma ya da tablespace değişimi → "bağlı oturum olmamalı" notu. */
export function needsNoSessions(original: DatabaseDef | null, d: DatabaseDef): boolean;
```

- Üretir (`strings-database.ts`, `tdb`): ad/yeniden adlandırma notu ("Veritabanına bağlı oturum olmamalı; bu uygulamanın Gezgin bağlantıları kapatılır"), sahip, **Oluşturma seçenekleri** başlığı + "oluşturulduktan sonra değiştirilemez" notu, şablon (`template_databases` datalist; ipucu "boş = template1"), kodlama (`encodings` `select`; "Varsayılan (şablondan)"), sağlayıcı (`select`: Varsayılan/libc/ICU (PG 15+)/builtin (PG 17+)), LC_COLLATE/LC_CTYPE (`locales{libc}` datalist), ICU/builtin yerel ayarı (`locales{icu|builtin}` datalist), template0 ipucu, tablespace (`pg_default` kuralı; taşıma notu "Dosyalar taşınır; bağlı oturum olmamalı; uzun sürebilir"), bağlantı sınırı + "Sınırsız", "Bağlantılara izin ver" (+ kapatma uyarısı "Yeni bağlantılar reddedilir; Gezgin dahil"), "Şablon veritabanı" (+ uyarı), ayarlar başlığı, yorum, silme notu metinleri gerekirse.
- Üretir (`DatabaseForm.tsx`): **Genel** (Ad + odak, Sahip, Yorum), **Oluşturma seçenekleri** (create'te düzenlenebilir; alter'da aynı alanlar salt okunur `readonly` metin olarak — yüklenen değerler), **Bağlantı ve depolama** (tablespace, sınır, iki bayrak), **Ayarlar** (`SettingsEditor`). `needsNoSessions` iken `role="note"` şerit.

- [ ] **Adım 1: Başarısız testler.** `settings-model.test.ts`: ekle/güncelle/sil değişmez ve sınır dışı → aynı dizi; `removedSettings`/`restoreSetting` özgün sıraya ve özgün değerle (defsEqual sonucu "değişiklik yok"); `settingNameWarning` (boş, `Work_Mem`, `a-b`, yinelenen); `isListSetting` (`search_path`, `temp_tablespaces` true; `work_mem` false). `database-model.test.ts`: `localeFields` tablosu; `withProvider` (icu→libc: providerLocale null); `template0Hint`; `connectionLimitFromInput` (`""` → undefined, `"5"` → 5, `"-1"` → undefined (sınırsız yalnız kutuyla), `"1.5"` → undefined); `needsNoSessions` (rename/tablespace → true; owner → false; create → false). `forms-model.test.ts`: `DATABASE_PATHS` (`name, owner, template, encoding, localeProvider, lcCollate, lcCtype, providerLocale, tablespace, connectionLimit, isTemplate, allowConnections, settings, comment`).
- [ ] **Adım 2–3:** Uygula; `FORMS.database`, `FORM_PATHS.database`.
- [ ] **Adım 4:** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (ana ≤ Task 1; `"Şablon veritabanı"` ana pakette yok; `SettingsEditor` yalnız tembel parçalarda; `__vite__mapDeps` ana pakette değişmedi).
- [ ] **Adım 5: Commit** (`feat(designer): Database formu (create-only seçenekler, sağlayıcı/yerel ayar, bağlantı/depolama) ve ortak ayar düzenleyicisi`).

---

### Task 5: Role formu (öznitelikler, yalnızca yazılır parola, üyelikler, ayarlar)

**Dosyalar:**
- Oluştur: `src/app/strings-role.ts`, `src/features/designer/role-model.ts` (+ test), `src/features/designer/RoleForm.tsx`
- Değiştir: `forms-model.ts` (+ test: `ROLE_PATHS`), `DesignerView.tsx` (`role` girdisi; **`formMissing` dalı ve `Partial` bu görevde kalkar** — artık her tür için form var), `src/app/strings-designer.ts` (`view.formMissing` silinir)

**Arayüzler:**
- Üretir (`role-model.ts`, saf; parola değeri bu modülde **yoktur** — yalnızca niyet ve iki girdinin eşleşme kuralı):

```ts
export type RoleDraft = RoleDef & { kind: "role" };
export function asRole(d: ObjectDef | null): RoleDraft | null;
export function roleEditPatch(d: ObjectDef, patch: Partial<RoleDef>): ObjectDef;
export const ROLE_FLAGS = ["login", "superuser", "createdb", "createrole", "inherit", "replication", "bypassrls"] as const;
export type RoleFlag = (typeof ROLE_FLAGS)[number];
/** Yeni verilen ayrıcalıklı öznitelikler (uyarı): superuser, replication, bypassrls, createrole; create'te true olanlar, alter'da false→true. */
export function privilegedGrants(original: RoleDef | null, d: RoleDef): RoleFlag[];
/** Parola niyeti seçenekleri: create → ["keep", "set"] ("keep" = parolasız); alter → ["keep", "set", "clear"]. */
export function passwordChoices(mode: "create" | "alter"): PasswordChange[];
/** İki girdi: boş → "empty"; farklı → "mismatch"; aynı → null. Değerler çağırana aittir (saklanmaz). */
export function passwordIssue(a: string, b: string): "empty" | "mismatch" | null;
// ---- üyelikler (rol adıyla)
export function addMembership(d: RoleDef, role: string): RoleDef;                 // boş/yinelenen/kendisi eklenmez; admin false; seçenekler null
export function removeMembership(d: RoleDef, i: number): RoleDef;
export function toggleAdmin(d: RoleDef, i: number, admin: boolean): RoleDef;
export function removedMemberships(original: RoleDef | null, d: RoleDef): RoleMembership[];
export function restoreMembership(d: RoleDef, original: RoleDef, role: string): RoleDef;   // özgün nesnenin kopyası, ad sırasına
/** Seçilebilir roller: lookup (grantable_roles) − mevcut üyelikler − rolün kendisi. */
export function availableRoles(items: readonly LookupItem[], d: RoleDef): LookupItem[];
// ---- VALID UNTIL
/** "infinity" | ISO ↔ (date "YYYY-MM-DD", time "HH:MM", utc) girdileri; yerel saat → UTC "…Z". Geçersiz → null. */
export function validUntilParts(v: string | null): { never: boolean; date: string; time: string } | null;
export function validUntilFromParts(p: { never: boolean; date: string; time: string }): string | null;
/** Alter'da yüklü değer varken "süre sınırı yok" seçeneği yalnızca "infinity" yazar (temizleme yok). */
export function validUntilClearValue(original: RoleDef | null): string | null;    // original?.validUntil ? "infinity" : null
```

- Üretir (`strings-role.ts`, `trole`): ad (ayrılmış adlar ipucu: `public`, `none`, `pg_` öneki), öznitelik etiketleri + kısa açıklamalar (LOGIN "Giriş yapabilir", SUPERUSER "Tüm denetimleri atlar", CREATEDB, CREATEROLE, INHERIT "Üye olduğu rollerin yetkilerini devralır", REPLICATION, BYPASSRLS), ayrıcalık uyarısı (`privileged(list)`), bağlantı sınırı + "Sınırsız", **VALID UNTIL** ("Süresiz", tarih, saat, "UTC olarak kaydedilir"), **Parola** başlığı, niyet seçenekleri ("Değiştirme", "Parola belirle", "Parolayı kaldır"; create: "Parolasız", "Parola belirle"), "Yeni parola", "Parola (tekrar)", hatalar ("Parola boş olamaz", "Parolalar eşleşmiyor"), **gizlilik notu** ("Parola sunucuya düz metin gönderilir ve password_encryption ile özetlenir; log_statement açık ya da pg_stat_statements kuruluysa sunucu günlüğüne/istatistiklerine düşebilir. Parola hiçbir zaman yüklenmez ve gösterilmez."), MD5/rename uyarısı, **Üye olduğu roller** (ekle `select` + Ekle düğmesi, satır: rol adı + "ADMIN OPTION" kutusu + PG 16 salt okunur rozetler "INHERIT: evet/hayır", "SET: evet/hayır" + Sil; kaldırılacaklar + Geri al; REVOKE notu "Bu rolün o roldeki yetkileri kaybolur"), ayarlar başlığı, düzenlenmeyenler ("Veritabanına özgü ayarlar", "Birden çok grantor"), yorum, silme notu ("Rolün sahip olduğu nesneler varsa silinemez: REASSIGN OWNED / DROP OWNED").
- Üretir (`RoleForm.tsx`): **Genel** (Ad + odak, Yorum), **Öznitelikler** (`ROLE_FLAGS` onay kutuları, açıklama metinleriyle; `privilegedGrants` uyarısı `text-warn-text`), **Bağlantı** (sınır + Sınırsız; VALID UNTIL: "Süresiz" kutusu + `date` + `time` girdileri, `validUntilFromParts`), **Parola** (niyet `radio` grubu `passwordChoices`; `set` iken iki `type="password"` girdi — değerler **bileşen yerel sinyallerinde**; her girdide `passwordIssue === null` ise `designer.setSecret(tabId, a)`, aksi halde `setSecret(tabId, null)`; niyet `set` dışına çıkınca girdiler temizlenir ve `setSecret(null)`; bileşen `onCleanup`'ta yerel sinyalleri boşaltır; gizlilik notu her zaman görünür), **Üyelikler** (`grantable_roles` lookup), **Ayarlar** (`SettingsEditor`), **Düzenlenmeyenler** (alter'da `unsupported` salt okunur liste). Odak isteği `name`.

- [ ] **Adım 1: Başarısız testler** (`role-model.test.ts`):

```ts
it("privilegedGrants: create'te true olan ayrıcalıklılar; alter'da yalnız false→true", () => {});
it("passwordChoices ve passwordIssue: boş, eşleşmeyen, eşleşen", () => {});
it("addMembership: boş/yinelenen/kendisi eklenmez; inheritOption/setOption null; removeMembership/toggleAdmin değişmez", () => {});
it("removedMemberships/restoreMembership: özgün nesnenin kopyası (PG16 seçenekleri dahil) ad sırasına; defsEqual(original) → true", () => {});
it("availableRoles: mevcut üyelikler ve rolün kendisi hariç; pg_* önceden tanımlı roller dahil", () => {});
it("validUntil: 'infinity' ↔ never; '2030-01-01T10:00:00Z' ↔ parçalar (yerel saat dönüşümü test saat dilimi sabit UTC); geçersiz → null; validUntilClearValue", () => {});
it("role-model hiçbir dışa aktarımında parola değeri tutmaz (modül durumsuz: aynı girdiler aynı çıktı)", () => {});
```

  `forms-model.test.ts`: `ROLE_PATHS` (`name, login, superuser, createdb, createrole, inherit, replication, bypassrls, connectionLimit, validUntil, password, memberOf, settings, comment`); **`FORM_PATHS` her `ObjectKind` için tanımlı** (dokuz tür; `Partial` yok).
- [ ] **Adım 2–3:** Uygula; `FORMS` tam `Record<ObjectKind, FormEntry>`, `formMissing` dalı + metni silinir (`rg "formMissing" src` → 0).
- [ ] **Adım 4:** `bun run test`, `bun run typecheck`, **Boyut doğrulaması** (ana ≤ Task 1; `"Parola (tekrar)"` ana pakette yok); gizlilik greps: `rg "console\." src/features/designer` → boş; `rg "localStorage|sessionStorage" src/features/designer` → boş; `rg "setSecret" src` yalnız `designer-controller.ts`, `RoleForm.tsx` ve testler.
- [ ] **Adım 5: Commit** (`feat(designer): Role formu (öznitelikler, yalnızca yazılır parola, üyelikler, ayarlar); formMissing koruması kaldırıldı`).

---

### Task 6: Uygulama sonrası akışlar ve hata durumları

**Dosyalar:**
- Değiştir: `src/features/designer/refresh-model.ts` (+ test), `designer-controller.ts` (+ test), `src/features/designer/errors.ts` (+ `errors.test.ts`) gerekirse

**Arayüzler:**
- Tüketir: Task 2–5.
- Üretir:
  - `titleAfterApply`: Database/Role/View/MatView yeniden adlandırmada yeni ad (mevcut kural; sunucu kapsamlıda veritabanı adı yok) — testle.
  - `extraRefreshIds`: View/MatView şema taşımada yeni şemanın yüklü `viewCollection`/`materializedViewCollection` çocuğu (sequence/table kuralının genellemesi: `${kindNode}Collection` eşlemesi `{ view: "viewCollection", materialized_view: "materializedViewCollection" }`); Database/Role için ek yenileme yok.
  - Silinen veritabanı: `ApplyOutcome.applied` + `op: drop` + `kind: database` sonrası aynı sunucudaki `databaseOid === <silinen oid>` designer sekmeleri `objectDropped` yoluyla "gone" olur (mevcut `objectDropped` yalnız aynı nesneyi işaretliyorsa: veritabanı kapsamlı tüm sekmeleri işaretleyen `databaseDropped(serverId, databaseOid)` eklenir; sekmeler kapanmaz, "Nesne artık yok" durumunu gösterir).
  - Hata metinleri: 55006/42501/2BP01 arka uç ipuçları mevcut `errorText` + hint yolunda görünür (yeni metin yok); `role_password_missing` Task 2'de.
- [ ] **Adım 1: Başarısız testler.** `refresh-model.test.ts`: view/MV şema taşıma ek yenilemesi (yüklü/ yüklü değil); Database/Role ek yenileme yok. `designer-controller.test.ts`: database drop sonrası o veritabanına ait (veritabanı kapsamlı) açık sekmeler `gone`, sunucu kapsamlı sekmeler etkilenmez; view `recreate` planı DataLoss → üretimde ad yazılmadan gönderilmez (mevcut kapı regresyonu).
- [ ] **Adım 2–3:** Uygula. **Adım 4:** `bun run test`, `bun run typecheck`, **Boyut doğrulaması**.
- [ ] **Adım 5: Commit** (`feat(designer): View/MatView şema taşıma yenilemesi, veritabanı silinince ilgili sekmeler, başlık güncellemeleri`).

---

### Task 7: Elle kontrol listesi, spec durumu ve bütünleşik doğrulama

**Dosyalar:**
- Değiştir: `docs/smoke-test.md`, `docs/superpowers/specs/2026-10-06-object-dialogs-design.md`

- [ ] **Adım 1: `docs/smoke-test.md`.** Plan 14 arka uç bölümündeki "GUI maddeleri Plan 15 bölümündedir" cümlesinin ardına **"## Nesne diyalogları: View/MatView/Database/Role arayüzü (Plan 15)"** (ön koşul: Docker PG 13+ ve 15+ ve 17+ birer sunucu; bir şema, bağımlı iki görünüm, indeksli MV, test veritabanı, CREATEROLE'lü süper kullanıcı olmayan rol). Maddeler (hepsi `- [ ] … **[—]**`):
  - Menü: Views / Materialized Views / Databases / Login-Group Roles koleksiyonlarında **Oluştur…**; görünüm, MV, veritabanı ve kullanıcı rolü düğümünde **Düzenle… / Yeniden adlandır… / Sil…**; `pg_*` sistem rollerinde ve görünüm/MV kolonlarında designer eylemi yok; kilitli sunucuda soluk + gerekçe.
  - View oluştur: sorgu alanı (çok satırlı, yatay kaydırma, imla denetimi yok), `;` ile biten sorgu → alan hatası, `'a\b'` → `E'…'` ipucu; check_option/security_barrier/security_invoker (PG 15 öncesinde öğretici hata) → önizleme `CREATE VIEW … WITH (…) AS (…)`; Uygula → Views yenilenir, sekme "Düzenle: …" başlığına geçer.
  - View düzenle: sona kolon ekleyen sorgu → `CREATE OR REPLACE` + not, Güvenli, GRANT'lar korunur; kolon silen sorgu → uygulama hatası + "Yeniden oluştur" ipucu; kutuyu işaretle → kaybolacaklar listesi + Veri kaybı onayı (üretimde ad yazma) → uygulanır; bağımlı görünüm varken yeniden oluştur → bağımlılar listelenen hata, hiçbir şey değişmez; yeniden adlandır + şema taşı + sahip + yorum birlikte; sorguyu özgüne döndürünce kutu kaybolur.
  - MV: WITH DATA / WITH NO DATA oluşturma; "Veriyle doldur" aç/kapa ve yenileme seçenekleri (CONCURRENTLY yalnız dolu MV'de; benzersiz indeks yoksa öğretici hata); REFRESH Yıkıcı onayı; sorgu değişimi → "yeniden kurulur; indeksler kaybolur" uyarısı + indeks adları + Veri kaybı onayı; tablespace (`pg_default`; temizlenemez), fillfactor.
  - Database oluştur: şablon listesi, kodlama listesi, sağlayıcı (ICU PG 15+, builtin PG 17+; eski sunucuda öğretici hata), LC_COLLATE/LC_CTYPE önerileri, template0 ipucu, bağlantı sınırı/Sınırsız, bayraklar, ayarlar (`search_path` ipucu, `work_mem`), yorum → önizlemede her ifade "transaction dışı" işaretli; Uygula → Databases yenilenir.
  - Database düzenle: oluşturma seçenekleri salt okunur; yeniden adlandır (Gezgin'de o veritabanı açıkken bile başarılı; açık bir Query Tool sekmesi varken öğretici "bağlı oturumlar" hatası + Dashboard ipucu); sahip; tablespace taşıma notu; "Bağlantılara izin ver" kapatma uyarısı; ayar ekle/değiştir/kaldır/geri al; bakım veritabanında yeniden adlandır/sil → açık ret iletisi; kısmi uygulama (ör. yeniden adlandırma + var olmayan sahip) → "N ifade kalıcı" bildirimi.
  - Database sil: CASCADE kutusu yok, FORCE kutusu (PG 13+) + ipucu; Veri kaybı onayı + üretimde ad yazma; bağlı oturum varken FORCE'suz → öğretici hata; FORCE ile → silinir, o veritabanındaki açık designer sekmeleri "artık yok" durumuna geçer.
  - Role oluştur: öznitelikler + açıklamalar, SUPERUSER/REPLICATION/BYPASSRLS/CREATEROLE uyarısı, bağlantı sınırı, VALID UNTIL (Süresiz/tarih/saat, UTC notu), parola (iki alan; eşleşmezse Uygula kapalı + gerekçe, önizleme sürer; önizlemede `PASSWORD ********`; gizlilik notu), üyelik ekle (önceden tanımlı roller listede), ADMIN OPTION, ayarlar, yorum → Uygula → Roles yenilenir; yeni parolayla giriş çalışır (psql ya da yeni sunucu kaydı).
  - Role düzenle: parola "Değiştirme" varsayılan; "Parolayı kaldır" → `PASSWORD NULL`; ad değişimi + MD5 uyarısı; üyelik çıkar → Yıkıcı onayı; ADMIN kaldır; PG 16'da INHERIT/SET rozetleri salt okunur ve değişmeden kalır; VALID UNTIL yüklüyken "Süresiz" → `infinity`; süper kullanıcı olmayan bağlantıyla SUPERUSER verme → yetki hatası + ipucu.
  - Role sil: CASCADE/FORCE yok; nesne sahibi rolde öğretici hata (REASSIGN/DROP OWNED); Yıkıcı onayı.
  - Parola gizliliği: önizleme/kopya/Query Tool'da aç (kapalı + gerekçe) parolayı hiçbir yerde göstermez; uygulama sonrası, yeniden yüklemede ve sekme kapanınca alanlar boş; geliştirici konsolunda/uygulama günlüğünde parola yok.
  - Klavye/erişilebilirlik/tema: tüm alanlar etiketli; radio/onay kutuları klavyeyle; hatalar ekran okuyucuda okunur; açık/koyu tema; uzun sorgu metni yatay sayfa kaydırması yaratmaz; dar pencerede (≈1100 px) form taşmaz.
  - "Ölçümler" altına son ölçüm: "Plan 15 sonu: index-*.js = N B; ViewForm-*.js = …; DatabaseForm-*.js = …; RoleForm-*.js = …".
- [ ] **Adım 2: Spec.** Durum satırı: "Plan 10–15 uygulandı (tüm nesne türleri)". "Sözleşme notları"na kısa **"View/MatView/Database/Role arayüzü (Plan 15) notları"**: sorgu `textarea` (Karar 1), parola yalnızca uygulamada ve denetleyici haritasında (Karar 2–3), "Yeniden oluştur" açık seçim, sunucu kapsamlı sekmeler `databaseOid: null`, ana paket telafisi (`t.query.editorSettings` → `strings-editor.ts`), backlog (CodeMirror sorgu düzenleyicisi; view/MV kolon yeniden adlandırma; rol üyeleri ters yönü; GRANT matrisi).
- [ ] **Adım 3: Bütünleşik doğrulama.** `bun run test`, `bun run typecheck`, `bun run build` + **Boyut doğrulaması** (ana ≤ Task 1 değeri; tembel parça boyutları rapora); `cargo test --workspace` (Docker'sız; Rust değişmedi), `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Adım 4: Kapanış greps:** `console\.` / `localStorage|sessionStorage` / `innerHTML` `src/features/designer` → boş; `strings-(view|database|role|settings)` yalnızca ilgili form/model dosyalarında; designer dışından designer içe aktarmaları yalnız tip, `lazy(() => import(…))`/dinamik `import(…)` ve `ddl-kinds.ts`; `t\.query\.editorSettings` → 0; `rg "password" src/features/designer --glob "!*.test.ts"` çıktısı yalnızca niyet alanı, `setSecret`/`secrets` haritası ve `applyOptionsFor` parametresi (incelemede tek tek).
- [ ] **Adım 5: Commit** (`docs(designer): View/MatView/Database/Role arayüzü elle kontrol listesi, spec durumu; bütünleşik doğrulama`).

---

## Kapanış kontrolü (Plan 15 bitişi)

- Dört tür için tam akış: menü → açma (sunucu kapsamlı/veritabanı kapsamlı) → düzenleme → önizleme → kapılı uygulama (parola yalnız uygulamada) → başlık + ağaç yenileme; silme diyaloğu (FORCE/CASCADE türe göre).
- Ana paket ≤ 233,088 B (Task 1 değeri); yeni kod ve metinler yalnızca tembel parçalarda; `index.html` önyüklemeleri yalnızca solid/web/Icon.
- Elle kontrol maddeleri `[—]`; final inceleme sonrası `main`'e yerel birleştirme **yalnızca kullanıcı onayıyla**.

## Öz inceleme (plan yazımı sonrası)

- **Spec kapsamı:** §4 giriş noktaları (Task 1–2), yerleşim (mevcut çerçeve; formlar Task 3–5), önizleme/uygulama/hata (mevcut + Task 2 parola kapısı, Task 6 ipuçları/yenileme); §5 View (şema/ad/sahip/sorgu/seçenekler/yeniden adlandırma/yeniden oluşturma — Task 3), MatView (WITH [NO] DATA, tablespace, REFRESH — Task 3), Database (tüm alanlar + rename/FORCE/oturum ipucu — Task 2, 4, 6), Role (öznitelikler, limit, VALID UNTIL, yalnız yazılır parola + maskeleme + log_statement uyarısı, üyelikler — Task 2, 5); §6 gizlilik (Task 2, 5, 7 greps); §1.2 ana paket (Task 1 + her görevde boyut doğrulaması).
- **Bilinçli kararlar:** sorgu `textarea` (Karar 1, spec "SQL editörü"nden sapma — bundle kuralı), parola gösterilmez (Karar 2), gizlenmiş planda Query Tool kapalı (Karar 3), yeniden oluşturma açık seçim (Karar 4; Plan 14 Karar 1'in arayüz karşılığı), sürüm alanları gizlenmez (Karar 5).
- **Yer tutucu taraması:** Task 2–4 boyunca `formMissing` yeni türlerde görünür (açıkça belirtilmiş dal içi ara durum), Task 5 kaldırır; "TBD/TODO" yok.
- **Tip tutarlılığı:** `isServerScoped`/`planHasRedacted` `designer-model.ts`'te (Task 2) tanımlanır, açıcı/denetleyici/ApplyBar kullanır; `setSecret`/`secretReady` denetleyicide (Task 2), RoleForm (Task 5) kullanır; `dropRequest(…, force)` Plan 14 Task 1 imzasının genişletilmiş hâli; `FORM_PATHS` Task 5 sonunda tam `Record<ObjectKind, …>`; `ConfigSetting` düzenleyicisi Database ve Role'de aynı `settings` yoluyla.
