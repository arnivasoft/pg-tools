# Görsel EXPLAIN (Plan 28) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Query Tool'a "Planı göster" (F7) ve "Planı göster (ANALYZE)" (Shift+F7). Arka uç `qt_explain` komutu tek ifadeyi `EXPLAIN (FORMAT JSON, …)` ile sarar ve mevcut güvenlik kapılarından geçirir; yazan ifadelerde ANALYZE varsayılan olarak geri alınır. Ön yüz yeni "Plan" sekmesinde girintili plan ağacını gösterir: süre ve maliyet çubukları, satır sapması rozetleri, en pahalı düğümler, ayrıntı paneli, ham JSON.

**Mimari:** Arka uç `pgt-query` oturum yöneticisine yeni bir yürütme yolu ekler. Aynı bağlantı, aynı kapılar (`plan_script` + `check_gates` sarmalanmış metinle), `statement_timeout` ve `qt_cancel` kullanılır. Tüm JSON tek metin olarak döner. Ayrıştırma ve hesaplar ön yüzde saf modeldedir (`plan-model.ts`). Görünüm (`PlanView.tsx`) yalnız QueryTab'ın tembel parçasındadır.

**Teknoloji:** Rust (`pgt-query`, `src-tauri`), SolidJS, vitest, Docker matrisi.

**Spec:** `docs/superpowers/specs/2026-10-18-visual-explain-design.md` (§1 kararlar — bağlayıcı).

## Global Constraints

- **Dal:** `feat/visual-explain` (main'den). main'e yalnız kullanıcı onayıyla birleşir.
- **Güvenlik:**
  - ANALYZE ifadeyi çalıştırır. Kapılar, sarmalanmış `EXPLAIN …` metni üzerinden mevcut `classify`/`plan_script`/`check_gates` ile uygulanır (kilitli sunucuda yazma reddi, tehlike/üretim onayı); istemciye güvenilmez.
  - Otomatik geri alma yalnız oturum `TxState::Idle` iken yapılır: `BEGIN` → `EXPLAIN` → `ROLLBACK`. EXPLAIN başarısız olsa ya da iptal edilse de `ROLLBACK` çalışır. Bağlantı koparsa transaction sunucuda zaten geri alınır.
  - Oturum `InTransaction` iken geri alma yapılmaz ve plan notu döner. `Failed` iken ret.
- **Kapsam:** tek ifade (spec §1 "Hedef ifade"); EXPLAIN'e uygunluk tablosu ve sürüm kapıları (`MERGE` PG 15+, `settings` PG 12+, `wal` PG 13+, ANALYZE'sız `buffers` PG 13+). İhlaller kodlu doğrulama hatasıdır (`explain_*`).
- **Sınırlar:** JSON en çok 32 MiB. Ön yüzde 200'den fazla düğümde ilk 3 düzey açık başlar.
- **Gizlilik:** SQL ve plan metni `tracing`'e yazılmaz; geçmiş kaydı mevcut kurala uyar.
- **Paket:** `bun run bundle:check` geçer. Ana pakete yalnız `commands.ts` sarıcısı girer ve aynı görevde geri ödenir (yalnız tembel parçaların kullandığı bir metin bloğu taşınarak). Plan modülleri yalnız QueryTab'dan içe aktarılır; ana modüllerden (`app/strings`, `explorer/*`) içe aktarmaz.
- **Erişilebilirlik:**
  - Ağaç `role="tree"`/`treeitem`, `aria-expanded`, `aria-level`. Klavyede ↑/↓/←/→/Home/End ile gezilir, Enter ayrıntıyı açar.
  - Rozetlerin metin karşılığı vardır.
  - Çubuklar `aria-hidden`; değerler ayrıca metin olarak verilir.
  - IME koruması.
- **Diğer:**
  - Yorumlar ve metinler Türkçe. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - Bindingler commit edilir; satır sonu gürültüsü atılır.
  - Crate genelinde `cargo fmt` çalıştırılmaz.
- **Docker:** yalnız `scripts/test-matrix.ps1 -Versions 10..18 -SkipPerf`. Kapsayıcılar geçicidir, başka kapsayıcılara dokunulmaz, URL/parola yazdırılmaz.

## Review Focus

- **Geri alma gerçekten çalışıyor mu:**
  - `EXPLAIN ANALYZE DELETE FROM t` sonrası satırlar duruyor mu?
  - İptal ya da hata sonrası `ROLLBACK` atılıyor mu, oturum `Idle` kalıyor mu?
  - Manuel modda açık transaction'a dokunulmuyor mu?
  - `SELECT … FOR UPDATE` ile ANALYZE kilit alıyor ve geri alma onu da bırakıyor mu?
- **Kapı atlatma:** istemci ANALYZE'ı kapalı bildirip sunucuda açık çalıştıramaz; tek doğruluk kaynağı seçeneklerdir. Sarmalanmış metin her zaman sınıflandırılır.
- **Çoklu ifade ve yorumlar:** kapsamda yorum, boş satır ya da sondaki `;` olabilir. `SELECT 1; DROP TABLE t` gibi bir seçim reddedilir: birden çok ifade vardır, sarmalanmış metne ikinci ifade girmez.
- **Ayrıştırma dayanıklılığı:** PG 10–18 JSON farkları (ör. `Async Capable`, `Disabled`, `Planning` buffers), paralel düğümler (`Workers`), CTE/InitPlan/SubPlan (`Parent Relationship`), `Never executed` (loops = 0), tetikleyiciler ve JIT. Bilinmeyen anahtarlar kaybolmaz.
- **Ölçek:** 1000+ düğümlü planda ağaç akıcı kalır (sanallaştırma ya da daraltma).

---

### Task 1: Arka uç — `qt_explain`

**Dosyalar:** `crates/pgt-query/src/{explain.rs (yeni), session.rs, classify.rs (gerekirse), lib.rs}`, `src-tauri/src/query.rs`, `src-tauri/src/main.rs`, testler (`crates/pgt-query/tests/explain.rs` Docker), bindingler.

**Arayüzler — Üretir:**

```rust
pub struct ExplainOptions { pub analyze: bool, pub buffers: bool, pub verbose: bool, pub settings: bool, pub wal: bool, pub timing: bool, pub costs: bool, pub summary: bool, pub rollback: bool }  // camelCase, TS
pub struct ExplainResult { pub json: String, pub notices: Vec<NoticeInfo>, pub elapsed_ms: u64, pub rolled_back: bool, pub sql: String, pub note: Option<String> }        // TS
pub fn explain_sql(stmt: &str, opts: &ExplainOptions, version_num: u32) -> Result<String, ExplainError>;   // saf: uygunluk + sürüm kapıları + metin
pub fn explainable(stmt: &str, version_num: u32) -> Result<(), ExplainError>;                                // saf
// QueryManager
pub async fn explain(&self, session: Uuid, sql: &str, scope: Scope, opts: ExplainOptions, confirmed: bool) -> Result<ExplainResult>;
// Tauri
#[tauri::command] async fn qt_explain(session: Uuid, sql: String, scope: Scope, options: ExplainOptions, confirmed: bool) -> Result<ExplainResult, ErrorPayload>;
```
- Akış:
  1. `scope_ranges` ile kapsam bulunur. Tam olarak bir ifade olmalı.
  2. `explain_sql` ile sarmalanmış metin üretilir.
  3. Sarmalanmış metin üzerinden `plan_script` + `check_gates` çalışır (kilit, tehlike, üretim yazımı, `confirmed`).
  4. Oturum meşgul ya da Failed ise ret.
  5. Geri alma koşulu sağlanıyorsa (`opts.analyze && kind == Write && opts.rollback && tx == Idle`) `BEGIN` → EXPLAIN → `ROLLBACK`; aksi hâlde doğrudan çalışır.
  6. Tek hücrenin metni alınır (32 MiB sınırı).
  7. NOTICE'lar toplanır.
  8. İptal mevcut `qt_cancel` ile yapılır. Oturum durum makinesi meşgul işaretini uygun biçimde tutar.
- `note`:
  - Geri alma yapıldıysa: "Değişiklikler geri alındı (sıra değerleri gibi transaction dışı etkiler kalabilir)."
  - InTransaction iken: "Etkiler açık transaction'ınızda kalır."
- `error_codes::query_payload` yeni `explain_*` kodlarını tanır.
- **Adım 1: Başarısız testler:**
  - Birim:
    - `explainable` tablosu (SELECT, WITH, VALUES, TABLE, DML, MERGE PG 14/15, EXECUTE, DECLARE, CTAS, CMVAS, SELECT INTO evet; CREATE INDEX, VACUUM, BEGIN, SET, EXPLAIN… hayır);
    - seçenek kapıları ve metin (insta);
    - çok ifadeli kapsam reddi;
    - sondaki `;`/yorum temizliği;
    - geri alma koşul tablosu.
  - Docker: Review Focus maddeleri; JSON'un `Plan` anahtarını içermesi; her sürümden bir örnek JSON `src/features/query/fixtures/plan-pg<NN>.json` olarak kaydedilir (Task 2 fixture'ları; test bir ortam değişkeniyle yazmayı tetikler ya da elle üretilip commit edilir — seçim rapora).
- **Adım 2–4:** uygula; `cargo test --workspace`, clippy, `cargo build -p pg-tools`, matris, bindingler, `bun run typecheck`.
- **Adım 5: Commit** (`feat(query): qt_explain — tek ifade EXPLAIN (FORMAT JSON), kapılar ve otomatik geri alma`).

### Task 2: Ön yüz saf modeli — ayrıştırma ve hesaplar

**Dosyalar:** `src/features/query/plan-model.ts` (+ test), `src/features/query/strings-plan.ts`, fixture'lar.

**Arayüzler — Üretir:** spec §2 `PlanNode`/`PlanSummary`, ayrıca:

```ts
export function parsePlan(json: string): { root: PlanNode; summary: PlanSummary } | { error: string };
export function hotNodes(root: PlanNode, n: number): PlanNode[];
export function initialExpanded(root: PlanNode): Set<number>;   // 200+ düğümde ilk 3 düzey
export function nodeLabel(n: PlanNode): string;                  // "Index Scan using idx on t" vb.
export function misestimate(planRows: number, actualRows: number, loops: number): { factor: number; direction: "over" | "under"; level: "ok" | "warn" | "crit" } | null;
export function looksLikePlanResult(columns: { name: string; typeName: string }[], rows: (string | null)[][]): boolean;   // ızgaradaki kullanıcı EXPLAIN'i
```
- **Adım 1: Başarısız testler:**
  - fixture'larla (PG 10/14/17/18) ayrıştırma;
  - dahil/özel süre (loops, CTE/InitPlan/SubPlan, negatif → 0);
  - sapma eşikleri;
  - `Never executed`;
  - paralel `Workers`;
  - ANALYZE'sız özel maliyet;
  - bilinmeyen anahtarların `details`'te korunması;
  - bozuk JSON → hata;
  - `looksLikePlanResult`;
  - 1000 düğümlü sentetik plan performansı (< 50 ms).
- **Adım 2–4:** uygula, doğrulamalar. **Adım 5: Commit** (`feat(query): plan ayrıştırma ve hesap modeli`).

### Task 3: Plan sekmesi, giriş noktaları ve kısayollar

**Dosyalar:** `src/features/query/PlanView.tsx` (yeni), `QueryTab.tsx` (yeni `ResultPane` "plan", explain akışı: onay modeli yeniden kullanılır), `ContextBar.tsx` (düğme + seçenek menüsü — Kobalte DropdownMenu tembel parçada serbest), `editor.ts` (F7, Shift+F7), `src/ipc/commands.ts` (`qtExplain`, ana — geri ödeme), `strings-plan.ts`, `docs/smoke-test.md`.

- **Akış:**
  - Explain çağrısı, onay modeliyle (`runWithGuards` benzeri) `confirmation_required` aldığında onay diyaloğunu gösterir ve ardından `confirmed: true` ile yeniden dener.
  - Sonuç "Plan" sekmesine düşer; notlar ve NOTICE'lar Mesajlar'a yazılır.
  - Hata Mesajlar'a gider, Plan sekmesi önceki planı korur.
- **Seçenek menüsü:** ANALYZE, BUFFERS, VERBOSE, SETTINGS, WAL, TIMING, COSTS, "yazan ifadede geri al". Sürüm altı seçenekler devre dışıdır ve gerekçe gösterilir. Seçimler sekme başına hatırlanır (oturum belleği; depolamaya yazılmaz).
- **Ağaç görünümü:** spec §1 "Görselleştirme". Seçili düğüm ayrıntı paneli, en pahalı 5 düğüm, özet ve "Ham JSON" (salt okunur `<pre>` + kopyala).
- **Izgara bağlantısı:** `looksLikePlanResult` doğruysa ızgara araç çubuğunda "Planı göster" görünür ve hücre metnini Plan sekmesinde açar.
- **Adım 1: Başarısız testler:**
  - explain akış modeli (saf denetleyici ya da QueryTab yardımcıları): onay ve yeniden deneme, meşgul, hata ve plan korunması;
  - seçenek kapıları;
  - kısayol eşlemesi (editor keymap testi varsa).
- **Adım 2–4:** uygula, doğrulamalar, `bundle:check` (geri ödeme).
- **Adım 5:** smoke maddeleri:
  - F7 / Shift+F7;
  - ANALYZE DELETE geri alındı notu;
  - manuel modda açık transaction notu;
  - kilitli sunucu;
  - büyük plan;
  - ızgaradaki kendi EXPLAIN'iniz;
  - klavye ağacı;
  - tema.

  **Commit** (`feat(query): "Plan" sekmesi, F7/Shift+F7 ve plan ağacı görünümü`).

### Task 4: Belgeler ve bütünleşik doğrulama

- Spec durumu ve uygulama notları; `docs/testing.md`.
- Tam matris, `cargo test --workspace`, clippy, `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`.
- Kapanış taraması: `rg "strings-plan" src` → yalnız Query Tool'un tembel dosyaları.
- **Commit** (`docs(query): Plan 28 smoke ve notlar`).

## Öz inceleme

- **Spec §1 kapsamı:**
  - Hedef ifade, seçenekler, güvenlik/geri alma, yürütme → T1.
  - Ayrıştırma, hesaplar → T2.
  - Görselleştirme, giriş noktaları → T3.
  - Paket → Global Constraints + T3.
- **Tip tutarlılığı:** `ExplainResult` (T1) → `parsePlan` (T2) → `PlanView` (T3); fixture'lar T1'den T2'ye geçer.
