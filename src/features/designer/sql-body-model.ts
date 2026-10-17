// Gövde düzenleyicisinin (textarea; Plan 19 R1) saf metin yardımcıları (tembel parça; Solid/DOM içe aktarmaz).
//
// Satır sonu: textarea değeri satır sonlarını LF'ye normalleştirir (CRLF ve tek `\r` → `\n`). Ham değer (taslaktaki
// gövde) hiç değiştirilmez: düzenleyici `displayText(ham)` gösterir, kullanıcının düzenlemesini `applyDisplayEdit` ile
// ham metne en küçük değişiklik olarak uygular — dokunulmayan satır sonları (CRLF, LF, tek `\r`, karışık) bayt bayt
// korunur; eklenen satır sonları ham metnin ayırıcısıyla (`separatorOf`) yazılır.
//
// Girinti: Tab iki boşluk ekler (`INDENT`; sekme karakteri değil — PL/pgSQL gövdelerinde yaygın biçim ve
// `pg_get_functiondef` çıktısıyla uyumlu); seçim varsa dokunduğu satırları girintiler. Shift+Tab satır başından en çok
// iki boşluk ya da bir sekme kaldırır.

/** Değerin satır ayırıcısı (CRLF korunur; aksi halde `\n`, tek başına `\r` sıradan karakter). */
export const separatorOf = (v: string): "\r\n" | "\n" => (v.includes("\r\n") ? "\r\n" : "\n");

/** Tab ile eklenen girinti. */
export const INDENT = "  ";

/** Textarea'nın göstereceği metin (CRLF ve tek `\r` → `\n`). */
export const displayText = (raw: string): string => raw.replace(/\r\n?/g, "\n");

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
  const cur = displayText(raw);
  if (cur === next) return raw;
  let start = 0;
  const max = Math.min(cur.length, next.length);
  while (start < max && cur.charCodeAt(start) === next.charCodeAt(start)) start++;
  let endA = cur.length;
  let endB = next.length;
  while (endA > start && endB > start && cur.charCodeAt(endA - 1) === next.charCodeAt(endB - 1)) {
    endA--;
    endB--;
  }
  const sep = separatorOf(raw);
  const insert = sep === "\n" ? next.slice(start, endB) : next.slice(start, endB).replace(/\n/g, sep);
  return raw.slice(0, rawOffset(raw, start)) + insert + raw.slice(rawOffset(raw, endA));
}

/** Tab/Shift+Tab düzenlemesi: görüntü metninde `[from, to)` → `insert`, ardından seçim `[selStart, selEnd]`. */
export interface TabEdit {
  from: number;
  to: number;
  insert: string;
  selStart: number;
  selEnd: number;
}

/**
 * Tab (`outdent = false`) / Shift+Tab (`true`) düzenlemesi; `text` görüntü metnidir (yalnız `\n`). Seçimsiz Tab imlece
 * `INDENT` ekler; seçimli Tab ve her Shift+Tab seçimin dokunduğu satırları (seçim sonu satır başındaysa o satır hariç)
 * girintiler/geri alır. Kaldırılacak girinti yoksa `null`.
 */
export function tabEdit(text: string, start: number, end: number, outdent: boolean): TabEdit | null {
  if (!outdent && start === end) {
    return { from: start, to: start, insert: INDENT, selStart: start + INDENT.length, selEnd: start + INDENT.length };
  }
  const lineStart = text.lastIndexOf("\n", start - 1) + 1;
  const effEnd = end > start && text[end - 1] === "\n" ? end - 1 : end;
  const nl = text.indexOf("\n", effEnd);
  const lineEnd = nl === -1 ? text.length : nl;
  const lines = text.slice(lineStart, lineEnd).split("\n");
  // Her satırın (görüntü) başlangıcı ve eklenen (+) / kaldırılan (−) karakter sayısı.
  const starts: number[] = [];
  const deltas: number[] = [];
  let pos = lineStart;
  const out = lines.map((l) => {
    starts.push(pos);
    pos += l.length + 1;
    if (!outdent) {
      deltas.push(INDENT.length);
      return INDENT + l;
    }
    const n = l.startsWith("\t") ? 1 : l.startsWith("  ") ? 2 : l.startsWith(" ") ? 1 : 0;
    deltas.push(-n);
    return l.slice(n);
  });
  if (deltas.every((d) => d === 0)) return null;
  const total = deltas.reduce((a, b) => a + b, 0);
  const map = (p: number, keepAtLineStart: boolean): number => {
    if (p > lineEnd) return p + total;
    let k = starts.length - 1;
    while (k > 0 && starts[k]! > p) k--;
    const before = deltas.slice(0, k).reduce((a, b) => a + b, 0);
    const ls = starts[k]!;
    const d = deltas[k]!;
    if (d >= 0) return p + before + (p === ls && keepAtLineStart ? 0 : d);
    return p + before - Math.min(-d, p - ls);
  };
  const selStart = map(start, true);
  const selEnd = start === end ? selStart : Math.max(selStart, map(end, false));
  return { from: lineStart, to: lineEnd, insert: out.join("\n"), selStart, selEnd };
}
