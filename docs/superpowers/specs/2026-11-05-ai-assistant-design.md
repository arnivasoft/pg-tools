# Yapay zekâ asistanı — tasarım (ilk dilim)

## Amaç
Query Tool içinde Claude ile: doğal dilden SQL yazma, seçili SQL'i açıklama, hata veren SQL'i düzeltme.
Asistan yalnız metin üretir; SQL asla kendiliğinden çalıştırılmaz ("Editöre ekle" → kullanıcı çalıştırır,
mevcut risk/kilit/üretim kapıları geçerlidir).

## Mimari
- `crates/pgt-ai`: `model` (IPC türleri `AiMode`, `AiRequest`, `AiEvent`), `prompt` (sabit sistem istemi +
  etiketli kullanıcı mesajı; 400.000 karakter üstü reddedilir, kırpılmaz), `sse` (SSE ayrıştırma, olay yorumu),
  `client` (ham HTTP: Rust için resmî Anthropic SDK'sı yok).
- İstek: `POST https://api.anthropic.com/v1/messages`, `anthropic-version: 2026-10-21`, model `claude-opus-5-5`,
  `stream: true`, `thinking: adaptive`, `output_config.effort: medium`, `max_tokens: 16000`,
  `fallbacks: "default"` (`anthropic-beta: server-side-fallback-2026-07-01`). Yedek modele geçişte (`fallback`
  içerik bloğu) arayüze `fallback` olayı gider; o ana kadarki metin geçerlidir, yedek model kaldığı yerden sürer.
- HTTP: `reqwest` 0.13 (HTTP/1.1, `rustls-no-provider` + uygulamanın ring sağlayıcısı, `rustls-platform-verifier`
  ile işletim sistemi sertifika deposu, `system-proxy`). İstemci ilk istekte kurulur (açılışı yavaşlatmaz).
- Komutlar (`src-tauri/src/ai.rs`): `ai_key_status`, `ai_key_set`, `ai_key_clear`, `ai_ask(id, request, onEvent)`,
  `ai_cancel(id)`. Sayfa yenilemede tüm akışlar iptal edilir.
- Anahtar: Windows Kimlik Bilgisi Yöneticisi, `pg-tools` / `ai:anthropic` (`Credentials::ai_key`); arayüze dönmez.
- Günlük: yalnız HTTP durumu, süre, belirteç sayıları ve durma nedeni. İstem, yanıt ve anahtar yazılmaz.

## Bağlam (ön yüz, `QueryTab.aiContext`)
- SQL: seçim; yoksa imleçteki ifade (`qt_analyze`); düzeltmede son çalıştırmanın hata veren ifadesi.
- Hata: başarısız ifadenin `message/detail/hint`'i, yoksa son hata notu.
- Şema: oturumun arama yolundaki ilişki adları (`qt_complete relations`), istemde adı geçen ilişkilerin kolonları
  (en çok 20) ve SQL kapsamındaki kolonlar (`scoped`). Oturum meşgulse şema özeti olmadan gönderilir.
- Satır verisi gönderilmez. Panel bu davranışı kullanıcıya açıkça yazar.

## Ön yüz
- Sonuç alanında "Asistan" sekmesi; `AiPane` ve `ai-ipc.ts` tembel parçadadır (ana paket bütçesi değişmez).
- `ai-model.ts`: yanıtı metin/kod bloklarına bölme (akışta açık blokta "Editöre ekle" gizli), şema metni.

## Çok turlu sohbet (ikinci dilim)
- Sekme başına bir sohbet. Geçmiş arka uçta (`pgt_ai::Conversations`, bellekte, en çok 20 sohbet, sohbet başına
  40 tur / 1,2 M karakter) tutulur; sayfa yenilenince silinir. Ön yüz sohbeti modül kapsamında tutar: panel ya da
  sekme değişince kaybolmaz, süren yanıt arka planda tamamlanır.
- Geçmiş yalnız eklenerek büyür: önceki mesajlar ve asistan içerik blokları (thinking blokları imzalarıyla) hiç
  değiştirilmeden geri gönderilir. Bu, Opus 5.5'in "preserved thinking" geçmiş denetimini (2026-08-31 sonrası
  hesaplarda düzenlenmiş geçmiş 400 döner) ve istem önbelleğini (`cache_control: ephemeral`, otomatik) korur.
- Şema özeti yalnız ilk turda ya da değiştiğinde gönderilir; sonraki turlar önceki turlardaki özeti görür.
- Başarısız, iptal edilen, reddedilen (`refusal`) ya da görünür metni olmayan tur geçmişe girmez; arayüz aynı
  kuralı uygular (`isCommitted`) ve bunu turun altında yazar. `ai_ask` arayüzün bildiği tur sayısını (`turns`)
  alır; arka uç geçmişiyle uyuşmazsa (sohbet bellekten düştü) `ai_stale` döner, kullanıcı yeni sohbet başlatır.
- Yanıt ortasında yedek modele geçişte gelen metin geçerli kalır (silinmez, `fallback` olayı yalnız not düşer);
  geçmişe eklenirken son `fallback` bloğundan önceki thinking/tool_use blokları atlanır, metin blokları kalır.
- "Yeni sohbet" (`ai_forget`) arka uç geçmişini siler.

## Katalog araçları (üçüncü dilim)
- Üç salt okunur araç (`pgt_ai::tools`): `list_relations(schema, name_contains)`, `describe_relation(schema, name)`
  (kolonlar, kısıtlar, gelen FK'ler, indeksler, görünüm tanımı), `search_columns(name_contains)`. Her biri sabit,
  `$n` parametreli katalog sorgusudur; model serbest SQL gönderemez, satır verisi okunmaz. Bölümler listelenmez;
  sonuç 200 satırla, uzun metin 8.000 karakterle sınırlı (kırpma açıkça yazılır). PG 10+ (entegrasyon testi
  `crates/pgt-ai/tests/catalog_tools.rs`, `pgt_ai_tools` veritabanı).
- Araçlar sekmenin veritabanında, uygulamanın katalog bağlantı havuzunda koşar (`ConnectionManager::run`, 35 sn):
  kullanıcının sorgu oturumu ve açık transaction'ı etkilenmez. Sekmenin veritabanı yoksa araç hata döner.
- Tanımlar `strict: true`, `additionalProperties: false`, tüm alanlar zorunlu (null olabilir); akışta
  `eager_input_streaming: true`. Girdi katı ayrıştırılır ve şemaya göre doğrulanır; geçersizse araç çalışmaz,
  modele `is_error` sonucu döner. `max_tokens`/`refusal` ile biten yanıttaki çağrılar çalıştırılmaz ve geçmişten
  atılır (sonuçsuz `tool_use` API'de reddedilir).
- Araç kümesi her istekte aynıdır (değişirse thinking blokları ve önbellek bozulur); son tur `tool_choice: none`
  ile gider (en çok 6 araç turu). Zorunlu `tool_choice` (`any`/`tool`) Opus 5.5'te 400 döndüğü için kullanılmaz.
- Bir tur birden çok mesaj olabilir (asistan → araç sonuçları → ... → son asistan); hepsi geçmişe eklenir,
  paralel çağrıların sonuçları tek kullanıcı mesajında döner. Tur sayacı mesaj sayısından ayrı tutulur.
- Arayüz yanıtın üstünde çalıştırılan araçları ("Tablo incelendi: shop.orders") ve belirteç satırında önbellekten
  okunan girdiyi gösterir. Günlüğe araç adı, süre ve başarı yazılır; girdi ve sonuç yazılmaz.

## "Asistana sor" girişleri (dördüncü dilim)
- Yeni kipler: `plan` (EXPLAIN planını yorumla; `extra` = plan JSON'u), `review` (değişiklik betiğinin kilit,
  yeniden yazım ve veri kaybı riskleri; betik `<script>` bölümünde), `optimize` (yavaş sorgu; `extra` =
  pg_stat_statements istatistikleri). `AiRequest.extra` yeni alan.
- Girişler:
  - İletiler paneli (sekmede hata varken): "Asistanla düzelt" → `fix` (hata veren ifade ve hata metni).
  - Plan paneli (plan varken): "Planı açıkla" → `plan` (EXPLAIN metni + JSON).
  - Şema karşılaştırma betik diyaloğu: "Riskleri açıkla" → betik hedefte yeni sorgu sekmesinde açılır
    (çalıştırılmaz), diyalog kapanır, `review` isteği bırakılır.
  - Dashboard sorgu istatistikleri ayrıntısı: "Asistanla iyileştir" → sorgu yeni sekmede açılır, `optimize`
    isteği istatistiklerle bırakılır. Yalnız veritabanı panosunda (sunucu panosunda sorgunun veritabanı bilinmez;
    katalog araçları yanlış veritabanına bakardı).
- Bekleyen istek sekme modelinde durur (`TabModel.aiIntent`); QueryTab paneli Asistan'a çevirir, AiPane isteği alıp
  gönderir (anahtar yoksa kaydedilene dek bekler). Taslağa yazılmaz.
- Paket bütçesi: istek mantığı ana pakette değildir. `ai-intent.ts`'i şema farkı ve dashboard yalnız tıklanınca
  dinamik yükler, QueryTab yalnız türünü kullanır; giriş metinleri her özelliğin kendi metin dosyasındadır. Ortak
  statik içe aktarma ortak parça yaratır ve ana paketin ön yükleme listesini büyütür (ilk denemede +322 / +78 bayt).

## Sonraki adımlar
- EXPLAIN aracı (READ ONLY transaction, tek ifade, zaman sınırı).
- Ayarlardan model/effort seçimi; kurumsal ağ geçidi (base URL) desteği.
