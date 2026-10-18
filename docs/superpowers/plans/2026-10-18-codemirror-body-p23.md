# Fonksiyon Gövdesi için CodeMirror (Plan 23) Uygulama Planı

> **Ajan çalıştırıcılar için:** ZORUNLU ALT-BECERİ: superpowers:subagent-driven-development (önerilen) ya da superpowers:executing-plans ile görev görev uygulayın. Adımlar `- [ ]` ile izlenir.

**Hedef:** Function/Procedure formundaki gövde düzenleyicisini (`SqlBodyEditor`, şu an `textarea` — Plan 19 Karar R1) CodeMirror 6 ile değiştirmek: sözdizimi renklendirme (SQL / PL/pgSQL), satır numaraları, otomatik girinti, parantez eşleme, geri alma geçmişi. Gövde baytları ve klavye/erişilebilirlik davranışı korunur.

**Mimari:** Yeni `SqlBodyEditor` aynı props'u (`id`, `value`, `onChange`, `readOnly`, `describedBy`, `label`, `invalid`, `class`, `ref`) ve yeni bir `language` prop'unu alır. CodeMirror durumu **görüntü metni** (`displayText(ham)`, LF) tutar; her kullanıcı düzenlemesi mevcut `applyDisplayEdit(ham, görüntü)` ile ham değere en küçük değişiklik olarak yazılır (CRLF/tek `\r` korunur). Dışarıdan gelen değer değişimi (yeniden yükleme, geri al) programatik olarak uygulanır ve `onChange` tetiklemez. Ortak SQL görünümü `inspector/codemirror-setup.ts`'teki `sqlLookExtensions` ile paylaşılır.

**Teknoloji:** CodeMirror 6 (`@codemirror/{state,view,commands,language,lang-sql}`), SolidJS, vitest.

**Spec:** `docs/superpowers/specs/2026-10-16-object-dialogs-routines-types-design.md` §4 (Plan 19 notları, gövde düzenleyicisi; R1 backlog) ve paket kuralları `docs/superpowers/specs/2026-10-17-main-bundle-split-design.md` + `docs/testing.md` "Ana paket bütçesi".

## Kararlar

1. **Dil:** `language` küçük harf `sql` ya da `plpgsql` → `sqlLookExtensions()` (PostgreSQL lehçesi; PL/pgSQL için de yeterli renklendirme). Diğer diller (plpython3u, plperl, pltcl, plv8 …) → vurgusuz düz metin + aynı tema. Saf karar fonksiyonu `bodyLanguageMode(language): "sql" | "plain"`.
2. **Satır sonu:** Plan 19 kuralı aynen — ham gövde bayt bayt korunur; CodeMirror'a `displayText` verilir; kullanıcı düzenlemesinde `onChange(applyDisplayEdit(props.value, doc))`, fark yoksa çağrılmaz. Dış `value` değişince (ve görüntüsü editördekinden farklıysa) belge programatik olarak değiştirilir (`Annotation` ile işaretli; `onChange` yok; geri alma geçmişine girmez — `addToHistory.of(false)`).
3. **Klavye:** `indentWithTab` (Tab girinti/Shift+Tab geri), `defaultKeymap`, `historyKeymap`, `indentOnInput`, `bracketMatching`, `closeBrackets` **yok** (gövde metnini yazarken tahmin edilemez ekleme yapmasın), girinti birimi iki boşluk (`indentUnit.of("  ")`). **Tuzaktan çıkış:** CodeMirror'ın yerleşik "Esc ardından Tab odağı taşır" davranışı (`indentWithTab` belgesindeki tab-focus modu) korunur; ipucu metni (`tdes.body.keyboardHint`) aynen kalır. Salt okunurda Tab girinti yapmaz, odak doğrudan çıkar.
4. **Salt okunur:** `EditorState.readOnly.of(true)` + seçilebilir/kopyalanabilir (odaklanabilir kalır); `Compartment` ile `readOnly` reaktif değişir (editör yeniden kurulmaz).
5. **Erişilebilirlik:** `contentDOM`'a `aria-label`, `aria-describedby` (props + ipucu), `aria-invalid`, `aria-readonly`, `id` (etiket `for` ve odak isteği için: `id` contentDOM'a verilir; `ref` contentDOM'u döndürür — mevcut `getElementById(id).focus()` çağrıları çalışır). Geçersizken çerçeve `!border-danger`; görünüm `inputClass`'a yakın (çerçeve, yuvarlatma, yükseklik `!h-64` varsayılanı, yeniden boyutlandırma `resize-y`).
6. **Paket:** gövde düzenleyicisi yalnız tembel `RoutineForm` parçasında. CodeMirror modülleri QueryTab ile paylaşılan tembel bir parçaya düşebilir — kabul edilir; **koşul**: `bun run bundle:check` geçer (ana ≤ `mainMaxBytes`, açılış kümesi ≤ `startupMaxBytes`, preload tam `["solid","web"]`, yasak paket yok). Paylaşılan CodeMirror parçası ana modülleri (strings, ui/Field, Button…) içe aktarmamalı (Rolldown dersi): `SqlBodyEditor` stil sınıflarını kendisi tanımlar ya da `inputClass`'ı props/yerel sabitle alır; `bundle:check` çıktısında yeni preload olmamalı. QueryTab ilk açılış parça boyutları (önce/sonra) rapora yazılır.
7. **Kapsam dışı:** gövde içinde otomatik tamamlama, `format`, lint; serbest SQL alanları (default, WHEN, CHECK) textarea/input olarak kalır.

## Global Constraints

- Dal `feat/codemirror-body` (main'den). main'e birleştirme yalnızca kullanıcı onayıyla.
- Plan 19 gövde kuralları: gövde istemcide değiştirilmez/taranmaz/tırnaklanmaz; `console.*`/depolamaya yazılmaz; IME bileşimi sırasında özel tuş işlemi yok (CodeMirror yerel IME desteği; ek işleyici eklenirse `ev.isComposing || ev.keyCode === 229` koruması).
- Mevcut testler geçer; `sql-body-model.ts` (`displayText`, `applyDisplayEdit`) korunur; artık kullanılmayan `tabEdit` kaldırılırsa testleri de kaldırılır (gerekçe rapora).
- Doğrulama: `bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`. Yorumlar Türkçe. Commit eki: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- **Bayt korunumu:** CRLF gövde yükle → hiç dokunmadan önizleme boş plan (alter sabit noktası); bir satır ekle → yalnız o değişiklik, diğer CRLF'ler korunur; dış değer değişimi `onChange` tetiklemez (sahte kirli durum yok).
- **Odak ve klavye tuzağı:** Esc → Tab formdan çıkar; salt okunurda Tab doğrudan çıkar; etiket tıklaması ve alan hatası "odakla" bağlantısı editöre odaklanır.
- **Reaktiflik:** `readOnly`/`language` değişimi editörü yeniden kurmaz (geri alma geçmişi ve imleç korunur); sekme değişiminde (DesignerPanels gizli sekmeleri bağlı tutar) editör bozulmaz.
- **Paket:** açılış kümesi değişmez; yeni preload yok.

---

### Task 1: CodeMirror gövde düzenleyicisi

**Dosyalar:** Değiştir `src/features/designer/SqlBodyEditor.tsx` (yeniden yazım), `src/features/designer/sql-body-model.ts` (+ `bodyLanguageMode`, test), `src/features/designer/RoutineForm.tsx` (`language={d().language}`), gerekirse `src/app/strings-designer.ts`.

- [ ] **Adım 1: Başarısız testler (saf):** `bodyLanguageMode` (`sql`, `SQL`, `plpgsql`, `plpython3u`, boş → plain); `applyDisplayEdit` ile CodeMirror belge akışı senaryoları (CRLF gövdede satır ekleme/silme, karışık satır sonları, aynı metin → aynı ham) — mevcut testler genişletilir.
- [ ] **Adım 2–3:** uygula (Kararlar 1–6). Editör `onMount`'ta kurulur, `onCleanup`'ta `destroy`; `value` için `createEffect` programatik eşitleme; `Compartment`'lar: dil, salt okunur.
- [ ] **Adım 4:** typecheck, test, build, `bundle:check` (sayılar + QueryTab/RoutineForm parça boyutları önce/sonra rapora).
- [ ] **Adım 5: Commit** (`feat(designer): fonksiyon gövdesi için CodeMirror düzenleyicisi`).

### Task 2: Belgeler ve doğrulama

**Dosyalar:** `docs/smoke-test.md` ("Fonksiyon gövdesi düzenleyicisi (Plan 23)" bölümü: renklendirme sql/plpgsql/plpython, Tab/Shift+Tab, Esc→Tab çıkış, salt okunur (eklentiye ait/C fonksiyonu) seçme/kopyalama, CRLF gövde sabit noktası, geri al/yinele, etiket/hata odaklama, tema açık/koyu, uzun satır yatay kaydırma), spec §4 Plan 19 notlarındaki R1 backlog maddesini "Plan 23'te uygulandı" olarak güncelle, `docs/testing.md`.

- [ ] **Adım 1:** Bütünleşik doğrulama (`bun run typecheck`, `bun run test`, `bun run build`, `bun run bundle:check`).
- [ ] **Adım 2: Commit** (`docs(designer): Plan 23 smoke ve notlar`).

## Öz inceleme

- Kararlar 1–6 → Task 1; belgeler → Task 2. Tip: `SqlBodyEditorProps` + `language: string`; `ref` artık `HTMLElement` (contentDOM) — çağıranlar (RoutineForm) uyarlanır.
