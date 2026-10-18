// Gövde düzenleyicisinin (CodeMirror; Plan 23) saf metin yardımcıları (tembel parça; Solid/DOM/CodeMirror içe aktarmaz).
//
// Satır sonu: CodeMirror belgesi satır sonlarını LF'ye normalleştirir (CRLF ve tek `\r` → `\n`; `doc.toString()` LF
// döner). Ham değer (taslaktaki gövde) hiç değiştirilmez: düzenleyici `displayText(ham)` gösterir, kullanıcının
// düzenlemesini `applyDisplayEdit` ile ham metne en küçük değişiklik olarak uygular — dokunulmayan satır sonları (CRLF,
// LF, tek `\r`, karışık) bayt bayt korunur; eklenen satır sonları ham metnin ayırıcısıyla (`separatorOf`) yazılır.
// Dış değer değişimi (yeniden yükleme, geri al): `sql-body-sync.resyncState` — `displayDiff` ile seçim/kaydırma eşlenir, yeni
// `EditorState` `view.setState` ile yüklenir (geri alma geçmişi sıfırlanır; metin aynıysa dokunulmaz).
//
// Girinti: Tab iki boşluk (`INDENT`; sekme karakteri değil — PL/pgSQL gövdelerinde yaygın biçim ve `pg_get_functiondef`
// çıktısıyla uyumlu). CodeMirror `indentUnit` olarak kullanılır.

/** Değerin satır ayırıcısı (CRLF korunur; aksi halde `\n`, tek başına `\r` sıradan karakter). */
export const separatorOf = (v: string): "\r\n" | "\n" => (v.includes("\r\n") ? "\r\n" : "\n");

/** Girinti birimi (Tab / otomatik girinti). */
export const INDENT = "  ";

/** Düzenleyicinin göstereceği metin (CRLF ve tek `\r` → `\n`). */
export const displayText = (raw: string): string => raw.replace(/\r\n?/g, "\n");

/** Gövde dili → vurgulama kipi: `sql`/`plpgsql` (büyük-küçük harf duyarsız) SQL vurgusu, diğerleri düz metin. */
export function bodyLanguageMode(language: string): "sql" | "plain" {
  const l = language.trim().toLowerCase();
  return l === "sql" || l === "plpgsql" ? "sql" : "plain";
}

/** `a` → `b` arasındaki ortak önek/sonek dışı tek değişiklik (`[from, endA)` → `b[from, endB)`); eşitse `null`. */
function diffRange(a: string, b: string): { from: number; endA: number; endB: number } | null {
  if (a === b) return null;
  let from = 0;
  const max = Math.min(a.length, b.length);
  while (from < max && a.charCodeAt(from) === b.charCodeAt(from)) from++;
  let endA = a.length;
  let endB = b.length;
  while (endA > from && endB > from && a.charCodeAt(endA - 1) === b.charCodeAt(endB - 1)) {
    endA--;
    endB--;
  }
  return { from, endA, endB };
}

/** Görüntü metni `cur` → `next` için CodeMirror değişikliği (`{from, to, insert}`); fark yoksa `null`. */
export function displayDiff(cur: string, next: string): { from: number; to: number; insert: string } | null {
  const d = diffRange(cur, next);
  return d ? { from: d.from, to: d.endA, insert: next.slice(d.from, d.endB) } : null;
}

/** Görüntü ofseti → ham ofset (her CRLF görüntüde tek karakterdir; çift bölünmez). */
function rawOffset(raw: string, display: number): number {
  let r = 0;
  for (let d = 0; d < display && r < raw.length; d++) r += raw[r] === "\r" && raw[r + 1] === "\n" ? 2 : 1;
  return r;
}

/**
 * Görüntü metnindeki düzenlemeyi (`next`) ham metne uygular: `displayText(raw)` ile `next` arasındaki ortak önek/sonek
 * dışı tek değişiklik ham ofsetlere çevrilir; eklenen metindeki `\n`'ler ham ayırıcıyla yazılır. Fark yoksa `raw` aynen.
 */
export function applyDisplayEdit(raw: string, next: string): string {
  const d = diffRange(displayText(raw), next);
  if (!d) return raw;
  const sep = separatorOf(raw);
  const ins = next.slice(d.from, d.endB);
  const insert = sep === "\n" ? ins : ins.replace(/\n/g, sep);
  return raw.slice(0, rawOffset(raw, d.from)) + insert + raw.slice(rawOffset(raw, d.endA));
}
