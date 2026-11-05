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
  içerik bloğu) arayüze `reset` gönderilir ve o ana kadarki metin atılır.
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

## Sonraki adımlar (bu dilimde yok)
- Çok turlu sohbet (konuşma geçmişi, `cache_control`).
- Tool use ile katalog sorgulama (yalnız okuma; `strict: true` araçlar).
- Ayarlardan model/effort seçimi; kurumsal ağ geçidi (base URL) desteği.
