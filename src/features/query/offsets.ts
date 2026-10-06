import type { Scope } from "../../ipc/bindings/Scope";

/** UTF-16 ofsetini Unicode scalar ofsetine çevirir. Sınır dışı kırpılır; vekil çiftinin ortası öncekine yuvarlanır. */
export function utf16ToScalar(text: string, utf16: number): number {
  const target = Math.max(0, Math.min(Math.trunc(utf16) || 0, text.length));
  let units = 0;
  let scalars = 0;
  for (const ch of text) {
    const next = units + ch.length;
    if (next > target) break;
    units = next;
    scalars += 1;
  }
  return scalars;
}

/** Unicode scalar ofsetini UTF-16 ofsetine çevirir. Sınır dışı kırpılır. */
export function scalarToUtf16(text: string, scalar: number): number {
  const target = Math.max(0, Math.trunc(scalar) || 0);
  let units = 0;
  let scalars = 0;
  for (const ch of text) {
    if (scalars >= target) break;
    units += ch.length;
    scalars += 1;
  }
  return units;
}

export function scopeFromEditor(
  text: string,
  selFrom: number,
  selTo: number,
  cursor: number,
  kind: "run" | "all",
): Scope {
  if (kind === "all") return { type: "all" };
  const a = Math.min(selFrom, selTo);
  const b = Math.max(selFrom, selTo);
  if (a !== b) return { type: "selection", start: utf16ToScalar(text, a), end: utf16ToScalar(text, b) };
  return { type: "atCursor", offset: utf16ToScalar(text, cursor) };
}

const WORD = /^[\p{L}\p{N}_]$/u;
const IDENT_START = /^[\p{L}_]$/u;
const IDENT_PART = /^[\p{L}\p{N}_$]$/u;

function codePointAt(text: string, i: number): string | undefined {
  const c = text.codePointAt(i);
  return c === undefined ? undefined : String.fromCodePoint(c);
}

/** `i`'de başlayan tek ad parçasının (düz ya da `""` kaçışlı çift tırnaklı) sonu; ad değilse null. */
function identEnd(text: string, i: number): number | null {
  const first = codePointAt(text, i);
  if (first === '"') {
    for (let j = i + 1; j < text.length; j++) {
      if (text[j] !== '"') continue;
      if (text[j + 1] !== '"') return j + 1;
      j++; // "" kaçışı
    }
    return null; // kapanmamış tırnak: eski davranış (tek karakter)
  }
  if (first === undefined || !IDENT_START.test(first)) return null;
  let j = i + first.length;
  for (let ch = codePointAt(text, j); ch !== undefined && IDENT_PART.test(ch); ch = codePointAt(text, j)) j += ch.length;
  return j;
}

function skipSpace(text: string, i: number): number {
  while (i < text.length && /\s/.test(text[i])) i++;
  return i;
}

/** `ad(.ad)*` nitelikli adın sonu (`şema.tablo`, `"A"."B"`, `şema.fonk(` → paranteze kadar); ad değilse null. */
function qualifiedNameEnd(text: string, from: number): number | null {
  let end = identEnd(text, from);
  if (end == null) return null;
  for (;;) {
    const dot = skipSpace(text, end);
    if (text[dot] !== ".") return end;
    const next = identEnd(text, skipSpace(text, dot + 1));
    if (next == null) return end; // ör. `t.*`
    end = next;
  }
}

/**
 * Hata konumunun (betik içi, UTF-16) aralığı. Konum bir ad zinciriyle başlıyorsa nitelikli adın tamamı
 * (`d001.vstokhar1` yalnızca `d001` değil); değilse hata karakterinden sonraki kelime sınırına kadar
 * (işleç/noktalama tek karakter).
 */
export function errorRange(
  text: string,
  failed: { charOffset: number; position: number | null },
): { from: number; to: number } | null {
  if (failed.position == null) return null;
  const from = scalarToUtf16(text, failed.charOffset + failed.position - 1);
  if (from >= text.length) return { from: text.length, to: text.length };
  const qualified = qualifiedNameEnd(text, from);
  if (qualified != null) return { from, to: qualified };
  let to = from;
  let first = true;
  for (const ch of text.slice(from)) {
    if (!WORD.test(ch)) {
      if (first) to += ch.length;
      break;
    }
    first = false;
    to += ch.length;
  }
  return { from, to };
}

export function stmtText(text: string, plan: { charOffset: number; charLen: number }): string {
  const from = scalarToUtf16(text, plan.charOffset);
  const to = scalarToUtf16(text, plan.charOffset + plan.charLen);
  return text.slice(from, to);
}
