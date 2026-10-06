/**
 * Arka uç sözcük çözümleyicisinin (`crates/pgt-query/src/lex.rs`) birebir TypeScript karşılığı. Ofsetler UTF-16'dır;
 * arka uçta "bayt >= 0x80" olan her şey burada "kod birimi >= 0x80"dir (ASCII dışı karakterlerin tüm baytları/kod
 * birimleri aynı sınıfa düşer, bu yüzden belirteç sınırları iki tarafta aynı karakterlere denk gelir).
 *
 * Kurallar (arka uçla aynı tutulmalı; ortak tablo: `crates/pgt-query/tests/fixtures/split-cases.json`):
 * - Boşluk yalnızca ASCII boşluğudur (` `, `\t`, `\n`, `\f`, `\r`); diğer her şey belirteçtir.
 * - `'…'` dizesinde `''` her zaman kaçıştır. Dize, hemen önündeki (boşluksuz) **tek karakterlik** `E`/`e` sözcüğüyle
 *   birlikte bir kaçış dizesidir (`E'…'`): içinde `\` sonraki karakteri atlar. `name'…'` (uzun sözcüğün son harfi)
 *   kaçış dizesi değildir. `U&'…'`, `B'…'`, `X'…'`, `N'…'` sıradan dizedir (ters bölü kaçış değildir).
 * - `"…"` tırnaklı ad (`""` kaçış), `--` satır yorumu, iç içe `/* … *\/`, `$etiket$ … $etiket$`.
 * - Sözcük: `[A-Za-z_\x80+0-9][A-Za-z0-9_$\x80+]*`; diğer tek karakterler noktalama (`(`/`)` derinliği izlenir).
 * - Kapanmamış dize/yorum/ad/dolar gövdesi metin sonuna kadar sürer (`closed: false`).
 */

export type SqlTokKind = "word" | "ident" | "str" | "dollar" | "comment" | "punct";

export interface SqlTok {
  kind: SqlTokKind;
  start: number;
  end: number;
  /** Tokenden önceki parantez derinliği (`(` dış derinlikte, `)` azaltıldıktan sonraki derinlikte). */
  depth: number;
  /** Dize/ad/yorum/dolar gövdesi kapandı mı (satır yorumu: ardından satır sonu geliyor mu). Diğerleri için `true`. */
  closed: boolean;
}

const isWs = (c: number): boolean => c === 0x20 || c === 0x09 || c === 0x0a || c === 0x0c || c === 0x0d;
const isAlpha = (c: number): boolean => (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a);
const isDigit = (c: number): boolean => c >= 0x30 && c <= 0x39;
const identStart = (c: number): boolean => isAlpha(c) || c === 0x5f || c >= 0x80;
const identPart = (c: number): boolean => isAlpha(c) || isDigit(c) || c === 0x5f || c === 0x24 || c >= 0x80;
const tagPart = (c: number): boolean => isAlpha(c) || isDigit(c) || c === 0x5f || c >= 0x80;

/** `$tag$` açılışı ise son `$`'ın indeksi; `$1` (konum parametresi) açılış değildir. */
function dollarTagEnd(s: string, i: number): number {
  let j = i + 1;
  if (j < s.length && identStart(s.charCodeAt(j))) {
    while (j < s.length && tagPart(s.charCodeAt(j))) j++;
  }
  return s.charCodeAt(j) === 0x24 ? j : -1;
}

export function lexSql(s: string): SqlTok[] {
  const n = s.length;
  const out: SqlTok[] = [];
  let i = 0;
  let depth = 0;
  while (i < n) {
    const c = s.charCodeAt(i);
    if (isWs(c)) {
      i++;
      continue;
    }
    let start = i;
    let kind: SqlTokKind;
    let closed = true;
    if (c === 0x2d && s.charCodeAt(i + 1) === 0x2d) {
      while (i < n && s.charCodeAt(i) !== 0x0a) i++;
      kind = "comment";
      closed = i < n;
    } else if (c === 0x2f && s.charCodeAt(i + 1) === 0x2a) {
      let nest = 1;
      i += 2;
      while (i < n && nest > 0) {
        if (s.charCodeAt(i) === 0x2f && s.charCodeAt(i + 1) === 0x2a) {
          nest++;
          i += 2;
        } else if (s.charCodeAt(i) === 0x2a && s.charCodeAt(i + 1) === 0x2f) {
          nest--;
          i += 2;
        } else i++;
      }
      kind = "comment";
      closed = nest === 0;
    } else if (c === 0x27) {
      // E'..': hemen önceki bitişik tek karakterlik E/e sözcüğü bu dizeye katılır; ters bölü kaçıştır.
      const prev = out[out.length - 1];
      const esc =
        prev !== undefined && prev.kind === "word" && prev.end === i && prev.end - prev.start === 1 &&
        (s[prev.start] === "e" || s[prev.start] === "E");
      if (esc) start = out.pop()!.start;
      i++;
      closed = false;
      while (i < n) {
        const d = s.charCodeAt(i);
        if (esc && d === 0x5c) {
          i += 2;
          continue;
        }
        if (d === 0x27) {
          if (s.charCodeAt(i + 1) === 0x27) {
            i += 2;
            continue;
          }
          i++;
          closed = true;
          break;
        }
        i++;
      }
      out.push({ kind: "str", start, end: Math.min(i, n), depth, closed });
      i = Math.min(i, n);
      continue;
    } else if (c === 0x22) {
      i++;
      closed = false;
      while (i < n) {
        if (s.charCodeAt(i) === 0x22) {
          if (s.charCodeAt(i + 1) === 0x22) {
            i += 2;
            continue;
          }
          i++;
          closed = true;
          break;
        }
        i++;
      }
      kind = "ident";
    } else if (c === 0x24 && dollarTagEnd(s, i) >= 0) {
      const tagEnd = dollarTagEnd(s, i);
      const tag = s.slice(i, tagEnd + 1);
      const p = s.indexOf(tag, tagEnd + 1);
      closed = p >= 0;
      i = closed ? p + tag.length : n;
      kind = "dollar";
    } else if (identStart(c) || isDigit(c)) {
      i++;
      while (i < n && identPart(s.charCodeAt(i))) i++;
      kind = "word";
    } else {
      i++;
      if (c === 0x28) {
        out.push({ kind: "punct", start, end: i, depth, closed });
        depth++;
        continue;
      }
      if (c === 0x29) depth = Math.max(0, depth - 1);
      kind = "punct";
    }
    out.push({ kind, start, end: Math.min(i, n), depth, closed });
  }
  return out;
}

const isWordTok = (s: string, t: SqlTok, w: string): boolean =>
  t.kind === "word" && s.slice(t.start, t.end).toLowerCase() === w;

/**
 * İfade ayırıcı `;` belirteçleri (arka uç `split.rs` ile aynı kural): derinlik 0'da ve `BEGIN ATOMIC … END` gövdesi
 * dışında. Yorumlar atlanır (`BEGIN /*c*\/ ATOMIC` yine gövde açar).
 */
export function separatorTokens(s: string, toks: SqlTok[] = lexSql(s)): SqlTok[] {
  const seps: SqlTok[] = [];
  let atomic = 0;
  let prevBegin = false;
  for (const t of toks) {
    if (t.kind === "comment") continue;
    if (t.kind === "punct" && s[t.start] === ";" && t.depth === 0 && atomic === 0) {
      seps.push(t);
      prevBegin = false;
      continue;
    }
    if (t.kind === "word") {
      if (atomic === 0) {
        if (prevBegin && isWordTok(s, t, "atomic")) atomic = 1;
      } else if (isWordTok(s, t, "case")) atomic++;
      else if (isWordTok(s, t, "end")) atomic--;
    }
    prevBegin = isWordTok(s, t, "begin");
  }
  return seps;
}
