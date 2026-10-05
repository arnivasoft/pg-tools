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

/** Hata konumunun (betik içi, UTF-16) aralığı: hata karakterinden sonraki kelime sınırına kadar. */
export function errorRange(
  text: string,
  failed: { charOffset: number; position: number | null },
): { from: number; to: number } | null {
  if (failed.position == null) return null;
  const from = scalarToUtf16(text, failed.charOffset + failed.position - 1);
  if (from >= text.length) return { from: text.length, to: text.length };
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
