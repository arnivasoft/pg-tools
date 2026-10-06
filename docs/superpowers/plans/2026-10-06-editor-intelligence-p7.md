# Gelişmiş Editör Zekâsı — Plan 7 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Query Tool editörüne alias/CTE/alt sorgu çözümlemeli tamamlama, sunucuda Parse ile yazarken hata altı çizme ve belirteç-korumalı SQL biçimlendirici eklemek.

**Architecture:** Ayrıştırma mantığı Rust'ta `pgt-query` içinde (`scope.rs`, `check.rs`, `format.rs`), mevcut `lex` üstünde. Sunucu çağrıları (Parse) Query Tool oturumunun kendi bağlantısında, Plan 5 tamamlama kurallarıyla (yalnızca boşta, savepoint koruması, kısa kilit). Ön yüz CodeMirror lint katmanı, alias bağlamı ve biçimlendir komutu ekler.

**Tech Stack:** Plan 5–6 ile aynı. **Yeni bağımlılık yok** (CodeMirror lint için `@codemirror/lint` GEREKMEZ: Plan 6 hata işareti dekorasyonu genişletilir; `@codemirror/lint` yalnızca zaten kuruluysa kullanılabilir).

**Spec:** `docs/superpowers/specs/2026-10-06-editor-intelligence-design.md` (bağlayıcı). **Arka uç sözleşmesi:** `docs/query-tool-plan6-notes.md` (ofsetler Unicode scalar; hata kodları; tamamlama meşgulken boş).

## Kod gerçekleri

- `crates/pgt-query/src/{lex.rs (lex, Kind, Token{kind,start,end,depth}), split.rs (split, stmt_at, Stmt), classify.rs (analyze, Analysis, StmtKind), exec.rs (Executor: exec_internal, in_tx, cursor_open), session.rs (QueryManager: complete, CompletionRequest/Item, SAVEPOINT pgt_complete koruması, try-lock + 5 sn tamamlama zaman aşımı, Plan 5/6 önbellek), error_codes.rs}`; `src-tauri/src/query.rs` (`qt_*`, `Box::pin`); `crates/pgt-core/src/store.rs` (`AppSettings` — `settings.json`, `history_enabled`).
- Ön yüz: `src/features/query/{editor.ts (createQueryEditor, setErrorMark, getState/setState/createState), completion.ts (contextAt, createCompletionSource, replaceRange, matchItems), offsets.ts, QueryTab.tsx, query-controller.ts, ContextBar.tsx}`, `src/app/strings.ts`, `src/ipc/commands.ts`.
- Testler: Vitest (node), Rust birim + Docker'lı geçici PG ile entegrasyon (`PGT_TEST_URLS`; kalıcı test sunucusu yok; kapsayıcıyı uygulayıcı başlatır/siler; URL/parola asla yazdırılmaz), `scripts/test-matrix.ps1 [-SkipPerf]` (PG 10–18). Yayın doğrulaması yalnızca `bun run tauri build --no-bundle` exe'siyle (`scripts/smoke-release.ps1` arayüzsüz exe'yi reddeder).

## Global Constraints

- Hiçbir yeni yol kullanıcı SQL'ini **çalıştırmaz**: yalnızca protokol düzeyi Parse (`client.prepare`), `Bind`/`Execute` yok.
- Denetim ve alias çözümlemesi kullanıcı transaction'ını ve oturum durumunu değiştirmez: `Idle` → doğrudan; `InTransaction` → `SAVEPOINT` + `ROLLBACK TO` + `RELEASE`; `Failed` → yapılmaz. `lock_timeout` 200 ms (yalnızca bu işlem için; oturumun ayarı geri döner). Oturum kilidi en çok 1 sn; meşgulken istek yapılmaz/boş döner; `run`/`more`/`export` başlarken süren denetim iptal edilir.
- Biçimlendirici yalnızca boşluk ve anahtar kelime harf büyüklüğünü değiştirir; belirteç dizisi eşitliği her çağrıda doğrulanır, bozulursa sonuç **verilmez** (`Error::Invalid` + "biçimlendirilemedi"); eşkuvvetli.
- Ofsetler Unicode scalar (girdi ve çıktı); ön yüz UTF-16 dönüştürür (`offsets.ts`).
- SQL metni ve parola günlüğe yazılmaz; denetim istekleri geçmişe girmez. Tamamlama/denetim hataları toast açmaz (sessiz).
- Yeni async Tauri komutları `Box::pin`; dosya G/Ç ana iş parçacığında değil.
- UI metinleri `src/app/strings.ts`. Yeni bağımlılık yok. Boşta bellek < 150 MB değişmez.

## Review Focus

1. **Parse yan etkisizliği:** `CREATE TABLE`, `DROP TABLE`, `TRUNCATE`, `CREATE INDEX`, `VACUUM`, `SET`, `DO`, `CALL`, `SELECT nextval(...)` denetlendiğinde hiçbir kalıcı ya da oturum etkisi olmamalı (PG 10–18). Test: Task 3.
2. **Transaction güvenliği:** açık transaction'da denetim sonrası `pg_locks`'ta ek kilit kalmamalı, transaction `InTransaction` kalmalı; `Failed`'da denetim yok; kilit bekleyen tabloda ≤ 200 ms vazgeçiş. Test: Task 3.
3. **Biçimlendirici bozulması:** dolar tırnaklı gövde, `E''` dizeler, iç içe yorum, `::`, `->>`, `BEGIN ATOMIC`, Unicode/emoji, çok uzun satır — belirteç dizisi aynı, eşkuvvetli. Test: Task 4 fuzz + altın dosyalar.
4. **Kapsam çözümleme sınırları:** korelasyonlu alt sorgu, CTE'nin CTE'ye bağımlılığı, `LATERAL`, `USING`, tırnaklı/büyük harfli alias, bozuk sözdizimi — panik yok, makul öneri. Test: Task 1/2.
5. **Kullanıcı işini bekletmeme:** denetim/çözümleme sürerken `run` en çok ~1 sn gecikir ya da denetim iptal edilir; `busy` dönmez (ön yüz çalıştırmadan önce denetimi iptal eder). Test: Task 3/5.
6. **Eski sonuçlar:** yazmaya devam edilirken gelen eski denetim sonuçları yanlış konumu işaretlemez (sürüm sayacı). Test: Task 5.

---

### Task 1: Kapsam çözümleyici (`scope.rs`, saf)

**Files:** Create `crates/pgt-query/src/scope.rs` (+ `lib.rs` modülü). **Produces:** `pub enum ScopeItem { Relation{alias, schema, name}, Cte{name, body: Range<usize>, columns: Option<Vec<String>>}, Subquery{alias, body, columns}, Function{alias, name} }`; `pub fn scope_at(sql: &str, offset: usize /* bayt */) -> Vec<ScopeItem>` (iç blok önce, sonra dış bloklar, sonra CTE'ler); `pub fn resolve(items: &[ScopeItem], qualifier: &str) -> Option<&ScopeItem>` (tırnaksız ad küçük harfe katlanır, tırnaklı ad olduğu gibi; alias öncelikli, yoksa ad); `pub fn ident_norm(token_text) -> String`.
- [ ] **Step 1: Testler** — spec §7 tablosu: `select o.| from app.orders o` (imleç FROM'dan önce), `… from orders as o join customers c on …`, `from "Order Items" "OI"`, `from a, b x`, `join t using (id)`, `from lateral fn(x) f`, `from only t`, iç içe alt sorgu + korelasyon (`select (select o2. from orders o2 where o2.id = o.id) from orders o`), `with recursive r(a,b) as (…) select r.`, CTE→CTE, `update t a set … from u b where a.`, `delete from t a using u b where a.`, `insert into t as a values (…) returning a.`, `from generate_series(1,3) g`, bozuk sözdizimi (açık parantez, yarım `JOIN`), Unicode tanımlayıcılar. Panik yok fuzz (Plan 5 lex fuzz kalıbı).
- [ ] **Step 2: Uygula** — `lex` belirteçleri + derinlik ile; sorgu bloğu sınırları parantezlerle; `FROM`/`JOIN`/`USING`(DELETE)/`UPDATE`/`INSERT INTO` öğe ayrıştırması; anahtar kelime olan alias adayları (`WHERE`, `ON`, `JOIN`, `LEFT`, `GROUP`, `ORDER`, `LIMIT`, `UNION`…) alias sayılmaz.
- [ ] **Step 3:** `cargo test -p pgt-query`, clippy; commit `feat(query): kapsam çözümleyici (alias, CTE, alt sorgu)`.

### Task 2: Alias'lı tamamlama (arka uç) ve ön yüz bağlamı

**Files:** Modify `crates/pgt-query/src/session.rs` (`CompletionRequest::Scoped { sql, offset, prefix }` — ofset Unicode scalar), `src-tauri/src/query.rs` (gerekirse), `src/features/query/completion.ts`, `src/ipc/bindings`. **Produces:** `Scoped` isteği: `scope_at` + `resolve(prefix)` → `Relation` → katalog kolonları (`pg_catalog.to_regclass($1)` ile `search_path`'e dokunmadan; şema verilmişse nitelikli); `Cte`/`Subquery` (kolon listesi yoksa) → gövde Parse (§Global korumaları; CTE bağımlılıkları `WITH` ile eklenir) → kolon adları/türleri; `Function` → alias; eşleşme yoksa Plan 5 `Columns` davranışı. Parse-kolon önbelleği (gövde karması, 60 sn; DDL/`SET` ile temizlenir). Ek: `select ` listesinde alias'sız yazarken kapsamdaki ilişkilerin `alias.kolon` önerileri (en çok 500).
- [ ] **Step 1: Testler** — Rust birim (resolve + istek kurma), Docker'lı: `select o. from app.orders o` kolonlar; `"Order Items" oi`; CTE kolonları Parse ile (ifade çalışmaz — `with x as (insert into t values (1) returning id) select x.` denetiminde `t`'ye satır eklenmez); `InTransaction` iken savepoint koruması (transaction bozulmaz, kilit kalmaz); `Failed` iken boş; meşgulken boş; `search_path` farklıyken doğru tablo. Ön yüz: `completion.ts` `relationMember` bağlamı önce `Scoped` ister (ofset scalar dönüşümü testli), boşsa Plan 5 yoluna düşer.
- [ ] **Step 2: Uygula**; **Step 3:** testler (PG 18 + PG 10), `bun run test/typecheck`; commit.

### Task 3: Parse tabanlı hata denetimi (`check.rs`, `qt_check`)

**Files:** Create `crates/pgt-query/src/check.rs`; Modify `session.rs` (`QueryManager::check(session, sql, scope) -> Result<Vec<CheckDiag>>`, iptal edilebilir; `run`/`more`/`export_csv` başında süren denetimi iptal eder), `error_codes.rs` (gerekirse), `src-tauri/src/query.rs` (`qt_check`, `Box::pin`), `src/ipc/commands.ts`. **Produces:** `#[derive(Serialize, TS)] struct CheckDiag { stmt_index: u32, char_offset: u32, position: Option<u32>, message: String, sqlstate: Option<String>, severity: "error" | "info" }`; en çok 20 ifade / 64 KB; isim hataları (`42P01, 42703, 42883, 42704, 3F000`) betikte önceki bir `Ddl` ifadesi varsa `info`; `42P18` gösterilmez; kilit zaman aşımı/iptal → o ifade için teşhis yok.
- [ ] **Step 1: Testler (PG 10–18 matrisinde)** — Review Focus 1 ve 2 tamamı: yan etkisizlik listesi (her ifade denetlenir, sonra tablo/dizi/oturum ayarı/`pg_stat_user_tables` değişmediği doğrulanır); `InTransaction`'da `pg_locks` farkı yok ve `tx_state == InTransaction`; `Failed`'da `check` boş; `ACCESS EXCLUSIVE` kilitli tabloya başvuran ifade ≤ 300 ms'de teşhissiz döner; denetim sürerken başlayan `run` ≤ 1,2 sn içinde başlar ve `busy` dönmez; sözdizimi hatası konumu (emoji öncesi metinle); önceki DDL'e bağlı isim hatası `info`.
- [ ] **Step 2: Uygula** (`lock_timeout`: `Idle`'da `SET lock_timeout = 200` + Parse + `RESET lock_timeout`'u kullanıcı ayarını koruyarak — önce `SHOW lock_timeout` ile değeri al ve geri yükle; `InTransaction`'da savepoint içinde `SET LOCAL`). **Step 3:** testler + `scripts/test-matrix.ps1 -SkipPerf`; commit.

### Task 4: Biçimlendirici (`format.rs`, `qt_format`)

**Files:** Create `crates/pgt-query/src/format.rs` + `tests/format/` altın dosyaları; Modify `src-tauri/src/query.rs` (`qt_format`, async, `Box::pin`), `src/ipc/commands.ts`. **Produces:** `pub struct FormatOptions { keyword_case: Upper|Lower|Preserve, indent: u8 /*2|4*/, comma: Trailing|Leading, max_line: u16 /*80..200*/ }` (Serialize/Deserialize/TS); `pub fn format(sql: &str, opts) -> Result<String>` — spec §5.1 kuralları; tanınmayan ifade (`CREATE FUNCTION … BEGIN ATOMIC`, `COPY`, `DO`, yorum-yalnız) → yalnız boşluk sadeleştirme; **her çağrının sonunda** belirteç dizisi eşitliği (boşluk hariç; anahtar kelime harfine duyarsız; yorumlar dahil) ve eşkuvvetlilik kontrolü — başarısızsa `Err(Invalid("biçimlendirilemedi"))`.
- [ ] **Step 1: Testler** — altın dosyalar (en az 25: basit/çok tablolu SELECT, CTE, alt sorgu, CASE, UNION, INSERT…SELECT, UPDATE…FROM, DELETE…USING, ON CONFLICT, window fonksiyonu, `::` ve JSON operatörleri, uzun seçim listesi, yorumlu sorgu, dolar tırnaklı fonksiyon, DO bloğu, emoji içeren dize); seçenek kombinasyonları; fuzz: Plan 5 ayırıcı korpusu + rastgele birleştirilmiş parçalar (sabit tohum, 2.000 deneme) → belirteç eşitliği + eşkuvvetlilik; 10.000 satır < 200 ms (release benzeri ölçüm, `#[ignore]` değil ama süreyi gevşek doğrula: < 2 sn debug).
- [ ] **Step 2: Uygula**; **Step 3:** `cargo test -p pgt-query`, clippy; commit.

### Task 5: Ön yüz — lint katmanı, biçimlendir komutu, ayarlar

**Files:** Modify `src/features/query/{editor.ts, QueryTab.tsx, query-controller.ts, ContextBar.tsx}`, Create `src/features/query/check-model.ts` (+test), Modify `crates/pgt-core/src/store.rs` (`AppSettings`: `check_enabled: bool = true`, `format: FormatOptions` — pgt-core'da aynı JSON biçimli bir kopya tip; `#[serde(default)]` ile eski `settings.json` uyumlu), `src-tauri/src/store_cmds.rs`, `src/app/strings.ts`, Ayarlar arayüzü (Geçmiş panelindeki ayar bölümü yanına "Editör" bölümü).
**Produces:** `check-model.ts`: sürüm sayacı, debounce 600 ms, yalnızca `tab.status == idle` ve oturum açık ve `txState != failed` iken istek; `run`/`more`/`export` başlamadan önce süren denetim iptali (`qt_cancel` değil — denetim iptali için `QueryManager::check`'in kendi iptal belirteci: `qt_check_cancel(session)` ya da `qt_run` başında arka uç iptal eder; Task 3'teki davranışla uyumlu seçim); sonuçları scalar→UTF-16 dönüştürüp editöre lint dekorasyonu olarak verir (hata: kırmızı dalgalı, bilgi: gri noktalı; gutter; üzerine gelince ileti; metin değişince ilgili aralık temizlenir). Editör: `Ctrl+Shift+F` biçimlendir (seçim/tümü, tek geri alma adımı, imleç korunur), biçimlendirilemezse toast `t.query.format.failed`. Bağlam şeridinde denetim aç/kapa düğmesi.
- [ ] **Step 1: Testler** — `check-model` (sürüm sayacı eski sonucu yok sayar; meşgulken istek yok; Failed'da istek yok; debounce; run öncesi iptal), biçimlendirme uygulama yardımcısı (seçim aralığı, imleç eşleme), ayarlar gidiş-dönüş (Rust: eski settings.json okunur).
- [ ] **Step 2: Uygula**; **Step 3:** `bun run test/typecheck/build`, `cargo test --workspace --all-features`; `bun run tauri dev` + CDP ile elle: hata altı çizme, info düzeyi, alias tamamlaması, Ctrl+Shift+F, açık transaction'da denetim; rapora yaz. Commit.

### Task 6: Doğrulama ve belgeler

- [ ] `bun run tauri build --no-bundle` + `scripts/smoke-release.ps1` (0); `scripts/measure.ps1 -Runs 5` (boşta < 150 MB, Plan 6'dan anlamlı farksız); `scripts/test-matrix.ps1` (perf dahil, PG 10–18); `bun run test/typecheck/build`.
- [ ] Ölçümler (CDP): alias tamamlama gecikmesi (sıcak/soğuk), denetim gecikmesi, 10.000 satırlık betik biçimlendirme süresi; `docs/performance.md` (MiB).
- [ ] `docs/smoke-test.md` "Editör zekâsı" bölümü; `docs/testing.md` (scope/check/format testleri); spec durum satırı "Uygulandı (Plan 7)"; commit.
