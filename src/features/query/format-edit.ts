// Biçimlendir komutu (tasarım §5.1) — saf yardımcılar: hangi aralık biçimlendirilir, sonuç nasıl uygulanır, imleç
// nereye gider. Biçimlendirici yalnızca boşlukları ve anahtar sözcük harfini (ASCII, aynı uzunluk) değiştirir; bu
// yüzden boşluk dışı karakter dizisi korunur ve imleç "önündeki boşluk dışı karakter sayısı" ile kesin eşlenir.

import { isolateHistory } from "@codemirror/commands";
import type { EditorState, TransactionSpec } from "@codemirror/state";

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
