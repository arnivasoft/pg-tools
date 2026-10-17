// Gövde düzenleyicisinin saf metin yardımcıları (tembel parça; Solid/CodeMirror içe aktarmaz).
// CodeMirror belgesinde her satır sonu (ayırıcı `\r\n` olsa da) TEK konumdur; metin ofsetleri buna çevrilir.

/** Değerin satır ayırıcısı (CRLF korunur; aksi halde `\n`, tek başına `\r` sıradan karakter). */
export const separatorOf = (v: string): "\r\n" | "\n" => (v.includes("\r\n") ? "\r\n" : "\n");

/** Metin ofsetinden belge konumu: `sep = "\r\n"` ise önceki her CRLF bir konum sayılır. */
function docPos(s: string, offset: number, sep: "\r\n" | "\n"): number {
  if (sep === "\n") return offset;
  let n = 0;
  for (let i = s.indexOf("\r\n"); i !== -1 && i + 2 <= offset; i = s.indexOf("\r\n", i + 2)) n++;
  return offset - n;
}

/**
 * `a` (belgenin metni) → `b` için en küçük tek değişiklik (ortak önek/sonek dışı), belge konumlarıyla; eşitse `null`.
 * `insert` `b`'nin özgün metnidir (CodeMirror ayırıcıya göre böler). Sınır bir CRLF çiftini bölmez.
 */
export function minimalChange(a: string, b: string, sep: "\r\n" | "\n" = "\n"): { from: number; to: number; insert: string } | null {
  if (a === b) return null;
  let start = 0;
  const max = Math.min(a.length, b.length);
  while (start < max && a.charCodeAt(start) === b.charCodeAt(start)) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a.charCodeAt(endA - 1) === b.charCodeAt(endB - 1)) {
    endA--;
    endB--;
  }
  if (sep === "\r\n") {
    // Başlangıç `\r|\n` arasındaysa çiftin önüne; bitiş arasındaysa çiftin arkasına (iki metinde de aynı sonek).
    if (start > 0 && a[start - 1] === "\r" && (a[start] === "\n" || b[start] === "\n")) start--;
    if (endA < a.length && a[endA] === "\n" && (a[endA - 1] === "\r" || b[endB - 1] === "\r")) {
      endA++;
      endB++;
    }
  }
  return { from: docPos(a, start, sep), to: docPos(a, endA, sep), insert: b.slice(start, endB) };
}
