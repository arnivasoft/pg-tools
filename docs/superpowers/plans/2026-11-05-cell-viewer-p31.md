# Hücre Görüntüleyici — Biçim Sekmeleri (Plan 31) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Query Tool hücre paneline biçim sekmeleri: **Metin**, **Onaltılık**, **Görsel**, **JSON**, **HTML**, **PDF**. JSON ve HTML biçimlendirilir ve sözdizimi renklendirilir; HTML korumalı iframe'de önizlenebilir; PDF varsayılan uygulamada açılır ya da kaydedilir. Sekme sütun türü ve içerikten kendiliğinden seçilir, elle seçim sütun başına hatırlanır.

**Mimari:**
- `CellPanel` zaten tembel QueryTab parçasındadır (`QueryTabs.tsx:23`). Sekme çubuğu, saf model (`cell-view-model.ts`) ve metinler (`strings-cell.ts`) o parçaya eklenir; ana pakete girmez.
- Metin dışındaki görünümler yeni tembel parçadadır: `CellViewers.tsx` (Onaltılık, Görsel, JSON/HTML kabı, PDF) + `cell-format.ts` (saf çözme/biçimleme) + `cell-code.ts` (salt okunur CodeMirror) + `cell-ipc.ts`. `@codemirror/lang-json` ve `@codemirror/lang-html` yalnız `cell-code.ts`'teki dinamik `import()` ile, ayrı parçalarda yüklenir.
- Tek arka uç ekleme: `src-tauri/src/cell.rs` — `cell_open_pdf` (geçici dosya + varsayılan uygulama) ve `cell_save_bytes` (Kaydet…); geçici dosyaların açılış/kapanış temizliği.
- Ana pakete tek ekleme: `commands.ts`'teki `call` yardımcısının dışa aktarılması (sarıcılar `commands`'a eklenmez). Bu bayt Task 7'de telafi edilir.

**Teknoloji:** SolidJS, `@tanstack/solid-virtual` (mevcut), CodeMirror 6 (mevcut + iki yeni dil paketi), `@tauri-apps/plugin-dialog` (mevcut; `save`), vitest (`node` ortamı); Rust yalnız `src-tauri/src/cell.rs` + `main.rs`.

**Spec:** `docs/superpowers/specs/2026-11-05-cell-viewer-design.md`. Bağlayıcı olanlar: §2.1 uygulanabilirlik tablosu ve seçim önceliği, §2.2 görünümler (iframe ve CSP meta kuralları), §2.3 biçimleyici kuralları ve bağımlılık sürümleri, §2.4 komutlar ve geçici dosya yaşam döngüsü, §2.5 sınır tablosu, §2.8 spec düzeltmeleri. Paket kuralları: `docs/testing.md` "Ana paket bütçesi". Emsal: Plan 27 (`docs/superpowers/plans/2026-10-31-backup-p27-on-yuz.md`; `backup_reveal`, `picker.ts`, telafi görevi).

## Kararlar

1. **Sekmeler ve sıra:** `text`, `hex`, `image`, `json`, `html`, `pdf` (etiketler "Metin", "Onaltılık", "Görsel", "JSON", "HTML", "PDF"). Hepsi her zaman görünür; uygulanamayanlar `aria-disabled` + gerekçe (spec §2.1). Değer NULL ise yalnız sekme çubuğu devre dışı görünür, gövde bugünkü "NULL" metnidir.
2. **Kendiliğinden seçim ve hatırlama:**
   - Öncelik: hatırlanan (uygulanabiliyorsa) → `json`/`jsonb` → JSON; `bytea` + görsel imzası → Görsel; `bytea` + `%PDF-` → PDF → Metin.
   - Anahtar `${typeOid}:${name}`; modül düzeyi `Map` (QueryTab parçası, uygulama oturumu boyunca). `localStorage` yok.
   - Elle seçim yalnız kullanıcı bir sekmeye tıkladığında ya da Enter/Space ile seçtiğinde yazılır; kendiliğinden seçim hafızaya yazılmaz.
3. **Algılama yalnız önekle:** `bytea` değerinde ilk 1 KiB çözülür (`decodePrefix`). Tam çözme yalnız görünüm açıldığında tembel parçada yapılır. JSON uygunluğu: `json`/`jsonb` türünde her zaman; diğer türde kırpılmış değer `{`/`[` ile başlıyorsa ve uzunluk ≤ 4 MiB ise `JSON.parse` denenir; > 4 MiB ise yalnız ilk karakter yeter (görünüm sınır iletisini gösterir).
4. **Sınırlar** (`LIMITS`; spec §2.5): `prefixBytes` 1 KiB, `hexBytes` 16 MiB, `imageBytes` 20 MiB, `codeChars` 4 MiB, `previewChars` 2 MiB, `fileBytes` 64 MiB. Sınır her zaman ham uzunluktan, çözmeden **önce** denetlenir.
5. **JSON görünümü:** `reindentJson` (mevcut, kayıpsız) + salt okunur CodeMirror (`lang-json`, `foldGutter`). Ayrı ağaç bileşeni yok (spec düzeltmesi 3).
6. **HTML görünümü:** "Kaynak" (varsayılan; `formatHtml` + `lang-html` + katlama; "Biçimlendir" anahtarı varsayılan açık) ve "Önizleme" anahtarı. Önizleme: `<iframe sandbox="" referrerpolicy="no-referrer" title=… srcdoc={previewDoc(özgün)}>`, beyaz arka plan. `sandbox` özniteliği **boş dizgidir**; hiçbir `allow-*` eklenmez. CSP (`tauri.conf.json`) değişmez; WebView2 önizlemeyi engellerse uygulayıcı durur ve sorar.
7. **Görsel:** `data:` URL ile `<img>`; sığdır/"Gerçek boyut"; boyut bilgisi `onLoad`'da; `onError` iletisi; "Kaydet…" (spec düzeltmesi 4). `blob:` URL kullanılmaz (CSP `img-src` izin vermez).
8. **PDF:** bilgi satırı + "Varsayılan uygulamada aç" + "Kaydet…" + uyarı satırı. Düğmeler çalışırken devre dışıdır ("Açılıyor…" / "Kaydediliyor…"). Hata bildirimi sabit metindir (yol ve değer içermez).
9. **Kaydet…:** `save({ defaultPath: suggestSaveName(sütun, uzantı), filters: [{ name, extensions: [uzantı] }], title })` → `cellIpc.saveBytes(ham, yol)`. İptal (`null`) sessizdir. Başarıda "Kaydedildi." bildirimi (yolsuz).
10. **Arka uç** (spec §2.4): `async` komutlar (UI iş parçacığını bloklamaz); `decode_bytea_hex`; PDF yeniden doğrulaması; `app_cache_dir()/cell-view/cell-<uuid>.pdf`, `create_new`; `explorer.exe` tam yoldan, `raw_arg` ile tırnaklı yol; yönetilen `CellTemp`; `RunEvent::Exit`'te kayıtlı dosyalar silinir; `setup`'ta 24 saatten eski `cell-*.pdf` süpürülür. Hata kodları `invalid`, `too_large`, `io`. Hiçbir şey günlüğe yazılmaz.
11. **Panel genişliği:** QueryTab'de `cellWide` sinyali; `w-72` ↔ `w-[36rem]`. Başlıktaki düğme `aria-pressed`.
12. **Kapsam dışı:** veri düzenleyici, gömülü PDF, bytea içi UTF-8 metni JSON/HTML olarak açma, base64 metin görselleri, sürüklenerek panel boyutlandırma.

- **Kullanıcı onayı (2026-11-05):** sekme seti, kendiliğinden seçim + sütun başına hatırlama, `sandbox=""` önizleme, PDF'nin dışarıda açılması; ana paket tavanı yükseltilmez (yetmezse durulur, sorulur).

## Global Constraints

- **Dal:** `feat/cell-viewer` (main'den). main'e birleştirme yalnız kullanıcı onayıyla.
- **Değişen alanlar:**
  - Ön yüz (`src/`), `package.json`, `bun.lock`, `docs/`, `scripts/bundle-budget.json` (yalnız tavanı **indirmek** için).
  - Rust'ta **yalnız** `src-tauri/src/cell.rs` (yeni) ve `src-tauri/src/main.rs`. Rust testleri: `cargo test -p pg-tools cell`, `cargo clippy --workspace --all-targets -- -D warnings`.
  - `src-tauri/tauri.conf.json` (CSP) ve `src-tauri/capabilities/*` **değişmez**.
  - Crate genelinde `cargo fmt` çalıştırılmaz; dokunulan satırlar elle biçimlendirilir.
- **Sözleşme:** komut adları `cell_open_pdf { data }` ve `cell_save_bytes { data, path }` (camelCase bağımsız değişkenler); `data` hücrenin ham `\x…` metnidir. Arka uç tek doğruluk kaynağıdır: ön yüz denetimleri (PDF imzası, boyut) yalnız düğme durumları içindir.
- **Paket:**
  - Ana paket ≤ `mainMaxBytes` (şu an 127,364 B), açılış kümesi ≤ `startupMaxBytes` (155,037 B). Preload tam `["solid","web"]`; açılış kümesinde yasak paket yok.
  - Tavan **asla yükseltilmez** (kullanıcı kararı). Task 1–6'da `bun run bundle:check` yalnız bu iki tavanın aşımı yüzünden düşebilir; aşım rapora yazılır. Başka her ihlal (preload, yasak paket, harita) görevi durdurur. Telafi Task 7'dedir.
  - `@codemirror/lang-json`, `@codemirror/lang-html`, `@lezer/json`, `@lezer/html`, `@lezer/css`, `@lezer/javascript`, `@codemirror/lang-css`, `@codemirror/lang-javascript` ana pakete ve QueryTab parçasına girmez; yalnız `cell-code.ts`'in dinamik `import()`'larıyla yüklenir.
  - `CellViewers.tsx` yalnız `CellPanel`'deki `lazy(() => import("./CellViewers"))` ile yüklenir; QueryTab parçası `cell-format.ts`, `cell-code.ts`, `cell-ipc.ts`'i statik içe aktarmaz.
- **Bağımlılıklar:** `"@codemirror/lang-json": "6.0.2"`, `"@codemirror/lang-html": "6.4.12"` (tam sürüm, `^` yok). Başka yeni paket yok (HTML biçimleyici el yazımı; `hex`/`base64` crate'i yok).
- **Gizlilik:**
  - Hücre değeri, çözülen baytlar, biçimlenmiş metin, geçici yol ve kayıt yolu `console.*`'a, `localStorage`'a, bildirime ve `tracing`'e yazılmaz.
  - Geçici PDF yalnız `app_cache_dir()/cell-view` altında, benzersiz adla; kapanışta silinir.
- **Güvenlik:**
  - HTML hiçbir koşulda ana belgede çizilmez: `innerHTML`, `DOMParser`, `document.write`, `eval`, `new Function`, `insertAdjacentHTML` yeni kodda yasaktır (Task 6 taramasıyla denetlenir).
  - iframe `sandbox=""` sabittir; `allow-*` yok.
  - Varsayılan uygulamada yalnız `%PDF-` ile başlayan baytlar açılır (arka uç doğrular).
- **Erişilebilirlik:** sekme çubuğu `role="tablist"`/`role="tab"`/`aria-selected`/gezici `tabindex`, Sol/Sağ/Home/End; devre dışı sekme `aria-disabled` + `title` + `sr-only` gerekçe; görünüm `role="tabpanel"` + `aria-labelledby`.
- **vitest:** ortam `node`. Mantık `cell-view-model.ts`, `cell-format.ts`, `cell-code.ts` (yalnız dil yükleyici) dosyalarındadır ve orada test edilir. GUI ajanlarca test edilemez; elle kontrol maddeleri `docs/smoke-test.md` "## Hücre görüntüleyici (Plan 31)" altına `[—]` olarak eklenir (Task 7).
- **Diğer:**
  - Yorumlar ve metinler Türkçe, doğru Türkçe karakterlerle. Teknik terimler (`bytea`, `srcdoc`, `sandbox`) olduğu gibi kalır.
  - Bindingler değişmez (ts-rs türü yok). Değişmiş görünürse yalnız satır sonudur: `git checkout -- src/ipc/bindings`.
  - `graphify-out/` commit edilmez.
  - Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Doğrulama (her görev sonu):** `~/.bun/bin/bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`; Rust'a dokunan görevde ayrıca `cargo test -p pg-tools cell` ve `cargo clippy --workspace --all-targets -- -D warnings`.

## Review Focus

- **Yanlış sekme seçimi ve hafıza:** `jsonb` hücresi; PNG imzalı `bytea`; imzasız `bytea`; `{"a":1}` içeren `text`; aynı sütunda önce PNG sonra imzasız satır; kullanıcı PNG sütununda Onaltılık'ı seçip başka satıra geçer.
  - Beklenen: sırasıyla JSON, Görsel, Metin, Metin (JSON etkin); imzasız satırda Görsel devre dışı → kendiliğinden Metin ama hafıza `image` kalır; elle seçilen Onaltılık sonraki satırlarda korunur. Kendiliğinden seçim hafızaya yazılmaz.
  - Test: Task 2 `pickFormat`/`availability`/`createFormatMemory`.
- **Büyük değerler:** 25 MB PNG, 70 MB `bytea`, 5 MB JSON, 3 MB HTML, 65 MB PDF.
  - Beklenen: Görsel/JSON/HTML görünümlerinde sınır iletisi (çözme yapılmadan); Onaltılık ilk 16 MiB + not; HTML önizleme anahtarı devre dışı; PDF düğmeleri baştan devre dışı + gerekçe; arka uç da `too_large` döner. Hücre seçimi algılaması yalnız 1 KiB önek çözer.
  - Test: Task 2 `sizeVerdict`/`decodePrefix`, Task 3 `decodeByteaHex(…, max)`, Task 1 `too_large`.
- **HTML güvenliği:** `<script>`, `<img src=x onerror=…>`, `<a target=_top>`, `<meta http-equiv=refresh>`, tırnaklı öznitelikte `>`, kapanmamış yorum, `<!DOCTYPE html>` ile başlayan ve başlamayan belge.
  - Beklenen: biçimleyici yalnız metin üretir, betik/stil içeriğini olduğu gibi korur, kapanmamış yapıda takılmaz; `previewDoc` CSP meta'yı doctype'tan sonra (yoksa başa) ekler; iframe `sandbox=""`. Yasak API taraması temiz.
  - Test: Task 3 `formatHtml`/`previewDoc`; Task 6 Adım 4 taraması; smoke maddeleri (IPC erişimi, uzak istek).
- **Geçici PDF ve yol güvenliği:** PDF olmayan veri, bozuk onaltılık, `"` içeren yol, göreli yol, aynı anda iki uygulama örneği, okuyucunun dosyayı kilitlemesi.
  - Beklenen: PDF dışı veri `invalid`; geçici adlar benzersiz ve `create_new`; kapanış yalnız bu sürecin dosyalarını siler; açılış süpürmesi yalnız 24 saatten eski `cell-*.pdf`'lere dokunur; silme hatası sessizdir.
  - Test: Task 1 Rust testleri (`decode_bytea_hex`, `is_pdf`, `path_arg`, `sweep_stale`, `CellTemp::remove_all`).
- **Bayt doğruluğu:** `\x` boş değer, büyük harfli onaltılık, tek uzunluk, geçersiz karakter, `text` sütununda `ş`, 17 baytlık değer, base64 sınır uzunlukları (0/1/2/3 bayt ve 32 KiB parça sınırı).
  - Beklenen: boş değer 0 bayt; bozuk değer `null` (görünüm "bytea çözülemedi"); `ş` → `C5 9F`; son satır boşlukla hizalanır; base64 Node `Buffer` sonucu ile birebir.
  - Test: Task 3 `decodeByteaHex`/`utf8Bytes`/`hexRow`/`bytesToBase64`.

---

### Task 1: Arka uç (`cell.rs`) ve IPC yüzeyi

**Dosyalar:**
- Oluştur: `src-tauri/src/cell.rs` (komutlar + saf yardımcılar + birim testleri).
- Değiştir: `src-tauri/src/main.rs` (`mod cell;`, `generate_handler!` kaydı, `setup`'ta `cell::setup(app)`, `RunEvent::Exit`'te `cell::on_exit(app)`).
- Değiştir: `src/ipc/commands.ts` (yalnız `async function call` → `export async function call`; gövde değişmez).
- Oluştur: `src/features/query/cell-ipc.ts`.

**Arayüzler — Üretir:**

```rust
// src-tauri/src/cell.rs
type CmdResult<T> = Result<T, ErrorPayload>;
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
pub fn decoded_len(text: &str) -> Option<usize>;                       // "\\x" önekli değilse None; aksi (len-2)/2
pub fn decode_bytea_hex(text: &str) -> Result<Vec<u8>, &'static str>;  // önek, çift uzunluk, [0-9a-fA-F]
pub fn is_pdf(bytes: &[u8]) -> bool;                                   // bytes.starts_with(b"%PDF-")
pub fn path_arg(path: &str) -> Result<String, &'static str>;           // Ok("\"C:\\x\\y.pdf\"")
pub fn temp_file_name(id: uuid::Uuid) -> String;                       // "cell-<uuid>.pdf"
pub fn sweep_stale(dir: &std::path::Path, now: std::time::SystemTime, max_age: std::time::Duration) -> usize;
#[derive(Default)] pub struct CellTemp { files: std::sync::Mutex<Vec<std::path::PathBuf>> }
impl CellTemp {
    pub fn remember(&self, path: std::path::PathBuf);
    pub fn remove_all(&self) -> usize;                                 // silinen sayısı; hata sessiz
}
pub fn setup(app: &tauri::App);                                        // manage(CellTemp) + sweep_stale(cell-view)
pub fn on_exit(app: &tauri::AppHandle);                                // state::<CellTemp>().remove_all()
#[tauri::command] pub async fn cell_open_pdf(app: tauri::AppHandle, temp: tauri::State<'_, CellTemp>, data: String) -> CmdResult<()>;
#[tauri::command] pub async fn cell_save_bytes(data: String, path: String) -> CmdResult<()>;
```
```ts
// src/ipc/commands.ts — tek değişiklik
export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
// src/features/query/cell-ipc.ts (yalnız tembel CellViewers içe aktarır)
export const cellIpc = {
  openPdf: (data: string) => call<null>("cell_open_pdf", { data }),
  saveBytes: (data: string, path: string) => call<null>("cell_save_bytes", { data, path }),
};
export type CellIpc = typeof cellIpc;
```

- **İletiler (sabit):** bozuk onaltılık "Değer çözülemedi (geçersiz bytea).", PDF değil "Değer bir PDF belgesi değil.", büyük "Değer dosya işlemi için çok büyük (sınır 64 MB).", yol `path_arg` iletileri ("Yol geçersiz karakter içeriyor.", "Tam (mutlak) yol gerekir."), yazma "Dosya yazılamadı.", açma "Varsayılan uygulama başlatılamadı.", Windows dışı "Bu işlem yalnız Windows'ta desteklenir.".
- **`cell_open_pdf` sırası:** `decoded_len` > `MAX_FILE_BYTES` → `too_large` (çözmeden) → `decode_bytea_hex` → `is_pdf` → dizin oluştur → `OpenOptions::new().write(true).create_new(true)` → yaz → `remember` → `explorer.exe` (`%SystemRoot%`'tan, `raw_arg(path_arg(..))`, `spawn`, beklenmez).
- **`cell_save_bytes` sırası:** `path_arg` (yalnız doğrulama) → boyut → çözme → `std::fs::write` (üzerine yazma; onayı yerel diyalog verdi).
- **`sweep_stale`:** yalnız adı `cell-` ile başlayıp `.pdf` ile biten **dosyaları**, `modified()` + `max_age` < `now` ise siler; dizin yoksa 0.

- [ ] **Adım 1: Başarısız testler (`cell.rs` `#[cfg(test)]`).**
  - `decode_bytea_hex("\\x")` → `Ok(vec![])`; `"\\x89504e47"` ve `"\\x89504E47"` → `[0x89, 0x50, 0x4E, 0x47]`; `"\\x123"` → `Err`; `"\\xzz"` → `Err`; `"89504e47"` (öneksiz) → `Err`.
  - `decoded_len("\\x0a0b")` → `Some(2)`; `decoded_len("abc")` → `None`.
  - `is_pdf(b"%PDF-1.7\n")` → true; `is_pdf(b" %PDF-")` → false; `is_pdf(b"")` → false.
  - `path_arg("C:\\Temp\\ş ı.pdf")` → `Ok("\"C:\\Temp\\ş ı.pdf\"")` (`#[cfg(windows)]`); göreli yol → `Err("Tam (mutlak) yol gerekir.")` (`#[cfg(windows)]`); `"` ya da `\n` → `Err("Yol geçersiz karakter içeriyor.")`.
  - `temp_file_name(Uuid::nil())` → `"cell-00000000-0000-0000-0000-000000000000.pdf"`.
  - `sweep_stale` (`tempfile::tempdir`): `cell-a.pdf` (eski), `cell-b.pdf` (yeni), `other.pdf` (eski), `cell-c.txt` (eski), `cell-d.pdf/` dizini → yalnız `cell-a.pdf` silinir, dönüş 1. "Eski" `now` parametresi `SystemTime::now() + 25 sa` verilerek sağlanır (dosya zamanı değiştirilmez). Var olmayan dizin → 0.
  - `CellTemp`: iki dosya `remember` edilir, biri elle silinmiş → `remove_all()` == 1, ikinci çağrı 0, kayıtlı olmayan komşu dosya durur.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `cargo test -p pg-tools cell` → derleme hatası (`cell` modülü yok).
- [ ] **Adım 3: Uygula.** Kararlar 10. `cell_open_pdf`/`cell_save_bytes` gövdeleri mantığı saf yardımcılara bırakır; `#[cfg(not(windows))]` dalında açma `invalid` döner, kaydetme çalışır. `tracing` çağrısı eklenmez. `cell-ipc.ts` yalnız tür ve iki sarıcıdır.
- [ ] **Adım 4: Doğrula.** `cargo test -p pg-tools cell` PASS; `cargo clippy --workspace --all-targets -- -D warnings` temiz. Dört ön yüz komutu; `bundle:check` ana paket baytı (tavana göre fark) rapora yazılır. `git status`'ta `src/ipc/bindings` değişmişse satır sonudur: `git checkout -- src/ipc/bindings`.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src-tauri/src/cell.rs src-tauri/src/main.rs src/ipc/commands.ts src/features/query/cell-ipc.ts
  git commit -m "feat(cell): PDF'yi varsayılan uygulamada açma ve bayt kaydetme komutları, geçici dosya temizliği"
  ```

### Task 2: Saf model ve metinler

**Dosyalar:** Oluştur `src/features/query/cell-view-model.ts`, `src/features/query/cell-view-model.test.ts`, `src/features/query/strings-cell.ts` (`export const tcell = { … } as const`; yalnız QueryTab parçası ve `CellViewers` içe aktarır).

**Arayüzler — Üretir:**

```ts
import type { Column } from "../../ipc/bindings/Column";
export type CellFormat = "text" | "hex" | "image" | "json" | "html" | "pdf";
export const CELL_FORMATS: readonly CellFormat[];          // ["text","hex","image","json","html","pdf"]
export type ImageKind = "png" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg";
export const MiB = 1024 * 1024;
export const LIMITS: { readonly prefixBytes: 1024; readonly hexBytes: number; readonly imageBytes: number; readonly codeChars: number; readonly previewChars: number; readonly fileBytes: number };
//                     1024, 16 MiB, 20 MiB, 4 MiB, 2 MiB, 64 MiB
export function isByteaHex(value: string, typeName: string): boolean;   // typeName === "bytea" && value.startsWith("\\x")
export function byteaLength(value: string): number;                     // (value.length - 2) >> 1
export function decodePrefix(value: string, maxBytes: number): Uint8Array | null;   // geçersiz onaltılık → null
export function sniffImage(bytes: Uint8Array): ImageKind | null;        // svg dahil (sniffSvg)
export function sniffSvg(text: string): boolean;
export function isPdf(bytes: Uint8Array): boolean;
export function looksLikeJson(value: string): boolean;                  // Kararlar 3
export function looksLikeMarkup(value: string): boolean;               // kırpılmış: "<" ile başlar ve ">" içerir
export interface CellFacts {
  isNull: boolean; bytea: boolean; jsonType: boolean;
  byteLength: number;   // bytea: byteaLength; diğer: UTF-8 bayt uzunluğu (TextEncoder değil, hızlı sayım)
  charLength: number;
  image: ImageKind | null; pdf: boolean; json: boolean; markup: boolean; badHex: boolean;
}
export function cellFacts(value: string | null, column: Pick<Column, "typeName">): CellFacts;
export type Availability = { enabled: boolean; reason: string | null };
export function availability(f: CellFacts): Record<CellFormat, Availability>;
export function autoFormat(f: CellFacts): CellFormat;
export function pickFormat(f: CellFacts, remembered: CellFormat | undefined): CellFormat;
export function columnKey(c: Pick<Column, "name" | "typeOid">): string;   // `${typeOid}:${name}`
export interface FormatMemory { get(key: string): CellFormat | undefined; set(key: string, f: CellFormat): void }
export function createFormatMemory(): FormatMemory;
export const formatMemory: FormatMemory;                                  // oturum boyu tekil
export type SizeVerdict = { ok: true } | { ok: false; size: number; limit: number };
export function sizeVerdict(format: Exclude<CellFormat, "text">, f: CellFacts): SizeVerdict;
export function previewAllowed(f: CellFacts): Availability;              // charLength ≤ previewChars
export function fileActionAllowed(f: CellFacts): Availability;           // bytea && byteLength ≤ fileBytes
export function nextTab(current: CellFormat, key: "ArrowLeft" | "ArrowRight" | "Home" | "End"): CellFormat;   // devre dışı dahil, dairesel
export function formatMb(bytes: number): string;                          // 1.5 MiB → "1,5"; tr-TR, en çok 1 ondalık
```

- **`tcell` metinleri:**
  - `tabs`: `{ text: "Metin", hex: "Onaltılık", image: "Görsel", json: "JSON", html: "HTML", pdf: "PDF" }`, `ariaLabel: "Hücre görünümü"`.
  - `reason`: `isNull: "Değer NULL."`, `notImage: "Tanınan bir görsel biçimi değil (PNG, JPEG, GIF, WebP, BMP, ICO, SVG)."`, `notJson: "Geçerli JSON değil."`, `notMarkup: "HTML/XML işaretlemesi değil."`, `notPdf: "PDF değil (%PDF- ile başlamıyor)."`, `onlyBytea: "Yalnız bytea sütunlarında."`, `onlyText: "Yalnız metin sütunlarında."`, `badHex: "bytea değeri çözülemedi."`.
  - `tooLarge: (size: string, limit: string) => \`Değer bu görünüm için çok büyük (${size} MB; sınır ${limit} MB). Metin ya da Onaltılık sekmesini kullanın ya da Kaydet… ile dosyaya yazın.\``; `tooLargeText` (aynı, "ya da Kaydet…" bölümü yok; bytea dışı türler için).
  - `hex`: `ariaLabel: "Onaltılık görünüm"`, `total: (n: number) => \`${n.toLocaleString("tr-TR")} bayt\``, `partial: (shown: string, total: string) => \`Yalnız ilk ${shown} MB gösteriliyor (toplam ${total} MB).\``.
  - `image`: `fit: "Sığdır"`, `actual: "Gerçek boyut"`, `alt: (kind: string, w: number, h: number) => \`Hücre görseli (${kind}, ${w} × ${h})\``, `broken: "Görsel çözülemedi (bozuk ya da desteklenmeyen veri)."`.
  - `html`: `source: "Kaynak"`, `preview: "Önizleme"`, `format: "Biçimlendir"`, `frameTitle: "HTML önizlemesi (korumalı)"`, `previewNote: "Betikler çalışmaz; uzak görsel ve stiller yüklenmez."`, `previewTooLarge: (limit: string) => \`Önizleme için çok büyük (sınır ${limit} MB).\``.
  - `pdf`: `info: (version: string | null, bytes: string) => …` ("PDF 1.7 · 12.345 bayt"; sürüm yoksa "PDF · …"), `open: "Varsayılan uygulamada aç"`, `opening: "Açılıyor…"`, `warn: "PDF dışarıda açılır; güvenmediğiniz belgeleri açmayın."`.
  - `save: "Kaydet…"`, `saving: "Kaydediliyor…"`, `saved: "Kaydedildi."`, `saveTitle: "Hücre değerini kaydet"`, `openFailed: "PDF açılamadı."`, `saveFailed: "Dosya kaydedilemedi."`, `loading: "Görünüm yükleniyor…"`, `loadFailed: "Görünüm yüklenemedi."`, `widen: "Paneli genişlet"`, `narrow: "Paneli daralt"`.
- **Gerekçe eşlemesi (`availability`):** NULL → tüm sekmeler `isNull`; `text`/`hex` her zaman etkin; `image` → `badHex` ise `badHex`, değilse `notImage`; `json` → `notJson`; `html` → bytea ise `onlyBytea` değil "Yalnız metin sütunlarında." (`tcell.reason.onlyText`), değilse `notMarkup`; `pdf` → bytea değilse `onlyBytea`, `badHex` ise `badHex`, değilse `notPdf`.

- [ ] **Adım 1: Başarısız testler (`cell-view-model.test.ts`).**
  - `sniffImage`: PNG/JPEG/GIF87a/GIF89a/WebP/BMP/ICO imzalı baytlar doğru tür; `RIFF????WAVE` → null; 2 baytlık dizi → null (taşma yok).
  - `sniffSvg`: `"<svg …"`, `"\uFEFF  <?xml version=\"1.0\"?>\n<!-- c --><!DOCTYPE svg><SVG"` → true; `"<html>"`, `"svg"` → false.
  - `decodePrefix("\\x89504e47", 2)` → `[0x89, 0x50]`; `"\\x8"` → null; `"\\xgg"` → null.
  - `cellFacts`:
    - `bytea` PNG → `{ bytea: true, image: "png", pdf: false, byteLength: N }`.
    - `bytea` `\x255044462d312e37` → `pdf: true`.
    - `bytea` `\xzz` → `badHex: true`, `image: null`.
    - `text` `"ş"` → `byteLength: 2`, `charLength: 1`.
    - `jsonb` `"[1,2]"` → `jsonType: true, json: true`; `text` `" {\"a\":1}"` → `json: true`; `text` `"42"` → `json: false`; `text` `"{bozuk"` → `json: false`.
    - `text` `"<p>x</p>"` → `markup: true`; `text` `"<svg …></svg>"` → `markup: true` ve `image: "svg"`.
    - `null` → `isNull: true`.
  - `availability`: yukarıdaki her durum için etkinlik ve gerekçe metni (ör. `text` sütununda PDF → `tcell.reason.onlyBytea`).
  - `autoFormat`: `jsonb` → `json`; `bytea` PNG → `image`; `bytea` PDF → `pdf`; `bytea` imzasız → `text`; `text` `{"a":1}` → `text`; `text` `<p>` → `text`.
  - `pickFormat`: hatırlanan `hex` → `hex`; hatırlanan `image` + imzasız bytea → `text` (kendiliğinden); hatırlanan `undefined` → `autoFormat`.
  - `createFormatMemory`: `set`/`get`; iki bellek örneği bağımsız; `columnKey({ name: "foto", typeOid: 17 })` → `"17:foto"`.
  - `sizeVerdict`: `image` 20 MiB → ok, 20 MiB + 1 → `{ ok: false, size, limit: 20 MiB }`; `json` 4 MiB + 1 karakter → reddedilir; `hex` her boyutta ok (kısmi gösterim görünümün işidir); `pdf` 64 MiB + 1 → reddedilir.
  - `previewAllowed`/`fileActionAllowed` sınır ve gerekçeleri; `fileActionAllowed` `text` sütununda `onlyBytea`.
  - `nextTab`: `text` + ArrowLeft → `pdf`; `pdf` + ArrowRight → `text`; Home → `text`; End → `pdf`.
  - `formatMb(1.5 * MiB)` → `"1,5"`; `formatMb(20 * MiB)` → `"20"`.
  - Metin sabitleri: `tcell.tabs.hex === "Onaltılık"`, `tcell.reason.notPdf` beklenen metin.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bun run test -- src/features/query/cell-view-model.test.ts` → FAIL (modül yok).
- [ ] **Adım 3: Uygula.** Kararlar 2–4. `cellFacts` bytea için yalnız `decodePrefix(value, LIMITS.prefixBytes)` çağırır; JSON denemesi Kararlar 3'e uyar. UTF-8 bayt sayımı kod birimi döngüsüyle yapılır (kopya üretmez).
- [ ] **Adım 4: Doğrula.** Dört ön yüz komutu. `rg "strings-cell|cell-view-model" src/app src/features/explorer src/ipc` → sonuç yok.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/query/cell-view-model.ts src/features/query/cell-view-model.test.ts src/features/query/strings-cell.ts
  git commit -m "feat(cell): biçim sekmesi modeli (algılama, uygunluk, kendiliğinden seçim, sınırlar) ve metinler"
  ```

### Task 3: Saf biçimleyiciler (`cell-format.ts`)

**Dosyalar:** Oluştur `src/features/query/cell-format.ts`, `src/features/query/cell-format.test.ts`.

**Arayüzler — Üretir** (yalnız tembel `CellViewers` içe aktarır):

```ts
import type { ImageKind } from "./cell-view-model";
export function decodeByteaHex(value: string, maxBytes?: number): Uint8Array | null;   // en çok maxBytes bayt; bozuk → null
export function utf8Bytes(text: string, maxBytes?: number): Uint8Array;               // TextEncoder; maxBytes'ta kesilir
export const HEX_ROW = 16;
export function hexRowCount(length: number): number;                                   // Math.ceil(length / 16)
export function hexRow(bytes: Uint8Array, row: number): string;
//   "00000010  49 48 44 52 00 00 02 80  00 00 01 E0 08 06 00 00  |IHDR............|"
//   son satır: eksik baytlar "   " ile, ASCII sütunu kısa kalır ("|ab|")
export function bytesToBase64(bytes: Uint8Array): string;                              // 0x8000'lik parçalarla btoa
export function imageMime(kind: ImageKind): string;                                    // svg → "image/svg+xml", ico → "image/x-icon", jpeg → "image/jpeg" …
export function dataUrl(kind: ImageKind, bytes: Uint8Array): string;
export function pdfVersion(bytes: Uint8Array): string | null;                          // "%PDF-1.7" → "1.7"
export function formatHtml(text: string): string;                                      // spec §2.3
export const PREVIEW_CSP: string;   // "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:"
export function previewDoc(html: string): string;
export function suggestSaveName(columnName: string, ext: string): string;              // Windows'ta geçersiz <>:"/\|?* ve denetim karakteri → "_"; boşsa "hucre"; 100 karaktere kırp
export function saveExt(f: { pdf: boolean; image: ImageKind | null }): string;         // "pdf" | görsel türü ("jpeg" → "jpg") | "bin"
```

- [ ] **Adım 1: Başarısız testler (`cell-format.test.ts`).**
  - `decodeByteaHex("\\x")` → boş; `"\\x0A0b"` → `[10, 11]`; `"\\x0a0"` → null; `"\\x0a0g"` → null; `("\\x00010203", 2)` → `[0, 1]`.
  - `utf8Bytes("ş")` → `[0xC5, 0x9F]`; `utf8Bytes("abc", 2)` → `[0x61, 0x62]`.
  - `hexRow`: 17 baytlık dizinin 0. satırı tam biçim (iki boşluklu grup ayracı, büyük harf); 1. satır `"00000010  "` + tek bayt + boşluk dolgusu + `"|x|"`; 0x20 altı ve 0x7E üstü ASCII'de `.`; `hexRowCount(0) === 0`, `hexRowCount(16) === 1`, `hexRowCount(17) === 2`.
  - `bytesToBase64`: 0, 1, 2, 3 bayt ve 0x8000 + 1 baytlık rastgele dizi için `Buffer.from(b).toString("base64")` ile eşit.
  - `dataUrl("png", …)` `data:image/png;base64,` ile başlar; `imageMime("svg") === "image/svg+xml"`.
  - `pdfVersion`: `%PDF-1.7\n` → `"1.7"`; `%PDF-2.0` → `"2.0"`; `%PDF-` → null.
  - `formatHtml`:
    - `"<div><p>a</p><br><img src=x></div>"` → `"<div>\n  <p>\n    a\n  </p>\n  <br>\n  <img src=x>\n</div>"`.
    - `<pre>  a\n b</pre>`, `<script>if (a<b) x()</script>`, `<style>p>a{}</style>` içerikleri birebir korunur.
    - `<a title="x>y">t</a>` öznitelik içindeki `>` etiketi bitirmez.
    - `<!DOCTYPE html>`, `<!-- y -->`, `<![CDATA[z]]>` ayrı satırlarda, değişmeden.
    - Kapanmamış `<!-- açık` ve `<div class="a` → kalan metin olduğu gibi eklenir, istisna yok.
    - `"a < b"` metni bozulmaz; eşleşmeyen `</div>` girintiyi negatife düşürmez; `<br/>` girinti artırmaz.
    - 1 MiB'lık iç içe olmayan girdi < 200 ms (kaba süre denetimi; vitest zaman aşımıyla).
  - `previewDoc`:
    - `"<p>x</p>"` → `<meta http-equiv="Content-Security-Policy" content="${PREVIEW_CSP}">` ile başlar.
    - `"  <!DOCTYPE html><html>…"` → meta doctype'tan hemen sonra gelir (doctype ilk kalır, baştaki boşluk korunur).
    - `"<!doctype html>"` küçük harfli doctype da tanınır.
    - Değerin geri kalanı değişmez.
  - `suggestSaveName("foto:1", "png")` → `"foto_1.png"`; `("", "pdf")` → `"hucre.pdf"`.
  - `saveExt({ pdf: true, image: null })` → `"pdf"`; `({ pdf: false, image: "jpeg" })` → `"jpg"`.
- [ ] **Adım 2: Çalıştır, düştüğünü gör.** `bun run test -- src/features/query/cell-format.test.ts` → FAIL.
- [ ] **Adım 3: Uygula.** `formatHtml` tek geçişli el yazımı belirteçleyicidir (spec §2.3); DOM API'si kullanılmaz. `decodeByteaHex` önceden boyutlu `Uint8Array`'e yazar (ara dizi yok).
- [ ] **Adım 4: Doğrula.** Dört ön yüz komutu. `rg "DOMParser|innerHTML|insertAdjacentHTML|document\.write|new Function|eval\(" src/features/query/cell-*` → sonuç yok.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/query/cell-format.ts src/features/query/cell-format.test.ts
  git commit -m "feat(cell): bytea çözme, onaltılık satırları, görsel data URL'i, HTML biçimleyici ve korumalı önizleme belgesi"
  ```

### Task 4: Dil paketleri ve kod görüntüleyici (`cell-code.ts`)

**Dosyalar:** Değiştir `package.json`, `bun.lock`. Oluştur `src/features/query/cell-code.ts`, `src/features/query/cell-code.test.ts`.

**Arayüzler — Üretir** (yalnız tembel `CellViewers` statik içe aktarır):

```ts
import type { Extension } from "@codemirror/state";
import type { Language } from "@codemirror/language";
import type { EditorView } from "@codemirror/view";
export type CodeLang = "json" | "html";
export function loadLanguage(lang: CodeLang): Promise<{ extension: Extension; language: Language }>;
//   json → import("@codemirror/lang-json") → { extension: json(), language: jsonLanguage }
//   html → import("@codemirror/lang-html") → { extension: html({ autoCloseTags: false }), language: htmlLanguage }
//   aynı dil için tek Promise (önbellek); reddedilirse önbellek temizlenir (yeniden denenebilir)
export const viewerHighlight: import("@codemirror/language").HighlightStyle;
//   sharedHighlight etiketleri + tags.propertyName, tags.bool, tags.null, tags.tagName, tags.attributeName,
//   tags.attributeValue, tags.angleBracket, tags.processingInstruction, tags.documentMeta; yalnız --syn-* ve --muted
export function createCodeViewer(parent: HTMLElement, doc: string, language: Extension): EditorView;
//   lineNumbers(), foldGutter(), EditorState.readOnly.of(true), EditorView.editable.of(false),
//   syntaxHighlighting(viewerHighlight), sharedTheme, EditorView.lineWrapping yok (yatay kaydırma), language
export function replaceCode(view: EditorView, doc: string, language: Extension): void;   // belge + dil (Compartment) değişimi
```

- [ ] **Adım 1: Bağımlılıklar.** `package.json` `dependencies`'e `"@codemirror/lang-json": "6.0.2"` ve `"@codemirror/lang-html": "6.4.12"` eklenir; `~/.bun/bin/bun install`. `bun.lock`'ta `@codemirror/language`, `@codemirror/view`, `@codemirror/state` **tek** sürümde kalmalıdır (6.12.4 / 6.43.13 / 6.7.6); ikinci kopya çıkarsa durulur ve raporlanır.
- [ ] **Adım 2: Başarısız testler (`cell-code.test.ts`).**
  - `loadLanguage("json")` çözülür; `language.parser.parse('{"a":[1,true,null]}').topNode.name === "JsonText"` ve ağaçta `⚠` (hata düğümü) yok.
  - `loadLanguage("html")` çözülür; `parse('<div a="1"><p>x</p></div>')` ağacında `Element` düğümü var; `<script>let a=1</script>` için gömülü dil ayrıştırması istisnasızdır.
  - İki çağrı aynı Promise'i döndürür (önbellek).
  - `viewerHighlight` tanımlıdır ve `module` alanı vardır (`HighlightStyle` örneği).
- [ ] **Adım 3: Çalıştır, düştüğünü gör.** `bun run test -- src/features/query/cell-code.test.ts` → FAIL (modül yok).
- [ ] **Adım 4: Uygula.** `sharedTheme`/`sharedHighlight` `src/features/inspector/codemirror-setup.ts`'ten içe aktarılır (kopyalanmaz). `lang-json`/`lang-html` yalnız `loadLanguage` içindeki dinamik `import()`'tadır; statik `import` yok (tür içe aktarımı `import type` olabilir).
- [ ] **Adım 5: Doğrula.**
  - Dört ön yüz komutu.
  - `bun run build` çıktısında `lang-json` ve `lang-html` için ayrı parçalar görülür; ana giriş parçasında ve `QueryTab-*.js`'te `@lezer/json`/`@lezer/html` yoktur (`bundle:check` kaynak tablosu + `rg -l "lezer/html" dist/assets` ile parça adı). Parça adları ve boyutları rapora yazılır.
- [ ] **Adım 6: Commit.**
  ```bash
  git add package.json bun.lock src/features/query/cell-code.ts src/features/query/cell-code.test.ts
  git commit -m "feat(cell): JSON/HTML renklendirme için tembel CodeMirror dil paketleri ve salt okunur görüntüleyici"
  ```

### Task 5: Sekme çubuğu, panel bağlantısı ve tembel parça iskeleti

**Dosyalar:**
- Oluştur: `src/features/query/CellTabs.tsx`.
- Değiştir: `src/features/query/CellPanel.tsx`, `src/features/query/QueryTab.tsx` (yalnız `cellWide` sinyali ve panel kabının genişlik sınıfı).
- Oluştur: `src/features/query/CellViewers.tsx` (iskelet: seçilen biçimin adı + `tcell.loading`; Task 6 gerçeğini yazar).

**Arayüzler — Üretir:**

```ts
// CellTabs.tsx
export function CellTabs(props: {
  value: CellFormat;
  avail: Record<CellFormat, Availability>;
  onSelect: (f: CellFormat) => void;     // yalnız etkin sekme için çağrılır
  idBase: string;                         // createUniqueId(); tab id `${idBase}-tab-${f}`, panel id `${idBase}-panel`
}): JSX.Element;
// CellPanel.tsx
export function CellPanel(props: { cell: ActiveCell | null; wide: boolean; onToggleWide: () => void }): JSX.Element;
const CellViewers = lazy(() => import("./CellViewers"));
// CellViewers.tsx (Task 6'da da aynı imza)
export default function CellViewers(props: {
  format: Exclude<CellFormat, "text">;
  value: string;                          // ham değer (NULL değil)
  column: Column;
  facts: CellFacts;
  ipc?: CellIpc;                          // yoksa cellIpc (testte sahte verilebilir)
}): JSX.Element;
```

- **Panel davranışı:**
  - `facts = createMemo(() => cellFacts(value, column))`; `format = createMemo(() => pickFormat(facts(), formatMemory.get(columnKey(column))))`. Hücre değişince yeniden hesaplanır.
  - Elle seçim: `formatMemory.set(columnKey(column), f)` + yerel sinyal tetikleme (aynı hücrede anında geçiş).
  - Başlık satırı bugünkü gibi; sağa "Genişlet/Daralt" düğmesi (`aria-pressed={props.wide}`) eklenir. Sekme çubuğu başlığın altında ikinci satırdır.
  - `format() === "text"` → bugünkü gövde (değişmeden). Diğerleri → `<Suspense fallback={tcell.loading}><CellViewers …/></Suspense>`; parça yükleme hatası `ErrorBoundary` ile `tcell.loadFailed`.
  - `CellTabs`: tek `tabindex=0` (seçili sekme), diğerleri `-1`; Sol/Sağ/Home/End `nextTab` ile odağı taşır (seçmez); Enter/Space ya da tıklama etkin sekmeyi seçer. Devre dışı: `aria-disabled="true"`, `title={reason}`, `<span class="sr-only">` gerekçe, soluk stil.
- **QueryTab:** `const [cellWide, setCellWide] = createSignal(false)`; kap `class={cellWide() ? "w-[36rem]" : "w-72"}`; `<CellPanel cell={cell()} wide={cellWide()} onToggleWide={() => setCellWide(!cellWide())} />`.

- [ ] **Adım 1: Başarısız test.** Görünüm DOM'u node'da test edilmez; bu görevin mantığı Task 2'dedir. `cell-view-model.test.ts`'e panel akışını izleyen bir senaryo eklenir: `formatMemory` yerine `createFormatMemory()` ile "PNG hücresi → `image`; kullanıcı `hex` seçer (`set`) → aynı sütunda imzasız hücre → `hex`; başka sütun → kendi kuralı". Çalıştır → bu senaryo Task 2 uygulamasıyla zaten geçiyorsa not düşülür ve Adım 2 atlanır.
- [ ] **Adım 2: Uygula.** Kararlar 1, 2, 11. `CellPanel` `cell-format`, `cell-code`, `cell-ipc` dosyalarını içe aktarmaz.
- [ ] **Adım 3: Doğrula.**
  - Dört ön yüz komutu.
  - `bun run build` çıktısında yeni `CellViewers-*.js` parçası; `QueryTab-*.js` içinde `cell-format`/`cell-code` kaynakları yok (kaynak haritasıyla: `node scripts/bundle-check.mjs` çıktısı yalnız ana paketi listeler; QueryTab parçası için `rg -c "formatHtml|loadLanguage" dist/assets/QueryTab-*.js` → 0).
  - `bundle:check` ana paket baytı rapora yazılır (değişmemiş olmalıdır; değiştiyse neden araştırılır).
- [ ] **Adım 4: Commit.**
  ```bash
  git add src/features/query/CellTabs.tsx src/features/query/CellPanel.tsx src/features/query/QueryTab.tsx src/features/query/CellViewers.tsx src/features/query/cell-view-model.test.ts
  git commit -m "feat(cell): hücre panelinde biçim sekmeleri, sütun başına hatırlama ve panel genişletme"
  ```

### Task 6: Görüntüleyiciler (`CellViewers.tsx`)

**Dosyalar:** Değiştir `src/features/query/CellViewers.tsx` (iskeletin yerine gerçek). İsteğe bağlı alt bileşenler aynı dosyada (`HexView`, `ImageView`, `CodeView`, `HtmlView`, `PdfView`, `SizeNotice`).

**Arayüzler — Tüketir:** Task 2 (`CellFacts`, `LIMITS`, `sizeVerdict`, `previewAllowed`, `fileActionAllowed`, `formatMb`, `tcell`), Task 3 (`decodeByteaHex`, `utf8Bytes`, `hexRow`, `hexRowCount`, `dataUrl`, `pdfVersion`, `formatHtml`, `previewDoc`, `suggestSaveName`, `saveExt`), Task 4 (`loadLanguage`, `createCodeViewer`, `replaceCode`), Task 1 (`cellIpc`), `reindentJson` (`grid-model.ts`), `save` (`@tauri-apps/plugin-dialog`), `toast` (`src/app/toast`), `createVirtualizer` (`@tanstack/solid-virtual`).

- **`SizeNotice`:** `sizeVerdict` reddederse görünüm yerine `tcell.tooLarge(formatMb(size), formatMb(limit))` (bytea dışı türde `tooLargeText`); bytea ise altında "Kaydet…" düğmesi (`fileActionAllowed` etkinse).
- **`HexView`:** baytlar bir kez `createMemo` ile: bytea → `decodeByteaHex(value, LIMITS.hexBytes)`, diğer → `utf8Bytes(value, LIMITS.hexBytes)`; null → `tcell.reason.badHex`. Sabit 18 px satır, `createVirtualizer({ count: hexRowCount(len), estimateSize: () => 18, overscan: 20 })`; satır metni `hexRow` ile yalnız görünür satırlar için üretilir. Üstte `tcell.hex.total(facts.byteLength)`; kesildiyse `tcell.hex.partial(…)`.
- **`ImageView`:** `sizeVerdict("image")`; bytea → `decodeByteaHex(value)`, metin SVG → `utf8Bytes(value)`; `dataUrl(kind, bytes)` bir kez `createMemo`; `img` `alt` boyut yüklenene dek tür adıyla; `onLoad` → boyut; `onError` → `tcell.image.broken`. Sığdır: `max-w-full max-h-full object-contain`; Gerçek boyut: kap `overflow-auto`. Desen (dama) arka plan saydamlığı gösterir. "Kaydet…" (bytea).
- **`CodeView` (JSON ve HTML kaynağı):** `onMount` → `loadLanguage(lang)` → `createCodeViewer(host, metin, extension)`; değer/biçim değişince `replaceCode`; `onCleanup` → `view.destroy()`. JSON metni `reindentJson(value) ?? value`; HTML metni `biçimlendir ? formatHtml(value) : value`. Yükleme hatası `tcell.loadFailed`.
- **`HtmlView`:** iki düğmeli kip seçici (Kaynak/Önizleme, `aria-pressed`) + "Biçimlendir" onay kutusu (yalnız Kaynak'ta). Önizleme: `previewAllowed` değilse düğme `aria-disabled` + gerekçe. `<iframe sandbox="" referrerpolicy="no-referrer" title={tcell.html.frameTitle} srcdoc={previewDoc(value)} class="h-full w-full bg-white" />` + altta `tcell.html.previewNote`. `srcdoc` dışında hiçbir HTML yolu yoktur.
- **`PdfView`:** `decodePrefix(value, 16)` + `pdfVersion` ile bilgi satırı; "Varsayılan uygulamada aç" → `ipc.openPdf(value)`; "Kaydet…" → `save({ defaultPath: suggestSaveName(column.name, "pdf"), filters: [{ name: "PDF", extensions: ["pdf"] }], title: tcell.saveTitle })` → `ipc.saveBytes(value, yol)`. İkisi `fileActionAllowed` ile etkin; çalışırken devre dışı ve metin `opening`/`saving`. Hata → `toast` `tcell.openFailed`/`saveFailed` (ileti ayrıntısı eklenmez). Uyarı satırı `tcell.pdf.warn`.
- **Ortak "Kaydet…"** yardımcısı `saveCell(value, column, facts, ipc)` (Görsel, PDF ve `SizeNotice` kullanır); `null` yol sessiz; başarı → `toast(tcell.saved)`.

- [ ] **Adım 1: Başarısız test.** Görünüm mantığı Task 2/3'tedir. `saveCell` saf bir koordinatöre ayrılır ve `cell-format.test.ts`'e eklenir: sahte `pick` `null` → `saveBytes` çağrılmaz; sahte `pick` `"C:\\x\\foto.png"` → `saveBytes(value, "C:\\x\\foto.png")` ve `onSaved` çağrılır; `saveBytes` reddi → `onError` çağrılır, istisna yayılmaz. İmza: `export async function saveCell(d: { pick(o: { defaultPath: string; filters: { name: string; extensions: string[] }[]; title: string }): Promise<string | null>; saveBytes(data: string, path: string): Promise<unknown>; onSaved(): void; onError(): void }, value: string, columnName: string, f: { pdf: boolean; image: ImageKind | null }): Promise<void>` (`cell-format.ts`'te). `bun run test -- src/features/query/cell-format.test.ts` → FAIL.
- [ ] **Adım 2: Uygula.** Yukarıdaki maddeler. Görünümler yalnız çizer; çözme/biçimleme `createMemo`'dadır ve yalnız görünüm açıkken çalışır.
- [ ] **Adım 3: Doğrula.**
  - Dört ön yüz komutu. `bun run build` çıktısında `CellViewers-*.js` boyutu ve dil parçaları rapora yazılır.
- [ ] **Adım 4: Güvenlik ve gizlilik taraması.**
  - `rg "DOMParser|innerHTML|insertAdjacentHTML|document\.write|new Function|eval\(" src/features/query/Cell* src/features/query/cell-*` → sonuç yok.
  - `rg "sandbox=" src/features/query` → yalnız `sandbox=""`; `rg "allow-(scripts|same-origin|top|forms|popups)" src` → sonuç yok.
  - `rg "console\." src/features/query/Cell* src/features/query/cell-*` → yok; `rg "localStorage|writeStorage" src/features/query/Cell* src/features/query/cell-*` → yok.
  - `git diff main -- src-tauri/tauri.conf.json src-tauri/capabilities` → boş.
- [ ] **Adım 5: Commit.**
  ```bash
  git add src/features/query/CellViewers.tsx src/features/query/cell-format.ts src/features/query/cell-format.test.ts
  git commit -m "feat(cell): onaltılık, görsel, JSON, HTML (kaynak + korumalı önizleme) ve PDF görüntüleyicileri"
  ```

### Task 7: Bütçe telafisi, smoke maddeleri ve belgeler

**Dosyalar:** `src/app/strings.ts` ve telafi için seçilen tembel tüketiciler (aşağıda), `scripts/bundle-budget.json` (yalnız indirme), `docs/smoke-test.md`, `docs/testing.md`, `docs/superpowers/specs/2026-11-05-cell-viewer-design.md`.

- [ ] **Adım 1: Ölç.** `bun run bundle:check`. Ana paket ve açılış kümesi baytları ve tavanlarla farkları (beklenen artış yalnız `call` dışa aktarımı; birkaç bayt) kaydedilir. Artış beklenenden büyükse (≥ 64 B) önce neden araştırılır: QueryTab dışı bir modülün yeni dosyaları statik içe aktarıp aktarmadığı kaynak tablosundan bulunur ve düzeltilir.
- [ ] **Adım 2: Telafi (yalnız tembel parçaların kullandığı kodu ana paketten çıkararak).** Aday sırası:
  1. `t.query.errorMark` ("Hata"): tek tüketicisi `src/features/query/editor.ts` (tembel QueryTab parçası; 2026-11-05 ölçümünde ana pakette değil). `tedit`'e (`src/features/query/strings-editor.ts`, yalnız tembel parça içe aktarır) `errorMark` olarak taşınır; `editor.ts` `tedit.errorMark` kullanır; `strings.ts`'ten silinir.
  2. Hâlâ fazla varsa: `rg "t\.<blok>\.<anahtar>" src` ile `strings.ts`'teki her anahtarın tüketicileri listelenir; tüketicilerinin **hepsi** tembel parçada olan anahtar ilgili tembel metin modülüne taşınır. Tembellik `bundle:check` kaynak tablosuyla kanıtlanır (tüketici dosya tabloda görünmemeli).
  3. Hâlâ fazla varsa: `call` dışa aktarımından vazgeçmek yerine durulur ve sorulur (tavan yükseltilmez).

  Taşınan her anahtar için `rg` kanıtı rapora yazılır. Metinler birebir korunur.
- [ ] **Adım 3: Tavanı indir.**
  - Hedef: ana paket ≤ 127,364 B ve açılış kümesi ≤ 155,037 B.
  - Ölçülen değerler tavandan küçükse `scripts/bundle-budget.json`'da `mainMaxBytes` ve `startupMaxBytes` yeni değerlere indirilir (sıfır pay).
  - Telafi yetmezse tavan yükseltilmez: görev BLOCKED raporlanır (bayt farkı ve denenen taşımalar yazılır); karar kullanıcıya aittir.
  - `bun run bundle:check` PASS.
- [ ] **Adım 4: Smoke maddeleri.** `docs/smoke-test.md`'de "## Görsel EXPLAIN arayüzü (Plan 28)" bölümünden sonra **"## Hücre görüntüleyici (Plan 31)"** eklenir.
  - Ön koşul paragrafı:
    - Yayın exe (`tauri build`); Docker PG 10 ve 18.
    - Örnek tablo: `bytea` sütununda PNG, JPEG, GIF (animasyonlu), WebP, BMP, ICO, SVG (içinde `<script>` olan), PDF, imzasız rastgele bayt, 25 MB PNG, 70 MB bayt; `jsonb` (iç içe, büyük tamsayılı `12345678901234567890`); `text` sütununda JSON, HTML e-posta gövdesi (uzak `<img>` izleme pikseli), `<script>` içeren HTML, XML, 3 MB HTML; `xml` sütunu; Türkçe adlı sütun.
    - Örnek SQL (`decode('…', 'base64')`, `pg_read_binary_file` yerine istemciden yüklenmiş veriler) bölümde verilir.
  - `[—]` maddeleri:
    - kendiliğinden seçim: `jsonb` → JSON, PNG → Görsel, PDF → PDF, imzasız → Metin; devre dışı sekmelerde ipucu gerekçesi;
    - sütun başına hatırlama: Onaltılık seçip satır değiştirme; başka sütun etkilenmez; yeni sorgu sekmesinde de geçerli (oturum boyu); uygulama yeniden açılınca sıfırlanır;
    - Metin sekmesi bugünkü gibi (1 MB kırpma uyarısı dahil) ve hiçbir yeni parça yüklemez (DevTools Network);
    - Onaltılık: ofset/16 bayt/ASCII hizası; 70 MB değerde ilk 16 MB notu ve akıcı kaydırma; `text` sütununda `ş` → `C5 9F`;
    - Görsel: yedi biçim çizilir; animasyonlu GIF oynar; SVG içindeki `<script>` çalışmaz (alert/IPC yok); sığdır/gerçek boyut; boyut bilgisi; 25 MB PNG'de sınır iletisi; bozuk PNG'de "çözülemedi"; "Kaydet…" dosyası özgünle aynı (`certutil -hashfile … SHA256` karşılaştırması);
    - JSON: renklendirme açık ve koyu temada okunur; katlama; büyük tamsayı kaybolmaz; 5 MB JSON'da sınır iletisi;
    - HTML kaynak: biçimlendirme ve "Biçimlendir" kapalıyken özgün metin; `<script>`/`<style>` içerikleri renklenir;
    - **HTML önizleme güvenliği:**
      - `<script>alert(1)</script>`, `<img src=x onerror="alert(1)">`, `<script>window.__TAURI_INTERNALS__.invoke('list_servers')</script>` ve `<script>parent.postMessage('x','*')</script>` içeren değerde hiçbir uyarı çıkmaz; DevTools Console'da yalnız sandbox engel iletileri görünür; Tauri IPC'ye **erişilemez** (uygulama günlüğünde ve Network'te `ipc.localhost` isteği yok);
      - uzak `<img src="https://…">`, `<link rel=stylesheet href="https://…">` ve `@import` için Network'te **hiç istek yok**;
      - `<a href="https://example.com">` ve `<a target="_top">` tıklaması uygulamayı ya da iframe'i uzak sayfaya götürmez; `<meta http-equiv="refresh" content="0;url=https://example.com">` yönlendirmez; `<form action=…>` gönderilmez;
      - WebView2 önizlemeyi CSP ihlali olmadan çizer (çizmezse durulur, sorulur);
      - 3 MB HTML'de önizleme anahtarı gerekçeyle devre dışı;
    - PDF: "Varsayılan uygulamada aç" sistemin PDF uygulamasını açar; dosya `%LOCALAPPDATA%\com.pgtools.desktop\cell-view\cell-<uuid>.pdf` altındadır; uygulama kapatılınca dosya silinir (okuyucu kapalıyken); okuyucu açıkken kapatılırsa dosya kalır ve 24 saatten eski olunca sonraki açılışta silinir; "Kaydet…" ve iptal; 65 MB PDF'de düğmeler gerekçeyle devre dışı;
    - Panel genişlet/daralt; dar panelde görünümler taşmaz;
    - klavye: sekme çubuğunda Sol/Sağ/Home/End, Enter/Space seçimi, devre dışı sekme seçilmez; ekran okuyucu sekme adını ve gerekçeyi okur;
    - gizlilik: değer, yol ya da dosya adı uygulama günlüğünde (`%LOCALAPPDATA%\com.pgtools.desktop\logs`) ve bildirimlerde görünmez;
    - açılış süresi ve bellek Plan 21 ölçümüyle aynı aralıkta (ana paket değişmedi).
- [ ] **Adım 5: Spec ve test belgesi.**
  - Spec `Durum`: "Plan 31 uygulandı (feat/cell-viewer); elle kontrol bekliyor (`docs/smoke-test.md` "Hücre görüntüleyici (Plan 31)")".
  - Spec'e yeni **"## 7. Plan 31 uygulama notları"** eklenir: paket ölçümleri (ana / açılış / QueryTab / `CellViewers` / dil parçaları), telafi kaydı, uygulamada çıkan sapmalar.
  - `docs/testing.md`'ye "## Hücre görüntüleyici testleri (Plan 31)" eklenir: dosya → kapsam tablosu (`cell-view-model.test.ts`, `cell-format.test.ts`, `cell-code.test.ts`, Rust `cell.rs` testleri), paket tavanları ve elle kontrol bağlantısı.
- [ ] **Adım 6: Kapanış taramaları ve doğrulama.**
  - `rg "strings-cell" src` → yalnız `src/features/query/*`.
  - `rg "lang-json|lang-html" src` → yalnız `src/features/query/cell-code.ts`.
  - `rg "cell-ipc" src` → yalnız `CellViewers.tsx`.
  - Task 6 Adım 4 güvenlik taraması yeniden.
  - `git status` → `graphify-out/` ve binding satır sonu gürültüsü sahnelenmemiş (`git checkout -- src/ipc/bindings`).
  - `cargo test -p pg-tools cell`, `cargo clippy --workspace --all-targets -- -D warnings` ve dört ön yüz komutu PASS.
- [ ] **Adım 7: Commit.**
  ```bash
  git add src/app/strings.ts src/features/query/strings-editor.ts src/features/query/editor.ts scripts/bundle-budget.json docs/smoke-test.md docs/testing.md docs/superpowers/specs/2026-11-05-cell-viewer-design.md
  git commit -m "docs(cell): Plan 31 paket telafisi, smoke maddeleri ve notlar"
  ```
  (Adım 2'de başka dosya taşındıysa `git add` listesine o dosyalar da eklenir.)

## Öz inceleme

- **Spec kapsamı:**
  - §2.1 uygulanabilirlik, seçim önceliği, hatırlama → T2; sekme çubuğu ve devre dışı gerekçeleri → T5.
  - §2.2 görünümler → T6; HTML iframe ve CSP meta → T3 `previewDoc` + T6 `HtmlView`; panel genişliği → T5.
  - §2.3 biçimleme → T3 `formatHtml`, `reindentJson` (mevcut); renklendirme ve bağımlılıklar → T4.
  - §2.4 komutlar, geçici dosya, temizlik → T1; ön yüz kullanımı → T6.
  - §2.5 sınırlar → T2 `sizeVerdict`/`previewAllowed`/`fileActionAllowed` + T3 `decodeByteaHex(max)` + T6 `SizeNotice` + T1 `too_large`.
  - §2.6 paket → T4/T5 parça doğrulamaları, T7 telafi.
  - §2.7 gizlilik/erişilebilirlik → Global Constraints, T5, T6 Adım 4, T7 smoke.
- **Tip tutarlılığı:**
  - `CellViewers` imzası T5 = T6.
  - `CellFacts`/`Availability`/`CellFormat`/`ImageKind` T2 → T3, T5, T6.
  - `CellIpc` T1 → T6; `saveCell` T6 (`cell-format.ts`).
  - `loadLanguage`/`createCodeViewer`/`replaceCode` T4 → T6.
- **Orantı:** kod gövdesi yok; yalnız imzalar, sabit metinler ve test beklentileri.
