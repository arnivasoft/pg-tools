# pg-tools — Alt Proje 3: Gelişmiş Editör Zekâsı Tasarımı

- **Tarih:** 2026-10-06
- **Durum:** Uygulandı (Plan 7)
- **Kapsam:** Query Tool editöründe alias/CTE/alt sorgu çözümlemeli tamamlama, sunucuda `PREPARE`/Parse ile yazarken hata altı çizme, kendi sözcük çözümleyicimiz üstünde SQL biçimlendirici
- **Önceki alt proje:** Query Tool (`2026-10-06-query-tool-design.md`, Plan 5–6) — `main`'de.

---

## 1. Amaç ve bağlam

Query Tool'un temel tamamlaması yalnızca `şema.` / `tablo.` öneklerini ve `FROM`/`JOIN` sonrası tablo adlarını tanır; alias'ları çözmez, hatalar ancak çalıştırınca görülür, biçimlendirme yoktur. Bu alt proje, pgAdmin'in en çok kullanılan üç editör yardımını hafif ve PostgreSQL'e sadık biçimde ekler.

### 1.1 Kullanıcıdan gelen kararlar

| Konu | Karar |
|---|---|
| Özellikler | **Alias çözümleme** (CTE ve alt sorgu sütunları dahil), **hata altı çizme**, **SQL biçimlendirici** |
| Kapsam dışı (bu alt proje) | Join önerisi (yabancı anahtardan `JOIN … ON …`) |
| Hata tespiti | **Sunucuda Parse/`PREPARE`** — ifade çalıştırılmaz; PostgreSQL'in kendi hatası ve konumu |
| Biçimlendirici | **Kendi sözcük çözümleyicimiz** (`pgt-query::lex`) üstünde kural tabanlı; yeni bağımlılık yok |

### 1.2 Başarı ölçütleri

| Ölçüt | Hedef |
|---|---|
| Alias'lı kolon tamamlaması (`select o.` → `orders o`) ilk öneriler | < 150 ms (önbellek sıcak), < 400 ms (soğuk, LAN) |
| Hata altı çizme gecikmesi (yazma durduktan sonra) | ≤ 700 ms (600 ms debounce + Parse) |
| Biçimlendirme (10.000 satırlık betik) | < 200 ms, arayüz donmaz |
| Biçimlendirici güvenliği | Biçimlendirme **yalnızca boşlukları ve anahtar kelime harf büyüklüğünü** değiştirir; belirteç dizisi aynı kalır (otomatik doğrulanır, bozulma = biçimlendirme iptal) |
| Hata denetiminin yan etkisi | Kullanıcı transaction'ı ve oturum durumu **hiç** değişmez; ifade hiçbir koşulda çalıştırılmaz |
| Bellek | Boşta < 150 MB değişmez; Query Tool açık + denetim < 250 MB |

### 1.3 Kapsam dışı

- Join önerisi, anlamsal yeniden yazım (ör. `SELECT *` açma), SQL "lint" kuralları (stil uyarıları).
- Fonksiyon imza ipuçları (parametre yardımı) ve fonksiyon adı tamamlaması (`CompletionKind::Function` hâlâ üretilmez).
- Çevrimdışı (sunucusuz) sözdizimi denetimi — bağlı değilken hata altı çizme yapılmaz.

---

## 2. Mimari

```
crates/pgt-query/src/scope.rs      (yeni) sorgu bloğu kapsam çözümleyici: FROM/JOIN öğeleri, alias'lar, CTE'ler, alt sorgular
crates/pgt-query/src/format.rs     (yeni) biçimlendirici (lex tabanlı, belirteç korumalı)
crates/pgt-query/src/check.rs      (yeni) Parse tabanlı hata denetimi (oturum bağlantısında, korumalı)
crates/pgt-query/src/session.rs    QueryManager::{check, resolve_columns}; tamamlamaya alias isteği
src-tauri/src/query.rs             qt_check, qt_format; qt_complete'e yeni istek türü
src/features/query/                editor.ts (lint katmanı, biçimlendir komutu), completion.ts (alias bağlamı)
```

İlke: **ayrıştırma mantığı Rust'ta**, tek kaynaklı (`pgt-query::lex` zaten ifade ayırıcı ve sınıflandırıcıda kullanılıyor). Ön yüz yalnızca imleç bağlamını gönderir ve sonucu gösterir. Katalog ve sunucu çağrıları Query Tool oturumunun kendi bağlantısında, **meşgul değilken** ve korumalarla yapılır (Plan 5'teki tamamlama kuralları).

---

## 3. Alias çözümleme (tamamlama)

### 3.1 Kapsam çözümleyici (`scope.rs`, saf)

Girdi: ifade metni + imleç ofseti (Unicode scalar). Çıktı: imlecin bulunduğu **sorgu bloğunun** görünür ilişki öğeleri:

```rust
enum ScopeItem {
    Relation { alias: Option<String>, schema: Option<String>, name: String },   // from app.orders o
    Cte      { name: String, body: Range<usize>, columns: Option<Vec<String>> }, // with x(a,b) as (…) — açık kolon listesi varsa
    Subquery { alias: String, body: Range<usize>, columns: Option<Vec<String>> },// from (select …) s(a,b)
    Function { alias: Option<String>, name: String },                            // from generate_series(1,3) g — kolon: alias adı
}
```

Kurallar (sözcüksel, belirteç ve parantez derinliği ile):
- Sorgu bloğu = imlecin derinliğindeki en yakın `SELECT … ` ile onu çevreleyen parantez; iç içe alt sorgular kendi bloğudur, dış bloğun öğeleri (korelasyon) de görünür (iç blok önce).
- `FROM`/`JOIN`/`,` ardından öğe: `[şema.]ad [[AS] alias [(kolonlar)]]`, `(alt sorgu) [AS] alias [(kolonlar)]`, `LATERAL`, `ONLY`, `fonksiyon(...) [AS] alias`. Çift tırnaklı adlar tırnaksız karşılaştırılır; tırnaksız adlar küçük harfe katlanır (PostgreSQL kuralı).
- `WITH [RECURSIVE] ad [(kolonlar)] AS [NOT] [MATERIALIZED] (…)` CTE'leri bloğun ve iç blokların kapsamındadır.
- `UPDATE t [AS] a … FROM …`, `DELETE FROM t [AS] a USING …`, `INSERT INTO t [AS] a … RETURNING` da kapsam üretir.
- Belirsizlik/bozuk sözdizimi: o noktaya kadar toplanan öğeler döner; hata atılmaz.

### 3.2 Kolonların bulunması

`a.` öneki için önce kapsamda alias/ad eşleşmesi aranır:
- `Relation` → katalogdan kolonlar (Plan 5 `Columns` isteği; `schema` yoksa oturumun `search_path`'iyle çözülen ilişki — `pg_catalog.to_regclass` ile, `search_path`'e dokunmadan).
- `Cte`/`Subquery` → açık kolon listesi varsa o; yoksa gövde metni **Parse** edilir (§4'teki korumalarla, çalıştırılmaz) ve tokio-postgres `Statement::columns()` adları/türleri kullanılır. Gövde başka CTE'lere bağlıysa önce o CTE'ler eklenerek `WITH … <gövde>` olarak hazırlanır.
- `Function` → alias adı tek kolon olarak (kolon listesi varsa o).
- Eşleşme yoksa Plan 5 davranışı (`a` bir ilişki adıdır).

Önbellek: Parse ile bulunan kolonlar `(gövde metni karması)` anahtarıyla 60 sn; katalog kolonları Plan 5 önbelleği. DDL ve `search_path` değişikliği önbelleği temizler (Plan 5 kuralı).

### 3.3 Arayüz

`completion.ts` `relationMember` bağlamında önce `CompletionRequest::Scoped { sql: <imleçteki ifade>, offset, prefix }` gönderir; arka uç alias'ı çözüp kolonları döner. Ek öneri: `FROM`'dan önce yazılan `select ` sonrası (alias'sız) — kapsamdaki tüm ilişkilerin kolonları `alias.kolon` biçiminde (en çok 500). Ofsetler Unicode scalar (Plan 6 dönüşümleri).

---

## 4. Hata altı çizme (`check.rs`)

### 4.1 Akış

- Editörde metin değişip **600 ms** durunca ve sekme oturumu **açık + boşta** iken: imlecin bulunduğu ifade ve görünür alandaki ifadeler (en çok 20, toplam en çok 64 KB) `qt_check(session, sql, scope)` ile gönderilir.
- Arka uç her ifade için **Parse** (protokol düzeyi `prepare`, adsız/kapanan deyim) yapar; ifade **çalıştırılmaz**. Hata → `{ stmtIndex, position, message, sqlstate, severity }`; başarı → işaret yok.
- Ön yüz hatayı CodeMirror lint katmanında dalgalı alt çizgi + gutter işareti + üzerine gelince ileti olarak gösterir (Plan 6 hata işareti stilini paylaşır). Metin değişince ilgili işaretler kaldırılır; sonuç eskiyse (sürüm sayacı) yok sayılır.

### 4.2 Korumalar (bağlayıcı)

1. **Hiçbir şey çalıştırılmaz:** yalnızca Parse; `Bind`/`Execute` yok. Yardımcı (utility) ifadeler (DDL, `VACUUM`, `SET`…) Parse'ta yalnızca ham ayrıştırılır — bu davranış PG 10–18'de testle sabitlenir.
2. **Kullanıcı transaction'ına dokunulmaz:** oturum `Idle` ise Parse doğrudan (örtük transaction); `InTransaction` ise `SAVEPOINT pgt_check` … `ROLLBACK TO` + `RELEASE` içinde (Parse'ın aldığı `AccessShareLock`'lar geri bırakılır); `Failed` ise **denetim yapılmaz**.
3. **Kilit bekleme yok:** Parse'tan önce `SET LOCAL lock_timeout = '200ms'` (savepoint içinde) ya da örtük transaction'da tek ifadelik `SET lock_timeout` + sıfırlama; zaman aşımı → o ifade için işaret yok (sessiz).
4. **Kullanıcının işini bekletmez:** denetim oturum kilidini en çok **1 sn** tutar; `run`, `more`, `export` başlarsa sıradaki denetim atlanır; çalışan denetim iptal belirteciyle kesilir. Kilitli/üretim sunucuda da güvenlidir (salt Parse).
5. **Sahte hatalar:** aynı betikte önceki bir `Ddl`/`CREATE TEMP` ifadesinin yarattığı nesneye başvuran ifadelerde isim hataları (`42P01`, `42703`, `42883`, `42704`, `3F000`) **bilgi** düzeyinde (gri noktalı) gösterilir, hata değil. `$1` gibi parametreler için "tür belirlenemedi" (`42P18`) gösterilmez.
6. **Hassas metin:** denetim istekleri günlüğe yazılmaz; geçmişe girmez.

### 4.3 Kapatma

Ayarlar'da "Yazarken hata denetimi" (varsayılan açık) ve sekme bağlam şeridinde geçici kapatma; bağlı olmayan sekmede devre dışı.

---

## 5. Biçimlendirici (`format.rs`)

### 5.1 Davranış

- `Ctrl+Shift+F`: seçim varsa seçimi, yoksa tüm metni biçimlendirir; tek geri alma adımı; imleç en yakın belirtece taşınır.
- Seçenekler (Ayarlar): anahtar kelime harf büyüklüğü (BÜYÜK / küçük / koru — varsayılan BÜYÜK), girinti (2 / 4 boşluk), virgül konumu (sonda / başta), satır uzunluğu (80–200, varsayılan 120).
- Kurallar: ana yan tümceler yeni satırda (`SELECT`, `FROM`, `WHERE`, `GROUP BY`, `HAVING`, `WINDOW`, `ORDER BY`, `LIMIT`, `OFFSET`, `RETURNING`, `UNION/INTERSECT/EXCEPT`, `VALUES`, `SET`, `ON CONFLICT`); `JOIN` türleri yeni satırda, `ON`/`USING` aynı satırda; seçim listesi ve `SET` atamaları satır uzunluğu aşılınca her öğe ayrı satıra; alt sorgu ve CTE gövdeleri girintili; `CASE` / `WHEN` / `ELSE` / `END` hizalı; `AND`/`OR` uzun koşullarda satır başında; ifadeler arasında bir boş satır.
- **Dokunulmayanlar:** dize, dolar tırnaklı gövde (`$$…$$`, fonksiyon gövdeleri), çift tırnaklı tanımlayıcı, yorum metni (yerleri korunur), tanımlayıcıların harf büyüklüğü, `::` tür dönüşümleri, operatörler.
- Tanımadığı yapılar (ör. `CREATE FUNCTION … BEGIN ATOMIC`, `COPY`, `DO`): o ifade yalnızca boşlukları sadeleştirilerek (satır içi çoklu boşluk → tek) bırakılır.

### 5.2 Güvenlik sözleşmesi

Biçimlendirme sonucunda **anlamlı belirteç dizisi** (yorumlar dahil, boşluk hariç; anahtar kelimeler harf büyüklüğünden bağımsız karşılaştırılır) girdiyle aynı olmalıdır; değilse işlem iptal edilir ve kullanıcıya "biçimlendirilemedi" bildirilir (metin değişmez). Biçimlendirme **eşkuvvetlidir** (`f(f(x)) == f(x)`). Bu iki özellik fuzz testleriyle (rastgele birleştirilmiş SQL parçaları + Plan 5 ayırıcı test korpusu) doğrulanır.

### 5.3 Yer

Saf Rust (`pgt-query::format`), `qt_format(sql, options)` Tauri komutu (oturum gerektirmez, async). 10.000 satırlık betik < 200 ms.

---

## 6. Hata yönetimi ve güvenlik

- `qt_check` ve alias çözümlemesi hatalarını kullanıcıya **toast** olarak göstermez; başarısızlık = işaret/öneri yok (sessiz bozulma). Yalnızca geliştirici günlüğüne (SQL metni olmadan) hata kodu.
- Parse ile çözülen alt sorgu/CTE kolonları için korumalar §4.2 ile aynıdır.
- Hiçbir yeni komut kullanıcı SQL'ini çalıştırmaz; kilitli sunucu kuralları etkilenmez.

## 7. Test stratejisi

- **Kapsam çözümleyici:** tablo testleri (alias'lı/alias'sız, `AS`'li, şemalı, tırnaklı, `JOIN … USING`, virgül listesi, `LATERAL`, iç içe alt sorgu ve korelasyon, `WITH RECURSIVE`, CTE kolon listesi, `UPDATE … FROM`, `DELETE … USING`, `INSERT … RETURNING`, fonksiyon kaynağı, bozuk/eksik sözdizimi, imleç farklı bloklarda).
- **Denetim (PG 10–18 matrisi):** sözdizimi hatası konumu; olmayan tablo/kolon; DDL ifadesinin Parse'ta yan etkisi olmadığı (`CREATE TABLE` denetimi tablo oluşturmaz, `DROP TABLE` denetimi silmez — her sürümde); `InTransaction` iken denetim sonrası transaction ve kilitler değişmez (`pg_locks`), `Failed` iken denetim yapılmaz; kilit bekleyen tabloda 200 ms içinde vazgeçer; denetim sürerken `run` gecikmesi ≤ 1 sn; önceki DDL'e bağlı isim hataları bilgi düzeyinde.
- **Biçimlendirici:** altın dosyalar (girdi → beklenen çıktı) + belirteç eşitliği + eşkuvvetlilik fuzz testi + dolar tırnaklı/yorumlu/Unicode metin korunumu + 10.000 satır süre testi.
- **Ön yüz:** lint durumu modeli (sürüm sayacı, eski sonuçların yok sayılması, oturum meşgulken istek yok), alias tamamlama bağlam testleri, biçimlendirme komutunun tek geri alma adımı (saf yardımcılarla).

## 8. Açık riskler

1. **Parse'ın yan etkisiz olduğu varsayımı:** PostgreSQL'de Parse/analiz aşaması tablo kilidi alır ve bazı fonksiyonları (ör. `regclass` sabitleri) çözer; yazma yapmaz. Sürüm matrisi testi bunu sabitler; aksi görülürse ilgili ifade türleri denetimden çıkarılır.
2. **Parse kilidi:** `ACCESS EXCLUSIVE` kilitli bir tabloya başvuran ifade denetimde 200 ms bekler ve sessizce atlanır.
3. **Sözcüksel kapsam çözümleme sınırları:** tam bir ayrıştırıcı değildir; karmaşık sözdiziminde öneri eksik olabilir (hata değil, eksik öneri).
4. **Biçimlendirici ile kişisel stil:** kural seti sabit ve az seçeneklidir; beğenilmeyen çıktı için "koru" seçenekleri ve tek geri alma adımı vardır.
5. **Denetimin oturum kilidini tutması:** en çok 1 sn; `busy` riski `run` başlangıcında denetimi iptal ederek azaltılır.
