# pg-tools — Hücre Görüntüleyici (Biçim Sekmeleri) Tasarımı

- **Tarih:** 2026-11-05
- **Durum:** Tasarım; Plan 31 yazıldı (`docs/superpowers/plans/2026-11-05-cell-viewer-p31.md`), XML sekmesi eklendi (2026-11-05), uygulanıyor
- **Kapsam:**
  - Query Tool'un hücre paneli (`src/features/query/CellPanel.tsx`) bugün seçili hücrenin değerini düz metin olarak gösterir (json/jsonb kayıpsız girintilenir, 1 MB üstü kırpılır).
  - Panele biçim sekmeleri eklenir: **Metin**, **Onaltılık**, **Görsel**, **JSON**, **XML**, **HTML**, **PDF**.
  - JSON, XML ve HTML biçimlendirilir **ve** sözdizimi renklendirilir (kullanıcı isteği; XML 2026-11-05'te eklendi). HTML ayrıca korumalı bir önizlemede çizilebilir; XML önizlenmez (yalnız kaynak).
  - PDF gömülü gösterilmez; işletim sisteminin varsayılan uygulamasında açılır ya da kaydedilir.
  - Yalnız Query Tool. Veri düzenleyici (`src/features/data`) `CellPanel` kullanmaz; ona taşımak takip işidir (§6).
- **Kullanıcı kararları (2026-11-05):**
  - XML sekmesi (2026-11-05 ek isteği): PostgreSQL `xml` türü ve metin türündeki işaretleme için biçimlendirilmiş + renklendirilmiş salt okunur kaynak görünümü.
  - Sekmeler sütun türü ve içerikten **kendiliğinden** seçilir; kullanıcının elle seçimi oturum boyunca sonuç sütunu başına hatırlanır. Uygulanamayan sekmeler görünür ama devre dışıdır ve gerekçe ipucu gösterir.
  - HTML önizlemesi `sandbox=""` iframe'de `srcdoc` ile çizilir: betik yok, aynı köken yok, uzak kaynak yok.
  - PDF için ilk sürümde gömülü görüntüleyici yoktur: "Varsayılan uygulamada aç" ve "Kaydet…".
  - Ana paket tavanı **asla yükseltilmez**. Görüntüleyiciler tembel parçadadır; ana pakete gelen her bayt, yalnız tembel parçaların kullandığı kodun ana paketten çıkarılmasıyla telafi edilir. Telafi yetmezse durulup sorulur.
  - Hücre değerleri hiçbir yerde günlüğe yazılmaz. Geçici PDF dosyaları kullanıcı verisi içerir: kısıtlı konumda, benzersiz adla tutulur ve uygulama kapanırken silinir.
- **Teslim:** Plan 31 (tek plan): küçük arka uç (`src-tauri/src/cell.rs`: PDF aç/kaydet, geçici dosya yaşam döngüsü), saf model ve biçimleyiciler, sekme çubuğu, tembel görüntüleyici parçası, üç yeni CodeMirror dil paketi (JSON, XML, HTML), paket telafisi, smoke maddeleri ve belgeler.

---

## 1. Mevcut altyapı (yeniden kullanılır)

- **Değerin yolu:** sonuç hücreleri arka uçtan **tam** metin olarak gelir (sunucu tarafında hücre kırpması yok). `ResultGrid` yalnız görüntüde 1000 karaktere kırpar (`MAX_CELL_CHARS`); panel `ActiveCell { row, column: Column, value: string | null }` alır. `Column` `name`, `typeName` (iç kısa ad: `bytea`, `jsonb`, `text`…), `typeOid`, `typeDisplay` taşır.
- **`bytea`** metin protokolünde `\x<onaltılık>` olarak gelir (sunucu varsayılanı `bytea_output = hex`). Görsel ve PDF bu metnin bayta çözülmesini gerektirir.
- **`grid-model.ts`:** `CELL_PANEL_MAX = 1024 * 1024` (karakter), `isJsonType`, `clipChars` (vekil çifti bölmez), `reindentJson` (kayıpsız: sayı ve dizgileri ayrıştırmadan kopyalar; büyük tamsayılar bozulmaz) ve `cellPanelText`. Metin sekmesi bunları olduğu gibi kullanır.
- **CodeMirror:** `src/features/inspector/codemirror-setup.ts` `sharedTheme`, `sharedHighlight` (`--syn-*` değişkenleri) ve `createReadOnlyEditor` sağlar. Kurulu paketler: `@codemirror/{autocomplete 6.20.3, commands 6.11.1, lang-sql 6.10.0, language 6.12.4, state 6.7.6, view 6.43.13}`, `@lezer/{common 1.5.3, highlight 1.2.5, lr 1.4.10}`. `@codemirror/lang-json`, `@codemirror/lang-xml` ve `@codemirror/lang-html` **kurulu değil**.
- **Sanal liste:** `@tanstack/solid-virtual` (kurulu; `ResultGrid` kullanır).
- **Kayıt diyaloğu:** `@tauri-apps/plugin-dialog` `save()` (emsal `ExportUi.tsx`: yol ön yüzde seçilir, yazmayı arka uç yapar). Yetki `dialog:allow-save` mevcut.
- **Dış uygulama başlatma:** `backup_reveal` (`src-tauri/src/backup.rs`) `%SystemRoot%\explorer.exe`'yi PATH'ten değil tam yoldan, `raw_arg` ile tırnaklı yol vererek başlatır ve beklemez; `reveal_arg` `"` ve denetim karakterlerini reddeder, Windows'ta mutlak yol ister. `tauri-plugin-opener` **yoktur** ve eklenmez.
- **CSP** (`src-tauri/tauri.conf.json`): `default-src 'self' ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; img-src 'self' data:`. `blob:` ve `frame-src` yok.
- **Paket:** `QueryTabs.tsx:23` `const QueryTab = lazy(() => import("./QueryTab"))`. `CellPanel`, `ResultGrid`, `grid-model.ts` ve `strings-grid.ts` **zaten tembel QueryTab parçasındadır** (2026-11-05 ölçümü: ana paket 127,364 B = tavan; açılış kümesi 155,037 B = tavan; `grid-model.ts` ana pakette görünmüyor). `src/ipc/commands.ts` ana pakettedir (4,291 B).

## 2. Kararlar

### 2.1 Biçimler ve uygulanabilirlik

| Sekme | Uygulanır | Kendiliğinden seçilir |
|---|---|---|
| Metin | her zaman (değer NULL değilse) | diğer hiçbir kural tutmazsa |
| Onaltılık | her zaman. `bytea` (`\x` önekli) → gerçek baytlar; diğer türler → değerin UTF-8 baytları | hiçbir zaman |
| Görsel | baytlar PNG/JPEG/GIF/WebP/BMP/ICO imzasıyla başlıyorsa ya da metin/bayt SVG koklamasından geçiyorsa | `bytea` + görsel imzası |
| JSON | `json`/`jsonb` türü; ya da metin türünde kırpılmış değer `{` veya `[` ile başlayıp `JSON.parse`'tan geçiyorsa | `json`/`jsonb` |
| XML | `xml` türü (NULL değilse her zaman); ya da `bytea` dışı türde kırpılmış değer `<` ile başlıyor ve `>` içeriyorsa (HTML ile aynı işaretleme koşulu) | `xml` |
| HTML | `bytea` dışı türde kırpılmış değer `<` ile başlıyor ve `>` içeriyorsa (HTML, XML, SVG kaynağı) | hiçbir zaman |
| PDF | `bytea` + ilk baytlar `%PDF-` | `bytea` + `%PDF-` |

- **İmzalar** (ilk baytlar): PNG `89 50 4E 47 0D 0A 1A 0A`; JPEG `FF D8 FF`; GIF `GIF87a`/`GIF89a`; WebP `RIFF????WEBP`; BMP `BM`; ICO `00 00 01 00`. SVG koklaması: ilk 1 KB UTF-8 çözülür; BOM, boşluk, `<?xml …?>`, yorumlar ve `<!DOCTYPE …>` atlanır; kalan `<svg` ile başlamalıdır (büyük/küçük harf duyarsız). MIME bu imzadan türetilir; sütun adı ya da uzantı kullanılmaz.
- **Algılama ucuzdur:** `bytea` için yalnız ilk 1 KB'lık önek (2 + 2048 karakter) çözülür. Tam çözme yalnız ilgili sekme gösterildiğinde, tembel parçada yapılır.
- **`bytea_output = escape`** oturumunda değer `\x` ile başlamaz: değer metin sayılır (Onaltılık UTF-8 baytlarını gösterir; Görsel/PDF uygulanmaz). Bilinen sınırdır (§5).
- **Seçim önceliği:** (1) bu sütun için hatırlanan seçim, **uygulanabiliyorsa**; (2) tablodaki kendiliğinden kural; (3) Metin. Hatırlanan seçim uygulanamıyorsa o hücrede kendiliğinden kurala düşülür ama hatırlanan değer silinmez (sonraki satırda yine geçerli olabilir).
- **Hatırlama anahtarı:** `${typeOid}:${name}`. Saklama modül düzeyinde bir `Map`'tir (QueryTab parçası; uygulama oturumu boyunca, sekmeler arasında ortak). `localStorage`'a yazılmaz (sütun adı da kullanıcı verisi sayılır).
- **XML ile HTML örtüşmesi:** işaretleme içeren bir `text` değerinde ikisi de etkindir; hiçbiri kendiliğinden seçilmez. `xml` türünde HTML sekmesi de etkindir (işaretleme koşulu tutarsa) ama kendiliğinden XML seçilir. İyi biçimlilik (well-formedness) denetlenmez: `DOMParser` yasaktır (§2.3) ve `xml` türü sunucuda zaten ayrıştırılmıştır; bozuk metin yine biçimlenir (biçimleyici hata vermez).
- **Devre dışı sekme:** `aria-disabled="true"` (odaklanabilir kalır, ipucu görünür), `title` ve ekran okuyucu için gizli metin gerekçeyi söyler. Gerekçeler: "Değer NULL", "Tanınan bir görsel biçimi değil", "Geçerli JSON değil", "XML işaretlemesi değil", "HTML/XML işaretlemesi değil", "PDF değil (`%PDF-` ile başlamıyor)", "Yalnız bytea sütunlarında", "Yalnız metin sütunlarında" (HTML, bytea için), "bytea değeri çözülemedi".

### 2.2 Görünümler

- **Metin:** bugünkü görünüm birebir (`cellPanelText`, 1 MB kırpma ve uyarısı). Tembel parça gerektirmez; varsayılan durumda hiçbir ek parça yüklenmez.
- **Onaltılık:** `00000000  89 50 4E 47 0D 0A 1A 0A  00 00 00 0D 49 48 44 52  |.PNG........IHDR|` biçiminde satır başına 16 bayt (ofset 8 onaltılık hane, 8+8 grup, ASCII sütununda 0x20–0x7E dışı `.`). Sanal listeyle çizilir (sabit satır yüksekliği). Üstte toplam bayt sayısı.
- **Görsel:** `<img src="data:<mime>;base64,…">`. Panel genişliğine sığdırılır; "Gerçek boyut" düğmesi taşmayı kaydırmaya çevirir. Altta biçim, piksel boyutu (`naturalWidth × naturalHeight`, yüklemeden sonra) ve bayt. `img` içindeki SVG betik çalıştıramaz ve dış kaynak yükleyemez. `onerror` → "Görsel çözülemedi (bozuk ya da desteklenmeyen veri)". "Kaydet…" (§2.4).
- **JSON:** salt okunur CodeMirror: `@codemirror/lang-json` + `syntaxHighlighting` + `foldGutter()` (katlama, ayrı ağaç görünümü yerine; kurulu `@codemirror/language`'dan gelir, ek maliyeti yok). Biçimlendirme `reindentJson` (kayıpsız). `json`/`jsonb` dışı türde değer zaten tek satırlık değilse de aynı biçimlendirme uygulanır.
- **XML:** salt okunur CodeMirror: `@codemirror/lang-xml` + `syntaxHighlighting` + `foldGutter()` + `lineNumbers()`. "Biçimlendir" anahtarı (varsayılan açık) `formatXml` (§2.3) çıktısını, kapatılınca özgün metni gösterir. Önizleme yoktur: XSLT/DTD işlenmez, dış varlık çözülmez; metin hiçbir ayrıştırıcıya verilmez, yalnız CodeMirror'un sözdizimi ağacı kurulur.
- **HTML:**
  - Varsayılan **Kaynak** kipi: el yazımı biçimleyiciyle (§2.3) girintilenmiş metin, salt okunur CodeMirror + `@codemirror/lang-html` (gömülü CSS/JS de renklenir) + katlama. "Biçimlendir" anahtarı (varsayılan açık) kapatılınca özgün metin gösterilir.
  - **Önizleme** anahtarı: `<iframe sandbox="" referrerpolicy="no-referrer" srcdoc=…>`. Önizleme her zaman **özgün** değeri kullanır (biçimlendirme satır içi boşlukları değiştirebilir). Arka plan temadan bağımsız beyazdır.
  - `srcdoc` belgesine ek bir CSP `<meta>` eklenir: `default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:`. Değer `<!DOCTYPE …>` ile başlıyorsa meta doctype'tan **sonra**, değilse en başa eklenir (doctype'tan önce eklemek belgeyi quirks kipine sokardı).
  - `sandbox=""`: betik, form gönderimi, açılır pencere, üst pencereyi yönlendirme ve aynı köken kapalıdır; belge opak kökendedir, `window.__TAURI_INTERNALS__`'a ve Tauri IPC'ye erişemez (betik zaten çalışmaz). `srcdoc` belgesi ebeveynin CSP'sini de devralır: uzak görsel/stil/yazı tipi istekleri iki politika tarafından da engellenir (e-postalardaki izleme pikselleri dahil).
  - **CSP değişikliği yoktur.** `srcdoc` bir ağ isteği değildir; `frame-src` (burada `default-src 'self'`'e düşer) iframe'in uzak bir adrese gitmesini engeller ve bu istenen davranıştır. Smoke'ta WebView2'nin önizlemeyi CSP ihlali olmadan çizdiği doğrulanır; çizmezse uygulayıcı durur ve sorar (CSP'ye `frame-src` eklemek kullanıcı onayı ister).
- **PDF:** gömülü görüntüleyici yok. Bilgi satırı (sürüm, `%PDF-1.7` → "PDF 1.7"; bayt) ve iki eylem: **"Varsayılan uygulamada aç"** ve **"Kaydet…"** (§2.4). Uyarı satırı: "PDF dışarıda açılır; güvenmediğiniz belgeleri açmayın."
- **Kopyala** düğmesi her sekmede ham değeri (`\x…` dahil) kopyalar; değişmez.
- **Panel genişliği:** bugün `w-72` (288 px). Başlığa "Genişlet" anahtarı eklenir: `w-72` ↔ `w-[36rem]`; durum QueryTab sinyalidir (oturumluk, kalıcı değil).

### 2.3 Biçimlendirme

- **JSON:** `reindentJson` (mevcut; `JSON.parse` ile doğrular, ardından dizgi dışı boşlukları atıp girintiler). Yeni kod yok.
- **HTML:** küçük, el yazımı, **saf metin** biçimleyici (`formatHtml(text): string`); bağımlılık eklenmez (js-beautify vb. ~80 KB ve gereğinden geniş).
  - Belirteçler: başlangıç etiketi (tırnaklı öznitelik değerindeki `>` etiketi bitirmez), bitiş etiketi, yorum `<!-- … -->`, `<!DOCTYPE …>`, `<?…?>`, CDATA, metin.
  - Her etiket kendi satırına, iç içelik 2 boşlukla girintilenir. Boş öğeler (`area base br col embed hr img input link meta source track wbr`) ve `/>` ile kapananlar girintiyi artırmaz.
  - `pre`, `textarea`, `script`, `style` içeriği **olduğu gibi** kopyalanır (tek parça; içinde etiket aranmaz).
  - Metin düğümleri kırpılır; yalnız boşluktan oluşanlar atılır. Kapanmamış etiket/yorum: kalan metin olduğu gibi eklenir (hata yok). Eşleşmeyen bitiş etiketi girintiyi 0'ın altına düşürmez.
  - **Hiçbir şey çalıştırılmaz:** `DOMParser`, `innerHTML`, `document.write`, `eval` kullanılmaz; çıktı yalnız CodeMirror belgesine metin olarak girer.
- **XML:** aynı belirteçleyiciyi paylaşan `formatXml(text): string` (el yazımı, bağımlılık yok). HTML'den farkları:
  - Boş öğe listesi ve ham metin öğeleri (`pre`/`textarea`/`script`/`style`) **yoktur**; yalnız `/>` ile kapanan öğe girintiyi artırmaz. Etiket adları büyük/küçük harf duyarlıdır; girinti yalnız başlangıç/bitiş etiketi sayımıyla yürür.
  - Yalnız metin içeren öğe tek satırda kalır: `<ad>Ayşe</ad>` (metin kırpılmaz, değişmez). Karma içerikte (metin + alt öğe) metin düğümleri kırpılarak kendi satırına yazılır; yalnız boşluktan oluşan metin düğümleri atılır.
  - `xml:space="preserve"` özniteliği taşıyan öğenin tüm içeriği (eşleşen bitiş etiketine kadar, iç içe aynı adlı öğeler sayılarak) **olduğu gibi** kopyalanır.
  - `<?xml …?>`, `<!DOCTYPE …>` (iç alt küme `[…]` dahil; köşeli parantez içindeki `>` belirteci bitirmez), yorum ve CDATA kendi satırında, değişmeden.
  - Kapanmamış yapı ve eşleşmeyen bitiş etiketi HTML'deki gibi ele alınır (hata yok, girinti 0'ın altına düşmez).
  - Hiçbir şey ayrıştırılmaz ya da çalıştırılmaz (`DOMParser` yok; `&…;` varlıkları çözülmez, metin olarak kalır).
- **Renklendirme:** salt okunur CodeMirror (`EditorState.readOnly`, `EditorView.editable.of(false)`), `sharedTheme` + görüntüleyiciye özgü bir `HighlightStyle` (JSON özellik adı, `bool`/`null`, HTML/XML etiket adı, öznitelik adı/değeri, köşeli parantez; renkler mevcut `--syn-*` değişkenlerinden) + `lineNumbers()` + `foldGutter()`.
- **Bağımlılıklar** (tam sürüm, `^` yok): `@codemirror/lang-json` **6.0.2** (bağımlılıkları `@codemirror/language ^6.0.0`, `@lezer/json ^1.0.0`), `@codemirror/lang-xml` **6.1.0** (`@codemirror/{autocomplete, language ^6.4.0, state, view}`, `@lezer/{common, xml ^1.0.0}`) ve `@codemirror/lang-html` **6.4.12** (`@codemirror/{autocomplete, lang-css, lang-javascript, language ^6.4.0, state, view ^6.17.0}`, `@lezer/{common, css, html ^1.3.12}`). Kurulu `language 6.12.4`, `view 6.43.13`, `state 6.7.6` aralıkları karşılar; yeni geçişli paketler (`@lezer/json`, `@lezer/xml`, `@lezer/html`, `@lezer/css`, `@lezer/javascript`, `@codemirror/lang-css`, `@codemirror/lang-javascript`) `bun install` ile kilitlenir ve `bun.lock` commit edilir.
- **Parça ayrımı:** `lang-json`, `lang-xml` ve `lang-html` yalnız dinamik `import()` ile, kod görüntüleyici tembel modülünden (`cell-code.ts`) yüklenir; her biri ayrı parçadadır. `lang-html` (CSS/JS dilleriyle birlikte büyük) ayrıdır: JSON ya da XML kullanan HTML maliyetini ödemez. Hiçbiri ana pakete ve QueryTab parçasına girmez (`bun run build` çıktısında doğrulanır).

### 2.4 PDF aç / Kaydet (arka uç)

- **Yeni modül** `src-tauri/src/cell.rs`; `tauri-plugin-opener` eklenmez (Plan 27 Kararlar 9 ile aynı gerekçe).
- **Komutlar:**
  - `cell_open_pdf(data: String)`: `data` hücrenin ham `\x…` metnidir. Arka uç çözer, ilk baytların `%PDF-` olduğunu **yeniden** doğrular (yalnız PDF dışarıda açılır; `.exe`, `.html`, `.svg` gibi türler asla varsayılan uygulamaya verilmez), geçici dosyaya yazar ve `%SystemRoot%\explorer.exe "<yol>"` ile varsayılan uygulamada açar (beklenmez). Windows dışında `invalid`.
  - `cell_save_bytes(data: String, path: String)`: çözülen baytları kullanıcının yerel kayıt diyaloğunda seçtiği yola yazar (üzerine yazma onayını Windows diyaloğu verir). PDF ve Görsel sekmelerinde kullanılır. Yol mutlak olmalı; `"` ve denetim karakteri reddedilir.
  - Ön yüz sarıcıları `commands` nesnesine **eklenmez** (bkz. Spec düzeltmesi 2).
- **Çözme:** `decode_bytea_hex(&str) -> Result<Vec<u8>, &'static str>`: `\x` öneki zorunlu, çift uzunluk, büyük/küçük onaltılık; yeni crate (`hex`) eklenmez.
- **Boyut sınırı:** çözülmüş en çok 64 MiB (`too_large`).
- **Geçici dosya:**
  - Konum: `app.path().app_cache_dir()/cell-view` (Windows'ta `%LOCALAPPDATA%\com.pgtools.desktop\cell-view`; kullanıcıya özel ACL'yi devralır, ortak `%TEMP%` kökü değildir).
  - Ad: `cell-<uuid v4>.pdf`, `create_new` ile açılır (çakışma ya da önceden yerleştirilmiş dosya → hata).
  - Oluşturulan yollar yönetilen durumda (`CellTemp { files: Mutex<Vec<PathBuf>> }`) tutulur.
  - **Temizlik:** `RunEvent::Exit`'te bu sürecin oluşturduğu dosyalar silinir (en iyi çaba: PDF okuyucu dosyayı kilitliyse silme başarısız olur). Açılışta (`setup`) klasörde **24 saatten eski** `cell-*.pdf` dosyaları silinir: önceki kilitli/çöken oturumdan kalanlar temizlenir, aynı anda çalışan ikinci bir uygulama örneğinin yeni dosyalarına dokunulmaz.
- **Hata kodları:** `invalid` (bozuk onaltılık, PDF değil, geçersiz yol, Windows dışı), `too_large`, `io` (yazma/başlatma başarısız; ileti sabit Türkçe metin, yol ve OS ayrıntısı **içermez**).
- **Gizlilik:** değer, yol ve dosya adı `tracing`'e yazılmaz; ts-rs türü yoktur (binding değişmez).

### 2.5 Boyut sınırları

| İş | Sınır | Aşılınca |
|---|---|---|
| Metin görünümü | 1 MiB karakter (mevcut) | kırpılır + mevcut uyarı |
| Algılama (bytea öneki) | 1 KiB bayt | — (yalnız önek çözülür) |
| Onaltılık | ilk 16 MiB bayt | ilk 16 MiB gösterilir + "Yalnız ilk 16 MB gösteriliyor (toplam N MB)." |
| Görsel | 20 MiB çözülmüş | sekme devre dışı değil, görünümde ileti |
| JSON biçimlendirme + renklendirme | 4 MiB karakter | ileti |
| XML biçimlendirme + renklendirme | 4 MiB karakter | ileti |
| HTML biçimlendirme + renklendirme | 4 MiB karakter | ileti |
| HTML önizleme | 2 MiB karakter | önizleme anahtarı devre dışı + gerekçe |
| PDF aç / kaydet | 64 MiB çözülmüş | arka uç `too_large`; ön yüz düğmeleri baştan devre dışı + gerekçe |

- Sınır karşılaştırması **önce** yapılır (ham metin uzunluğundan: bytea'da `(len − 2) / 2` bayt); sınırı aşan değer hiç çözülmez/biçimlendirilmez.
- Sınır iletisi tek kalıptır: "Değer bu görünüm için çok büyük (N MB; sınır M MB). Metin ya da Onaltılık sekmesini kullanın ya da Kaydet… ile dosyaya yazın." (Kaydet yalnız bytea'da önerilir.)
- Uygulanabilirlik sınırdan bağımsızdır: büyük bir PNG için Görsel sekmesi etkin kalır, görünüm iletiyi gösterir (kullanıcı neden görmediğini anlar).

### 2.6 Paket bütçesi

- **Tembel parçalar:**
  - QueryTab parçası (mevcut): sekme çubuğu, `cell-view-model.ts`, `strings-cell.ts`, `CellPanel` değişiklikleri.
  - Yeni `CellViewers-*.js`: Onaltılık, Görsel, JSON/HTML kabı, PDF, `cell-format.ts`, `cell-ipc.ts`. `CellPanel` bunu yalnız Metin dışı bir sekme ilk kez gösterildiğinde `lazy()` ile yükler.
  - Yeni `cell-code-*.js` + `lang-json` parçası + `lang-xml` parçası + `lang-html` parçası (§2.3).
- **Ana paket maliyeti:** yalnız `commands.ts`'in `call` yardımcısını dışa aktarmak (birkaç bayt). Ölçülür ve telafi edilir (Plan 31 Task 7). Tavan yükseltilmez; yetmezse durulup sorulur.

### 2.7 Gizlilik ve erişilebilirlik

- Hücre değeri, çözülen baytlar, biçimlenmiş metin, geçici yol ve kayıt yolu `console.*`'a, `localStorage`'a, bildirime (toast) ve `tracing`'e yazılmaz. Hata bildirimleri sabit metindir.
- Sekme çubuğu `role="tablist"`; sekmeler `role="tab"`, `aria-selected`, gezici `tabindex`; Sol/Sağ ok, Home/End. Devre dışı sekmeye ok tuşuyla gelinir ama Enter/Space seçmez.
- Görünüm `role="tabpanel"` + `aria-labelledby`. Onaltılık liste `aria-label="Onaltılık görünüm"`; iframe `title="HTML önizlemesi (korumalı)"`; görsel `alt` = "Hücre görseli (PNG, 640 × 480)".

### 2.8 Spec düzeltmeleri (görev tanımına göre)

1. **Sekme çubuğu ana pakete girmez.** Görev tanımı sekme çubuğu ve metinlerin ana pakete maliyet getireceğini varsayıyordu; `CellPanel` zaten tembel QueryTab parçasındadır (`QueryTabs.tsx:23`). Telafi yalnız ana pakete gerçekten eklenen bayt (`call` dışa aktarımı) içindir.
2. **IPC sarıcıları `commands`'a eklenmez.** `commands.ts` ana pakettedir; iki sarıcı ~150 B getirirdi. Bunun yerine `call` dışa aktarılır ve sarıcılar tembel `src/features/query/cell-ipc.ts`'te durur.
3. **Katlanır JSON ağacı ayrı bileşen değildir.** CodeMirror `foldGutter()` (kurulu paket) `lang-json`'un katlama bilgisini kullanır; ağaç görünümü maliyetsiz karşılanır.
4. **"Kaydet…" Görsel sekmesinde de vardır.** Aynı `cell_save_bytes` komutu; ek arka uç kodu yoktur.
5. **PDF türü arka uçta yeniden doğrulanır**; dışarıda açma yalnız PDF içindir.

## 3. Test

- **Rust (`cargo test -p pg-tools cell`):** `decode_bytea_hex` (önek, tek uzunluk, geçersiz karakter, büyük/küçük harf, boş `\x`), `is_pdf`, `too_large` sınırı, `open_arg`/`save` yol denetimi (mutlak, `"`/denetim karakteri), geçici ad kalıbı, eski dosya süpürmesinin yalnız `cell-*.pdf` ve > 24 sa dosyalarını sildiği (`tempfile` ile, değişiklik zamanı enjekte edilerek), çıkış temizliğinin yalnız kayıtlı dosyaları sildiği.
- **Ön yüz (vitest, `node` ortamı):** `cell-view-model.test.ts` (algılama, uygulanabilirlik, gerekçeler, kendiliğinden seçim, hatırlama, sınır kararları), `cell-format.test.ts` (tam çözme, onaltılık satırları, UTF-8 baytları, base64, `formatHtml`, `formatXml`, `previewDoc`). CodeMirror ve iframe DOM'u node'da test edilmez.
- **Paket:** `bun run bundle:check` (tavanlar değişmeden PASS), `bun run build` çıktısında `lang-html`/`lang-json`/`lang-xml` parçalarının ana pakette ve QueryTab parçasında olmadığı.
- **Elle:** `docs/smoke-test.md` "## Hücre görüntüleyici (Plan 31)" altında `[—]` maddeleri; özellikle iframe'den Tauri IPC'ye erişilemediği, uzak isteklerin gitmediği, geçici PDF'lerin kapanışta silindiği.

## 4. Performans hedefi

- Metin sekmesi bugünkü kadar hızlıdır (ek parça yüklenmez).
- 1 MB JSON: biçimlendirme + ilk çizim < 300 ms. 16 MB bytea Onaltılık: ilk çizim < 300 ms (yalnız görünür satırlar üretilir). 20 MB görsel: < 1 sn.
- Hücreden hücreye geçişte algılama < 5 ms (yalnız 1 KB önek).

## 5. Bilinen sınırlar (ilk sürüm)

- Yalnız Query Tool; veri düzenleyicinin hücre paneli yoktur.
- `bytea_output = escape` oturumlarında bytea metin sayılır; Görsel/PDF uygulanmaz.
- Metin sütunundaki base64 görseller ve `data:` URL'leri Görsel sekmesine alınmaz. `bytea` içindeki UTF-8 metin (JSON/HTML) Metin sekmesinde `\x…` olarak görünür.
- PDF gömülü gösterilmez. Dışarıda açılan PDF, okuyucunun kendi güvenlik ayarlarıyla çalışır (ör. PDF içi JavaScript).
- Geçici PDF, okuyucu dosyayı kapanışta kilitli tutuyorsa silinemez; en geç 24 saat sonra bir sonraki açılışta silinir.
- HTML önizlemesi betik çalıştırmaz: betikle çizilen içerik boş görünür. Uzak görsel, stil ve yazı tipleri yüklenmez.
- XML sekmesi iyi biçimliliği doğrulamaz, ad alanlarını ve varlıkları çözmez, şemaya (XSD/DTD) göre denetlemez; önizleme yoktur.
- HTML biçimleyici tam bir HTML5 ayrıştırıcısı değildir (örtük kapanan `<p>`/`<li>` girintisi kayabilir); önizleme her zaman özgün metni kullanır.
- Panel genişliği iki adımlıdır (sürüklenerek boyutlandırma yok).

## 6. Takip işleri

- Veri düzenleyiciye aynı görüntüleyici (paylaşılan `CellViewers` parçası).
- `bytea` içindeki UTF-8 metni JSON/XML/HTML olarak açma; metin sütunundaki base64 görseller.
- Gömülü PDF görüntüleyici (pdf.js; ayrı tembel parça, CSP `worker-src` gerektirir).
