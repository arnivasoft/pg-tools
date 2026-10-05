# Query Tool — Plan 6/6: Arayüz (Sekmeler, Editör, Izgara, Onay, Geçmiş) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Plan 5'in arka ucunu (`qt_*` komutları) kullanıcıya açmak: sağ panelde sekmeli Query Tool (Nesne sekmesi + sorgu sekmeleri), düzenlenebilir CodeMirror editörü (Ctrl+Enter/F5/iptal, hata konumu, katalog tamamlama), TanStack Virtual ile satır+sütun sanallaştırmalı sonuç ızgarası (daha fazla yükle, hücre paneli, TSV kopyalama), sonuç şeridi + Mesajlar + Geçmiş, onay/kilit/transaction akışları (tehlikeli ifade onayı, üretim etiketi, manuel transaction, kapanışta açık transaction onayı), CSV dışa aktarma, sunucu diyaloğunda renk/etiket/kilit alanları, taslak ve geçmiş kalıcılığı; ardından bellek/açılış hedeflerinin yayın exe'sinde ölçülmesi.

**Architecture:** İş mantığı **saf TypeScript modüllerinde** (Vitest ile sınanır; DOM yok): ofset dönüşümü, hata kodu koruyucusu, sekme/çalıştırma durum makinesi (`run-model.ts`), sonuç birikimi ve bellek sınırı (`result-model.ts`), ızgara seçimi/TSV/sütun genişliği (`grid-model.ts`), onay kararı (`confirm-model.ts`), geçmiş/taslak modelleri. Solid bileşenleri bu modüllere ince kabuk olur. Kalıcılık (geçmiş, taslaklar) arka uçta küçük bir `pgt-core::store` modülü ve iki Tauri komut grubuyla yapılır (atomik yazım, Rust testleri). Query Tool bileşenleri, CodeMirror ve ızgara **tembel yüklenir** (ilk sorgu sekmesi açılana kadar ana paket değişmez).

**Tech Stack:** Plan 1–5 ile aynı. **Yeni npm bağımlılıkları:** yalnızca `@codemirror/commands` (geçmiş/undo, varsayılan tuş haritaları) ve `@codemirror/autocomplete` (zaten geçişli bağımlılık; doğrudan bildirilir). Başka bağımlılık yok (TanStack Virtual, Kobalte, `@tauri-apps/plugin-dialog` mevcut).

**Spec:** `docs/superpowers/specs/2026-10-06-query-tool-design.md` (§5 arayüz, §6 korumalar, §7 hata, §1.3 ölçütler). **Arka uç sözleşmesi:** `docs/query-tool-plan6-notes.md` — **her görevin implementer'ı bu dosyayı okur**; buradaki her madde bağlayıcıdır.

## Plan 5 sonrası kod gerçekleri

- Tauri komutları ve TS sarmalayıcıları: `src/ipc/commands.ts` (`qtOpen(serverId, databaseOid, options)`, `qtAnalyze(session, sql, scope)`, `qtRun(session, sql, scope, limit, confirmed, onEvent)`, `qtMore(session, stmtIndex, onEvent)`, `qtCancel`, `qtTx(session, action)`, `qtTxState`, `qtSetOptions`, `qtClose(session, policy)`, `qtComplete(session, request)`, `qtExportCsv(session, sql, path, options, onProgress)`). Olaylar `Channel` ile akar; **bir çalıştırma `scriptDone` olayıyla biter**, `invoke` çözülmesiyle değil.
- Bağlamalar `src/ipc/bindings/`: `QueryEvent` (`type`: `started|rows|done|failed|scriptDone`), `StmtPlan` (`index, kind, danger, productionWrite, lockedReject, lockedReason, sensitive, charOffset, charLen`), `Scope` (`all | selection{start,end} | atCursor{offset}`), `SessionOptions{mode: "autocommit"|"manual", statementTimeoutMs}`, `TxState` (`idle|inTransaction|failed`), `TxAction`, `ClosePolicy` (`commit|rollback|refuse`), `CompletionRequest/Item/Kind`, `CsvOptions{delimiter, header, bom}`, `ExportSummary`, `QueryErrorCode`, `ServerConfig{…, color, label, readOnlyLock}`. **Tüm ofsetler Unicode scalar**dır (CodeMirror UTF-16).
- Hata kodları: `confirmation_required`, `locked`, `busy`, `tx_failed`, `open_transaction`, `session_reset` + genel kodlar (`query`, `not_found`, `not_connected`, `unknown_server`, `invalid`, …). Metinle eşleştirme yapılmaz.
- Ön yüz: `src/app/App.tsx` (düzen, tembel diyalog kalıbı `lazy` + `everOpened`), `src/app/{strings.ts, toast.ts, storage.ts, clipboard.ts}`, `src/ui/{Button,Dialog,Field,Tabs,ConfirmDialog}.tsx`, `src/features/inspector/{Inspector.tsx, codemirror-setup.ts (salt okunur editör, tema, vurgulama)}`, `src/features/explorer/{actions.ts (executeAction, actionsFor, qualifiedParts), ExplorerMenu.tsx, tree-model.ts (ancestorOfKind, serverOf)}`, `src/features/servers/{ServerDialog.tsx, server-form.ts}`. Sunucu diyaloğu Kobalte Tabs kullanır (Genel/Bağlantı/SSL).
- Kalıcılık: `%APPDATA%\pg-tools\servers.json` (`Registry`, atomik yazım kalıbı: geçici dosya + rename). `src-tauri/src/state.rs::config_dir()`.
- Test: Vitest `src/**/*.test.ts` (node ortamı). Rust entegrasyon testleri Docker'lı geçici PG ile (`PGT_TEST_URLS`; kalıcı sunucu yok). `scripts/{smoke-release,measure,test-matrix}.ps1`. bun `~/.bun/bin/bun`.

## Global Constraints

- Hedef: Windows 10/11, Server 2019/2022; PG 10–18. Boşta (Query Tool açılmamışken) toplam özel çalışma kümesi < 150 MB **değişmemeli**; Query Tool sekmesi ilk açılış < 500 ms; Query Tool + 10.000 satır < 250 MB; 100.000 satır kaydırma akıcı (sanallaştırma; DOM'da yalnızca görünür hücreler).
- **Arka uç sözleşmesi (`docs/query-tool-plan6-notes.md`) bağlayıcıdır:** ScriptDone ile bitir; ofsetleri UTF-16↔scalar dönüştür (girdiler ve çıktılar); `qtAnalyze` ve `qtRun`'ı **aynı metin ve kapsamla** çağır; hata kodlarıyla dallan; yeni çalıştırma başladıktan sonra eski ızgaradan `more` isteme; `qt_set_options` sonrası "daha fazla"yı kapat; üretim etiketinde DO/CALL/EXECUTE onayı; Failed'da yalnız Rollback; 25001'de "autocommit'e geçin" ipucu; hücre gösterimini kırp.
- Rutin hatalar modal açmaz (sonuç alanında/Mesajlar'da/toast). Onay diyalogları yalnızca: tehlikeli/üretim yazma, açık transaction'lı kapanış, var olan CSV'nin üzerine yazma. Onay diyaloğunda odak **Vazgeç**'te başlar.
- Tüm UI metinleri `src/app/strings.ts`. Komutlar Türkçe, PostgreSQL terimleri İngilizce.
- SQL metni ve parolalar günlüğe yazılmaz; geçmiş Ayarlar'dan kapatılabilir, `sensitive` ifadeler geçmişe **hiç** yazılmaz.
- Yeni bağımlılık yalnızca `@codemirror/commands` ve `@codemirror/autocomplete`. Rust tarafında yeni bağımlılık yok.
- Yeni async Tauri komutları `Box::pin` ile kutulanır.
- Sekme başına yüklü satır üst sınırı 50.000 (aşılırsa en eski ifadenin sonucu bırakılır ve işaretlenir); hücre gösterimi 1.000 karakterde kırpılır (tam değer hücre panelinde, panel de 1 MB'ta kırpar ve uyarır).

### Bu planın bilinçli sınırları

- Gelişmiş editör zekâsı (alias çözümleme, join önerisi, formatter, hata altı çizme LSP) sonraki alt projedir.
- Hücre düzenleme, EXPLAIN görselleştirme, birden çok sonuç kümesinin yan yana gösterimi kapsam dışı.
- İkinci pencere yok (arka uçtaki `on_page_load` tüm oturumları kapatır; tek pencere varsayımı korunur).

## Review Focus

1. **UTF-16 ↔ scalar ofsetleri:** emoji/astral karakter içeren betikte seçim çalıştırma, imleçteki ifade, hata konumu işaretleme ve onay diyaloğundaki ifade metni kaymamalı. Test: Task 1 `offsets` tabloları + Task 5 hata işaretleme testi.
2. **Eski çalıştırmanın olayları:** iptal edilip yeni çalıştırma başlatıldığında eski Channel'ın geç olayları yeni sonuçlara karışmamalı (çalıştırma kimliği); `more` yalnızca son çalıştırmanın açık imleci için. Test: Task 3 `stale events are ignored`.
3. **Onay atlatma:** onay diyaloğundan sonra metin/kapsam değişirse onaylanmamış bir betik `confirmed=true` ile gitmemeli (analiz ile çalıştırma aynı anlık görüntüden). Test: Task 4 `confirmation is bound to the analyzed snapshot`.
4. **Bellek:** 1M satırlık sorguda ızgara yalnızca yüklenen satırları tutar; sekme başına 50.000 sınırı; 1 GB'lık hücre ızgarayı dondurmaz (kırpma). Test: Task 3/6 model testleri + Task 9 ölçümü.
5. **Açık transaction'lı kapanış:** sekme kapatma, uygulama kapatma, sunucu silme/bağlantı kesme sırasında açık transaction kullanıcıya sorulur (kaydet = COMMIT, at = ROLLBACK, vazgeç); Failed durumda "kaydet" sunulmaz. Test: Task 3 durum makinesi + Task 8.
6. **Kilitli/üretim sunucu görünürlüğü:** renkli şerit, etiket ve kilit simgesi her sorgu sekmesinde görünür; kilitli sunucuda yazma istemcide reddedilir (arka uç `locked` döner, Mesajlar'da açık metin).

## Dosya Haritası

```
crates/pgt-core/src/store.rs                 geçmiş + taslak kalıcılığı (Task 2)
src-tauri/src/store_cmds.rs                  qt_history_* / qt_drafts_* komutları (Task 2)
src/features/query/
  offsets.ts, errors.ts                      UTF-16↔scalar, hata kodu koruyucusu (Task 1)
  run-model.ts, result-model.ts              sekme/çalıştırma durum makinesi, sonuç birikimi (Task 3)
  confirm-model.ts, ConfirmRunDialog.tsx     onay kararı ve diyaloğu (Task 4)
  editor.ts, completion.ts                   düzenlenebilir CodeMirror, tamamlama kaynağı (Task 5)
  grid-model.ts, ResultGrid.tsx, CellPanel.tsx   ızgara (Task 6)
  ResultStrip.tsx, MessagesView.tsx, HistoryView.tsx, history-model.ts, export.ts   (Task 7)
  QueryTab.tsx, QueryTabs.tsx, tabs-store.ts, ContextBar.tsx                         (Task 3, 8)
src/features/servers/ServerDialog.tsx, server-form.ts   Güvenlik sekmesi (Task 2)
src/features/explorer/{NodeIcon,TreeRow,ExplorerMenu,actions}.ts(x)   renk noktası, "Query Tool'da aç" (Task 8)
src/app/{App.tsx, strings.ts, Toolbar.tsx}   sekmeli sağ panel, kapanış onayı (Task 8)
docs/…, scripts/smoke-release.ps1            Task 9
```

---

### Task 1: Saf yardımcılar — ofset dönüşümü ve hata kodu koruyucusu

**Files:** Create `src/features/query/offsets.ts`, `src/features/query/errors.ts` (+ `*.test.ts`).

**Interfaces (Produces):**
- `utf16ToScalar(text: string, utf16: number): number` ve `scalarToUtf16(text: string, scalar: number): number` — sınır dışı değerler kırpılır; yarım vekil çiftinin ortasındaki UTF-16 ofseti bir önceki karaktere yuvarlanır.
- `scopeFromEditor(text: string, selFrom: number, selTo: number, cursor: number, kind: "run" | "all"): Scope` — `all` → `{type:"all"}`; seçim boş değilse `selection` (scalar), değilse `atCursor` (scalar).
- `errorRange(text: string, failed: { charOffset: number; position: number | null }): { from: number; to: number } | null` — UTF-16 aralığı: `charOffset + position - 1` karakterinden bir sonraki kelime sınırına (boşluk/noktalama) kadar; `position` null ise null.
- `stmtText(text: string, plan: { charOffset: number; charLen: number }): string`.
- `errors.ts`: `isQueryErrorCode(code: string): code is QueryErrorCode` (`QueryErrorCode.ts` listesi), `hintFor(payload: ErrorPayload): string | null` — `25001` (SQLSTATE detail/mesaj içinde) → `t.query.hint.autocommit`; `tx_failed` → `t.query.hint.rollback`; `session_reset` → `t.query.hint.sessionReset`; `locked` → `t.query.hint.locked`.

- [ ] **Step 1: Başarısız testler.** Tablolar: ASCII, Türkçe (`çğü`), BMP dışı emoji (`😀` = 2 UTF-16 birimi, 1 scalar), birleşik karakter (`é` = 2 scalar), vekilin ortası, metin sonu, negatif/aşırı değer. `scopeFromEditor`: emoji sonrası seçim doğru scalar aralığı verir. `errorRange`: `select '😀' || x` için `x`'in UTF-16 aralığı. Gidiş-dönüş özelliği: rastgele (sabit tohum) metinlerde `scalarToUtf16(utf16ToScalar(t, i))` kelime sınırlarında eşit.
- [ ] **Step 2: Uygula** (`for…of` ile kod noktası sayımı; O(n)); **Step 3:** `bun run test`, `bun run typecheck`; commit `feat(query-ui): ofset dönüşümü ve hata kodu yardımcıları`.

---

### Task 2: Kalıcılık (geçmiş + taslaklar) ve sunucu diyaloğu güvenlik alanları

**Files:** Create `crates/pgt-core/src/store.rs`, `src-tauri/src/store_cmds.rs`; Modify `crates/pgt-core/src/lib.rs`, `src-tauri/src/{main.rs,state.rs}`, `src/ipc/commands.ts`, `src/features/servers/{ServerDialog.tsx, server-form.ts}`, `src/features/explorer/{NodeIcon.tsx or TreeRow.tsx}`, `src/app/strings.ts`.

**Interfaces (Produces):**
- Rust `store.rs`: `HistoryEntry { id: Uuid, sql: String /* ≤10 KB, kesilirse sonuna "…" */, server_id: Uuid, server_name: String, database: Option<String>, at_ms: i64, elapsed_ms: u64, ok: bool }` (`Serialize/Deserialize/TS`, camelCase); `History::{load(path), push(entry) /* en çok 500, en eski silinir, atomik yazım */, list(query: Option<&str>, limit) , clear()}`; `Drafts { dir }` — `DraftTab { id: Uuid, title: String, server_id: Option<Uuid>, database_oid: Option<u32>, options: SessionOptions-benzeri {mode, statement_timeout_ms} }`, `tabs.json` + `<id>.sql`; `save_tab(tab, sql)`, `remove(id)`, `load_all() -> Vec<(DraftTab, String)>`, `clear()`; bozuk `tabs.json` → yedeklenir, boş liste (Registry kalıbı). Ayar `history_enabled: bool` (`settings.json`, varsayılan açık).
- Tauri: `qt_history_push(entry)`, `qt_history_list(query, limit)`, `qt_history_clear()`, `qt_drafts_save(tab, sql)`, `qt_drafts_remove(id)`, `qt_drafts_load()`, `qt_drafts_clear()`, `settings_get()`, `settings_set_history(enabled)`. Geçmiş kapalıyken `push` hiçbir şey yazmaz. **Arka uç da `sensitive` koruması uygular**: `push` gelen SQL'i `pgt_query::classify::analyze` ile tekrar denetler ve `sensitive` ise yazmaz (istemciye güvenilmez).
- Sunucu diyaloğu: yeni **Güvenlik** sekmesi — Etiket (serbest metin + öneriler Üretim/Test/Geliştirme), Renk (8 renk düğmesi + yok), "Salt okunur kilit" onay kutusu; etiket üretim sayılan bir değere değiştirildiğinde kilit kutusu **otomatik işaretlenir** (kullanıcı kaldırabilir; spec §6). Object Explorer sunucu satırında renkli nokta (renk varsa) ve kilitliyse küçük kilit simgesi; `title`'da etiket.

- [ ] **Step 1: Rust testleri** (`store.rs`): 501 kayıtta en eski düşer; 10 KB kesme (UTF-8 sınırında); sensitive SQL yazılmaz; geçmiş kapalıyken yazılmaz; atomik yazım (yarım dosya kalmaz — geçici dosya + rename); bozuk dosya yedeklenir; taslak gidiş-dönüş, silme, `tabs.json` sırası korunur; Türkçe/emoji SQL.
- [ ] **Step 2: Uygula + Tauri komutları + `commands.ts` sarmalayıcıları** (bağlamalar commit'e).
- [ ] **Step 3: Diyalog ve ağaç** — `server-form.ts` testleri: üretim etiketi kilidi önerir (form modelinde `suggestLock(prevLabel, nextLabel, lock)` saf fonksiyonu; kullanıcı kaldırdıysa tekrar zorlamaz); renk/etiket gidiş-dönüş.
- [ ] **Step 4:** `cargo test --workspace --all-features` (Docker'sız yeterli; store testleri dosya tabanlı), `bun run test/typecheck/build`; commit.

---

### Task 3: Sekme ve çalıştırma modeli (saf) — `tabs-store.ts`, `run-model.ts`, `result-model.ts`

**Files:** Create `src/features/query/{run-model.ts, result-model.ts, tabs-store.ts}` (+ testler).

**Interfaces (Produces):**
- `result-model.ts`: `ResultState { runId: number; stmts: StmtResult[]; messages: Message[]; done: boolean; stoppedOnError: boolean }`, `StmtResult { index, kind, columns, rows: (string|null)[][], more: boolean, truncated: boolean, tag?: string, rowsAffected?: number|null, elapsedMs?: number, status: "running"|"ok"|"failed"|"cancelled", error?: ErrorPayload & {position, charOffset}, dropped: boolean }`; `applyEvent(state, runId, ev: QueryEvent): ResultState` — **`runId` eşleşmezse durum değişmez** (Review Focus 2); `rows` olayı satırları ekler, toplam yüklü satır `MAX_TAB_ROWS = 50_000`'i aşarsa en eski ifadenin satırları boşaltılır ve `dropped = true`; `failed` ve `done` NOTICE'ları `messages`'a ekler; `57014` + "kullanıcı" iletisi `status: "cancelled"`; `scriptDone` → `done = true`. `canLoadMore(state, stmtIndex)`: yalnızca son çalıştırmanın, son satır döndüren ifadesinde `more` ise ve `done`.
- `run-model.ts` (sekme durum makinesi): `TabStatus = "noSession" | "opening" | "idle" | "running" | "cancelling" | "lost"`, `TxView = TxState`; olaylar: `open`, `opened`, `openFailed`, `runStart`, `scriptDone`, `cancelRequested`, `txChanged`, `sessionLost`, `optionsChanged` (→ "daha fazla" kapanır). Saf geçiş fonksiyonu + seçiciler: `canRun`, `canCancel`, `canCommit` (`inTransaction`), `canRollback` (`inTransaction|failed`), `closeNeedsConfirm` (`inTransaction|failed`), `closeChoices` (`failed` → yalnızca `rollback`/`cancel`).
- `tabs-store.ts` (Solid store sarmalayıcısı, testte saf işlevleri): sekme ekle/kapat/yeniden adlandır/sırala, aktif sekme, "Nesne" sekmesi sabit ve kapatılamaz; sekme başlığı varsayılanı `"<sunucu>/<db> #n"`.

- [ ] **Step 1: Başarısız testler:** olay dizileri (tek ifade, betik hata ile durur, iptal, `more`, 50.000 sınırı ve `dropped`, eski runId olayları yok sayılır, NOTICE'lar failed'da da), durum makinesi tabloları (kapanış seçenekleri, Failed'da commit yok, optionsChanged sonrası canLoadMore false).
- [ ] **Step 2–3:** uygula, `bun run test/typecheck`, commit.

---

### Task 4: Onay ve koruma akışı — `confirm-model.ts`, `ConfirmRunDialog.tsx`

**Interfaces (Produces):** `decide(plans: StmtPlan[], server: {label, readOnlyLock, isProduction}): { kind: "run" } | { kind: "confirm"; items: {index, reason: "danger"|"production", danger?: Danger, text: string}[] } | { kind: "locked"; items: … }` (metin `stmtText` ile); `isProductionLabel(label)` (Rust `is_production` ile aynı katlama: İ/ı/U+0307, `üretim|uretim|production|prod`; tablo testleri Rust testleriyle aynı satırlar). Çalıştırma akışı (`runWithGuards(snapshot, deps)`): **anlık görüntü** = `{text, scope}` editörden bir kez alınır; `qtAnalyze(snapshot)` → `decide` → gerekiyorsa diyalog → `qtRun(snapshot, confirmed)`; arka uç yine `confirmation_required` dönerse (ör. etiket arada değişti) diyalog **yeni analizle** yeniden gösterilir; `locked` → çalıştırmadan Mesajlar'a açıklama.

- [ ] **Step 1: Testler:** WHERE'siz UPDATE → confirm(danger); üretim + `insert` → confirm(production); üretim + `do $$…$$` → confirm; kilitli + `delete` → locked; `confirmation is bound to the analyzed snapshot` (onaydan sonra editör değişse de `qtRun` analiz edilen metni gönderir — sahte deps ile); arka uç `confirmation_required` dönerse yeniden analiz.
- [ ] **Step 2:** Diyalog: tehlike türüne göre Türkçe açıklama (`strings.ts`), ifade metinleri (kırpılmış, eş genişlikli yazı), üretim sunucuda kırmızı şerit; odak **Vazgeç**; "Çalıştır" düğmesi yalnızca fareyle/Tab ile. Commit.

---

### Task 5: Düzenlenebilir editör ve katalog tamamlama

**Files:** Create `src/features/query/{editor.ts, completion.ts}` (+ `completion.test.ts`); Modify `src/features/inspector/codemirror-setup.ts` (ortak tema/vurgulama dışa aktarılır); `package.json` (`@codemirror/commands`, `@codemirror/autocomplete`).

**Interfaces (Produces):** `createQueryEditor(parent, doc, handlers: { onRun(kind: "stmt"|"all"), onCancel(), onChange(doc) , complete: CompletionSource }) : { view, setErrorMark(range|null), getSnapshot(kind): {text, scope}, setDoc(text), focus() }`. Tuşlar: `Ctrl+Enter` (seçim ya da imleçteki ifade), `F5` (tümü), `Ctrl+.` (iptal), `Ctrl+/` (satır yorumu), geçmiş (undo/redo), çoklu imleç, parantez eşleme, satır numaraları, PostgreSQL vurgulama. Hata işareti: `Decoration.mark` (kırmızı dalgalı alt çizgi) + gutter işareti; metin değişince temizlenir.
`completion.ts`: saf `contextAt(textBefore: string): {kind: "schemaMember", schema} | {kind: "relationMember", relation, schema?} | {kind: "relation"} | {kind: "keyword"}` — `şema.` sonrası, `tablo.` sonrası, `FROM`/`JOIN`/`UPDATE`/`INTO` sonrası; çift tırnaklı adlar; yorum/dize içinde `keyword`'e düşmez → **hiç öneri yok**. Kaynak `qtComplete`'i çağırır (yazarken 150 ms debounce, CodeMirror iptal sinyalinde sonuç yok sayılır), boş listede anahtar kelimelere düşer; ekleme metni `quoted`. **Çalıştırma sürerken tamamlama istenmez** (arka uç kilidi tutmasın — plan6-notes "Diğer").

- [ ] **Step 1: Testler** (`contextAt` tablosu: `select * from app.`, `select o.` (alias yok → relation `o`), `from "Order Items".`, `-- from x.`, `'from x.'`, `join `, `update `, Türkçe ad). **Step 2:** editör + tamamlama; ilk açılış süresi (CodeMirror tembel) not edilir. Commit.

---

### Task 6: Sonuç ızgarası — `grid-model.ts`, `ResultGrid.tsx`, `CellPanel.tsx`

**Interfaces (Produces):** `grid-model.ts`: `initialWidths(columns, sampleRows /* ilk 200 */, measure: (s)=>number): number[]` (min 60 px, max 480 px), `isNumericType(typeName)` (sağa hizalama: int2/4/8, numeric, float4/8, oid, money), `displayCell(v: string|null): {text, null: boolean, truncated: boolean}` (1.000 karakter kırpma, satır sonları `⏎`), `selectionToTsv(rows, columns, sel: {r0,r1,c0,c1}, opts: {headers})` (TSV: sekme/satır sonu içeren değerler tırnaklanır, NULL boş), seçim modeli (hücre/satır/sütun, Shift ile genişletme, Ctrl+A).
`ResultGrid.tsx`: TanStack Virtual ile **satır ve sütun** sanallaştırma (24 px satır; sabit başlık ve satır numarası sütunu), sütun genişliği sürükleme + çift tık otomatik sığdır, `[NULL]` soluk italik, klavye ile gezinme, `Ctrl+C` TSV kopyalar (`copyText`), alt bilgi: "N satır yüklendi" + `truncated`/`more` ise "Daha fazla yükle (1.000)" düğmesi + sınır uyarısı; `dropped` ifade için açıklama. `CellPanel.tsx`: seçili hücrenin tam değeri (JSON ise güzel yazdırma, 1 MB'ta kırp + uyarı), kopyala düğmesi.

- [ ] **Step 1: Testler** (`displayCell` kırpma ve NULL/boş ayrımı, `selectionToTsv` kaçışları, `initialWidths` sınırları, `isNumericType`). **Step 2:** bileşen; 100.000 satırlık sahte veriyle (dev'de) kaydırma ve DOM düğüm sayısı (`document.querySelectorAll('[role=gridcell]').length` < 2.000) elle doğrulanır ve rapora yazılır. Commit.

---

### Task 7: Sonuç şeridi, Mesajlar, Geçmiş ve CSV dışa aktarma

**Interfaces (Produces):** `ResultStrip.tsx` (ifade başına satır: `#`, tür, durum ✓/✗/⏹, satır sayısı/etkilenen, süre; tıklayınca ızgara o ifadeyi gösterir); `MessagesView.tsx` (NOTICE/WARNING, hata ayrıntısı `detail`/`hint`/konum, `hintFor` ipuçları, komut etiketleri); `history-model.ts` + `HistoryView.tsx` (arama, tıklayınca editöre ekle — imleç konumuna; Ayarlar'da "Sorgu geçmişini kaydet" anahtarı ve "Geçmişi/taslakları temizle" düğmesi onaylı); her `scriptDone` sonrası çalışan ifadeler için `qt_history_push` (yalnızca `sensitive=false` planlar; analiz sonucu kullanılır). `export.ts`: "CSV'ye aktar" → yalnızca tek satır döndüren ifade seçiliyse etkin; `@tauri-apps/plugin-dialog` `save` (filtre `*.csv`, varsayılan ad `<tablo|sorgu>-<tarih>.csv`); hedef varsa **üzerine yazma onayı**; seçenekler (ayırıcı, başlık, BOM); ilerleme (satır sayısı, iptal düğmesi = `qtCancel`); sonuç toast'u; `session_reset` ve hata kodları Mesajlar'da.

- [ ] **Step 1: Testler** (`history-model`: arama, ekleme konumu; geçmişe yazılacak ifadelerin seçimi — sensitive hariç, başarısızlar `ok:false`; export etkinlik kuralı). **Step 2:** bileşenler. Commit.

---

### Task 8: Sekmeli sağ panel, bağlam şeridi, "Query Tool'da aç", kapanış akışları

**Files:** Create `src/features/query/{QueryTabs.tsx, QueryTab.tsx, ContextBar.tsx}`; Modify `src/app/App.tsx`, `src/features/explorer/{actions.ts, ExplorerMenu.tsx}`, `src/app/{Toolbar.tsx, strings.ts}`, `src-tauri/capabilities/default.json` (pencere kapatma isteği için gereken izin, örn. `core:window:allow-close`/`allow-destroy`).

Davranış (spec §5.1, §5.6, §6, plan6-notes):
- Sağ panel = sekme çubuğu: sabit **Nesne** (mevcut Inspector) + sorgu sekmeleri (kapat düğmesi, orta tık kapatır, sürükleyerek sırala opsiyonel değil — yok). Query Tool bileşenleri `lazy` ile yüklenir; ilk sorgu sekmesi açılana kadar CodeMirror/ızgara paketleri inmez.
- **Açma:** Object Explorer bağlam menüsünde sunucu/veritabanı/şema/tablo/görünüm için **"Query Tool'da aç"** (Ctrl+Alt+Q seçili düğümde); araç çubuğunda "Yeni sorgu" (seçili düğümün sunucusu/veritabanı). Tabloda/görünümde taslak `SELECT * FROM <nitelikli ad> LIMIT 100;` (`quoteIdentifiers`). Veritabanı OID'i ağaçtan (`ancestorOfKind(…,["database"])` düğüm kimliğinden değil — kimlik ayrıştırılmaz; `TreeNode`'da OID yoksa `databaseOid: null` ile bakım veritabanı). **Etiketsiz sunucuda** ilk açılışta ortam istemi: "Bu sunucunun ortamı?" Üretim / Test / Geliştirme / Atla → seçim `saveServer` ile kaydedilir (Üretim → kilit önerisi diyalogda gösterilir); sekme oturumu **ilk çalıştırmada** açılır (`qtOpen`), mod varsayılanı üretim etiketinde **Manuel**.
- **Bağlam şeridi:** renkli üst çizgi + etiket, sunucu/veritabanı adı, kilit simgesi, transaction modu seçici (açık transaction varken devre dışı), zaman aşımı seçici (30 sn / 5 dk / sınırsız), Çalıştır / Tümünü çalıştır / İptal / Commit / Rollback düğmeleri (durum makinesine göre), açık transaction rozeti ve süre sayacı, Failed rozeti ("yalnızca Rollback").
- **Kapanış:** sekme kapatma `closeNeedsConfirm` ise diyalog (Kaydet=COMMIT / At=ROLLBACK / Vazgeç; Failed'da Kaydet yok) → `qtClose(policy)`; meşgul sekme kapatma "sorgu iptal edilip geri alınacak (≤10 sn)" bilgisiyle `rollback`. **Uygulama kapatma:** `getCurrentWindow().onCloseRequested` → açık transaction'lı sekmeler listelenir (tek diyalog: Tümünü kaydet / Tümünü at / Vazgeç) → sonra kapat. Sunucu silme/bağlantı kesme/kilit değişimi öncesinde, o sunucunun açık transaction'lı sekmeleri varsa aynı diyalog (arka uç zaten geri alır; diyalog kullanıcıyı uyarır). Oturumu arka uçta kapanan sekme (sunucu silindi, kilit değişti, sayfa yenilendi) `lost` durumuna geçer: "Yeniden bağlan" düğmesi.
- **Taslak kalıcılığı:** metin/sunucu/veritabanı/ayar değişikliklerinde 5 sn sessizlikte ve kapanışta `qt_drafts_save`; açılışta `qt_drafts_load` → sekmeler **bağlanmadan** geri gelir; sunucusu silinmiş sekme "sunucu yok" (salt okunur taslak).
- Hata kodu akışları: `busy` → toast; `session_reset` → Mesajlar'da açıklama + "Tekrar çalıştır"; `not_connected`/`unknown_server` → `lost`; `25001` → autocommit ipucu.

- [ ] **Step 1: Saf testler** (Task 3 modellerine eklenen: açma bağlamı → başlık/taslak/varsayılan mod; etiketsiz sunucu istemi kararı; kapanış diyaloğu seçenekleri; uygulama kapanışında sekme listesi; taslak debounce kararı). **Step 2:** bileşenler ve App entegrasyonu. **Step 3:** `bun run test/typecheck/build`; `bun run tauri dev` ile elle duman (rapora): sekme aç/çalıştır/iptal/manuel commit/kapanış onayı/yenileme sonrası taslaklar. Commit.

---

### Task 9: Ölçüm, uçtan uca duman testi ve belgeler

- [ ] **Step 1: Yayın ölçümleri.** `bun run build`, `cargo build --release -p pg-tools`, `scripts/smoke-release.ps1` (0), `scripts/measure.ps1 -Runs 5` → boşta toplam özel çalışma kümesi **< 150 MB ve Plan 5 değerinden (73 MiB) anlamlı farksız** (Query Tool tembel). Query Tool açık + 10.000 satır ölçümü: `measure.ps1`'e `-Scenario query` seçeneği ekleyin YA DA elle ölçün (CDP/WebView2 devtools ile 10.000 satırlık sorgu çalıştırıp `measure.ps1`'in süreç ağacı toplamını alın) → < 250 MB; ilk sorgu sekmesi açılış süresi < 500 ms (performance.now farkı, rapora). Hedef aşılırsa `docs/performance.md` karar kuralı; kuralda olmayan iyileştirme gerekiyorsa `BLOCKED`.
- [ ] **Step 2: Paket boyutu.** `dist/assets` ana parça boyutu öncesi/sonrası (Query Tool parçaları ayrı olmalı); NSIS boyutu < 15 MB.
- [ ] **Step 3: Elle kontrol listesi** `docs/smoke-test.md`'ye "Query Tool" bölümü: renkli üretim etiketi ve kilit, tehlikeli ifade onayı, manuel transaction + kapanış onayı, iptal (tekrar basma), CSV (üzerine yazma onayı, iptal), emoji içeren betikte seçim çalıştırma ve hata işareti, 100.000 satır kaydırma, taslakların yeniden açılışta gelmesi, geçmiş arama ve kapatma, `session_reset` (sunucuyu yeniden başlat → durumlu sekme).
- [ ] **Step 4: Belgeler.** `docs/performance.md` (Plan 6 ölçümleri, birimler MiB), `docs/testing.md` (ön yüz model testleri), spec durum satırı "Uygulandı (Plan 5–6)", `docs/query-tool-plan6-notes.md` başına "Plan 6'da karşılandı" işaretleri (madde madde, karşılanmayanlar açıkça).
- [ ] **Step 5:** `cargo test --workspace --all-features` (Docker'lı PG), `bun run test`, `bun run typecheck`, `bun run build`; commit.
