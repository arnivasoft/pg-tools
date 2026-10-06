// Biçimlendir komutu (tasarım §5.1) — saf yardımcılar: hangi aralık biçimlendirilir, sonuç nasıl uygulanır, imleç
// nereye gider. Biçimlendirici yalnızca boşlukları ve anahtar sözcük harfini (ASCII, aynı uzunluk) değiştirir; bu
// yüzden boşluk dışı karakter dizisi korunur ve imleç "önündeki boşluk dışı karakter sayısı" ile kesin eşlenir.

import { isolateHistory } from "@codemirror/commands";
import type { EditorState, TransactionSpec } from "@codemirror/state";
import { analyzeBefore, statementBounds } from "./completion";

const WS = /\s/;

/** Biçimlendirilecek aralık (UTF-16): boş olmayan seçim ya da tüm belge. */
export function formatRange(docLength: number, selFrom: number, selTo: number): { from: number; to: number; whole: boolean } {
  const a = Math.min(selFrom, selTo);
  const b = Math.max(selFrom, selTo);
  return a === b ? { from: 0, to: docLength, whole: true } : { from: a, to: b, whole: false };
}

/**
 * Biçimlendirilmiş metnin yerleştirilecek hali. Seçimde, seçimin baştaki/sondaki boşlukları korunur (biçimlendirici
 * baştaki boşluğu atar; seçim çevresine yapışmasın). Değişiklik yoksa `null`.
 */
export function formatReplacement(original: string, formatted: string, whole: boolean): string | null {
  let insert = formatted;
  if (!whole) {
    const lead = /^\s*/.exec(original)![0];
    const trail = /\s*$/.exec(original.slice(lead.length))![0];
    insert = lead + formatted.replace(/\s+$/, "") + trail;
  }
  return insert === original ? null : insert;
}

/**
 * `original` içindeki `offset`'in (UTF-16) `formatted` içindeki karşılığı. Önündeki boşluk dışı karakter sayısı
 * korunur; imleç bir belirtecin önündeyse o belirtecin önüne, boşluktaysa önceki belirtecin ardına gider. Boşluk dışı
 * diziler farklıysa (beklenmez) orantılı yedek.
 */
export function mapCursor(original: string, formatted: string, offset: number): number {
  const pos = Math.max(0, Math.min(offset, original.length));
  if (original.replace(/\s+/g, "").length !== formatted.replace(/\s+/g, "").length) {
    return original.length === 0 ? formatted.length : Math.round((pos / original.length) * formatted.length);
  }
  let k = 0;
  for (let i = 0; i < pos; i++) if (!WS.test(original[i]!)) k++;
  const beforeToken = pos < original.length && !WS.test(original[pos]!);
  if (beforeToken) {
    // (k+1)'inci boşluk dışı karakterin başı
    let seen = 0;
    for (let j = 0; j < formatted.length; j++) {
      if (WS.test(formatted[j]!)) continue;
      if (seen === k) return j;
      seen++;
    }
    return formatted.length;
  }
  if (k === 0) return 0;
  // k'ıncı boşluk dışı karakterin ardı
  let seen = 0;
  for (let j = 0; j < formatted.length; j++) {
    if (WS.test(formatted[j]!)) continue;
    seen++;
    if (seen === k) return j + 1;
  }
  return formatted.length;
}

/**
 * Biçimlendirme işlemi: tek değişiklik, ayrı geri alma adımı (`isolateHistory`), imleç `mapCursor` ile. Belge
 * `expectDoc`'tan farklıysa (istek sürerken yazıldı) ya da salt okunursa `null`.
 */
export function formatTransaction(state: EditorState, expectDoc: string, from: number, to: number, insert: string): TransactionSpec | null {
  if (state.readOnly || state.doc.toString() !== expectDoc) return null;
  const original = expectDoc.slice(from, to);
  const head = Math.min(Math.max(state.selection.main.head - from, 0), original.length);
  return {
    changes: { from, to, insert },
    selection: { anchor: from + mapCursor(original, insert, head) },
    scrollIntoView: true,
    userEvent: "input.format",
    annotations: isolateHistory.of("full"),
  };
}

const WORD_CH = /[\p{L}\p{N}_$]/u;
const OP_CH = /[+\-*/<>=~!@#%^&|`?:]/;

/** `p` bir belirtecin ortası mı (iki yanı aynı sınıf ya da önek/sayı bitişikliği). */
function midToken(doc: string, p: number): boolean {
  if (p <= 0 || p >= doc.length) return false;
  const a = doc[p - 1]!;
  const b = doc[p]!;
  if (WORD_CH.test(a) && (WORD_CH.test(b) || b === "'")) return true; // ad ortası, `E'…'`/`B'…'` öneki
  if (a === "&" && (b === "'" || b === '"')) return true; // `U&'…'`
  if (OP_CH.test(a) && OP_CH.test(b)) return true; // `--`, `/*`, `::`, `->>` …
  if ((a === "." && /\d/.test(b)) || (/\d/.test(a) && b === ".")) return true; // `1.5`
  return false;
}

/** `p` kodda mı: yorum, dize, dolar gövdesi ya da tırnaklı ad içinde değil ve bir belirtecin ortasında değil. */
function inCode(doc: string, p: number): boolean {
  if (p <= 0 || p >= doc.length) return true; // belge uçlarında parça ile belge aynı sözcüklenir
  const a = analyzeBefore(doc.slice(0, p));
  return a.context.kind !== "none" && !a.partial.startsWith('"') && !midToken(doc, p);
}

/**
 * Seçim biçimlendirmesinin güvenli aralığı (tasarım §5.2 belge düzeyinde). Seçimin ucu bir dize/yorum/dolar
 * gövdesi/tırnaklı ad içinde ya da belirteç ortasındaysa, biçimlendirici o metni kod sanıp değiştirebilirdi; uç
 * çevreleyen ifadenin sınırına genişletilir (`statementBounds`). Genişletme de güvenli değilse `null` (reddet).
 */
export function safeFormatSelection(doc: string, from: number, to: number): { from: number; to: number; snapped: boolean } | null {
  let a = from;
  let b = to;
  if (!inCode(doc, a)) a = statementBounds(doc, a).from;
  if (!inCode(doc, b)) b = statementBounds(doc, Math.max(a, b - 1)).to;
  if (!inCode(doc, a) || !inCode(doc, b) || a >= b) return null;
  return { from: a, to: b, snapped: a !== from || b !== to };
}
