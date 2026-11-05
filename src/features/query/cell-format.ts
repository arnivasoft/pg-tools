// Hücre görüntüleyicinin saf biçimleyicileri: bytea çözme, onaltılık satırlar,
// görsel data URL'i, PDF sürümü, HTML/XML biçimleyici ve korumalı önizleme belgesi.
// DOM API'si kullanılmaz; çıktı yalnız metindir.
import type { ImageKind } from "./cell-view-model";

// ---------------------------------------------------------------------------
// Bayt yardımcıları

/**
 * `\x…` bytea metnini çözer; en çok `maxBytes` bayt. Bozuk girdi → null.
 * `maxBytes` verilirse yalnız çözülen önek doğrulanır (hane sayısının çift olması dışında).
 */
export function decodeByteaHex(value: string, maxBytes = Infinity): Uint8Array | null {
  if (value.charCodeAt(0) !== 0x5c || value.charCodeAt(1) !== 0x78) return null;
  const digits = value.length - 2;
  if (digits % 2 !== 0) return null;
  const n = Math.min(digits / 2, Math.max(0, Math.floor(maxBytes)));
  const out = new Uint8Array(n);
  for (let i = 0; i < n; i++) {
    const hi = hexVal(value.charCodeAt(2 + i * 2));
    const lo = hexVal(value.charCodeAt(3 + i * 2));
    if (hi < 0 || lo < 0) return null;
    out[i] = (hi << 4) | lo;
  }
  return out;
}

function hexVal(c: number): number {
  if (c >= 0x30 && c <= 0x39) return c - 0x30;
  if (c >= 0x41 && c <= 0x46) return c - 0x41 + 10;
  if (c >= 0x61 && c <= 0x66) return c - 0x61 + 10;
  return -1;
}

/** Metni UTF-8 baytlarına çevirir; `maxBytes`'ta keser. */
export function utf8Bytes(text: string, maxBytes = Infinity): Uint8Array {
  const all = new TextEncoder().encode(text);
  return all.length > maxBytes ? all.subarray(0, Math.max(0, maxBytes)) : all;
}

export const HEX_ROW = 16;

export function hexRowCount(length: number): number {
  return Math.ceil(length / HEX_ROW);
}

const HEX_DIGITS = "0123456789ABCDEF";

function hexByte(b: number): string {
  return HEX_DIGITS[b >> 4] + HEX_DIGITS[b & 15];
}

/** `00000010  49 48 …  |IHDR…|` biçiminde tek satır. */
export function hexRow(bytes: Uint8Array, row: number): string {
  const start = row * HEX_ROW;
  const offset = start.toString(16).toUpperCase().padStart(8, "0");
  const cells: string[] = [];
  let ascii = "";
  for (let k = 0; k < HEX_ROW; k++) {
    const idx = start + k;
    if (idx < bytes.length) {
      const b = bytes[idx];
      cells.push(hexByte(b));
      ascii += b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : ".";
    } else {
      cells.push("  ");
    }
  }
  return `${offset}  ${cells.slice(0, 8).join(" ")}  ${cells.slice(8).join(" ")}  |${ascii}|`;
}

export function bytesToBase64(bytes: Uint8Array): string {
  const CHUNK = 0x8000;
  let bin = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK) as unknown as number[]);
  }
  return btoa(bin);
}

const IMAGE_MIME: Record<ImageKind, string> = {
  png: "image/png",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  bmp: "image/bmp",
  ico: "image/x-icon",
  svg: "image/svg+xml",
};

export function imageMime(kind: ImageKind): string {
  return IMAGE_MIME[kind];
}

export function dataUrl(kind: ImageKind, bytes: Uint8Array): string {
  return `data:${imageMime(kind)};base64,${bytesToBase64(bytes)}`;
}

/** `%PDF-1.7` → "1.7"; başlık eksikse null. */
export function pdfVersion(bytes: Uint8Array): string | null {
  const head = "%PDF-";
  if (bytes.length <= head.length) return null;
  for (let i = 0; i < head.length; i++) if (bytes[i] !== head.charCodeAt(i)) return null;
  let v = "";
  for (let i = head.length; i < bytes.length && i < head.length + 8; i++) {
    const c = bytes[i];
    if ((c >= 0x30 && c <= 0x39) || c === 0x2e) v += String.fromCharCode(c);
    else break;
  }
  return v === "" ? null : v;
}

// ---------------------------------------------------------------------------
// HTML / XML belirteçleyici (tek geçiş, geri izlemesiz)

type TokKind = "start" | "end" | "comment" | "cdata" | "doctype" | "pi" | "text" | "unclosed";

interface Tok {
  kind: TokKind;
  end: number;
  /** başlangıç/bitiş etiketi adı (HTML'de küçük harf) */
  name: string;
  selfClose: boolean;
}

interface Rules {
  xml: boolean;
}

function isNameStart(c: number): boolean {
  return (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a) || c === 0x5f || c === 0x3a || c > 0x7f;
}

function isWs(c: number): boolean {
  return c === 0x20 || c === 0x09 || c === 0x0a || c === 0x0d || c === 0x0c;
}

function readName(text: string, from: number, rules: Rules): { name: string; next: number } {
  let j = from;
  while (j < text.length) {
    const c = text.charCodeAt(j);
    if (isWs(c) || c === 0x2f || c === 0x3e) break;
    j++;
  }
  const raw = text.slice(from, j);
  return { name: rules.xml ? raw : raw.toLowerCase(), next: j };
}

/** `<` işaretinin geçerli bir yapı başlatıp başlatmadığı. */
function startsConstruct(text: string, i: number): boolean {
  const c = text.charCodeAt(i + 1);
  if (c === 0x21 || c === 0x3f) return true;
  if (c === 0x2f) return isNameStart(text.charCodeAt(i + 2));
  return isNameStart(c);
}

function nextToken(text: string, i: number, rules: Rules): Tok {
  const len = text.length;
  if (text.charCodeAt(i) !== 0x3c || !startsConstruct(text, i)) {
    let j = i + 1;
    for (;;) {
      j = text.indexOf("<", j);
      if (j < 0) return { kind: "text", end: len, name: "", selfClose: false };
      if (startsConstruct(text, j)) return { kind: "text", end: j, name: "", selfClose: false };
      j++;
    }
  }
  const c1 = text.charCodeAt(i + 1);
  if (c1 === 0x21) {
    if (text.startsWith("<!--", i)) return closed(text, i, "-->", "comment");
    if (text.startsWith("<![CDATA[", i)) return closed(text, i, "]]>", "cdata");
    return doctype(text, i);
  }
  if (c1 === 0x3f) return closed(text, i, "?>", "pi");
  if (c1 === 0x2f) {
    const { name } = readName(text, i + 2, rules);
    const e = text.indexOf(">", i + 2);
    if (e < 0) return { kind: "unclosed", end: len, name, selfClose: false };
    return { kind: "end", end: e + 1, name, selfClose: false };
  }
  const { name, next } = readName(text, i + 1, rules);
  let quote = 0;
  let prev = 0; // son boşluk olmayan karakter
  let unquoted = false; // tırnaksız öznitelik değerinin içinde miyiz
  for (let j = next; j < len; j++) {
    const c = text.charCodeAt(j);
    if (quote) {
      if (c === quote) quote = 0;
      continue;
    }
    if (unquoted) {
      if (isWs(c)) unquoted = false;
      else if (c === 0x3e) return { kind: "start", end: j + 1, name, selfClose: false };
      continue;
    }
    if ((c === 0x22 || c === 0x27) && prev === 0x3d) {
      quote = c;
      prev = c;
      continue;
    }
    if (c === 0x3e) {
      return { kind: "start", end: j + 1, name, selfClose: prev === 0x2f };
    }
    if (prev === 0x3d && !isWs(c)) {
      unquoted = true; // `href=/>`: `/` değerdir, etiketi kapatmaz
      prev = c;
      continue;
    }
    if (!isWs(c)) prev = c;
  }
  return { kind: "unclosed", end: len, name, selfClose: false };
}

function closed(text: string, i: number, terminator: string, kind: TokKind): Tok {
  const e = text.indexOf(terminator, i + 2);
  if (e < 0) return { kind: "unclosed", end: text.length, name: "", selfClose: false };
  return { kind, end: e + terminator.length, name: "", selfClose: false };
}

/** `<!DOCTYPE …>`; iç alt küme `[…]` ve içindeki tırnaklar tek parçadır. */
function doctype(text: string, i: number): Tok {
  let depth = 0;
  let quote = 0;
  for (let j = i + 2; j < text.length; j++) {
    const c = text.charCodeAt(j);
    if (quote) {
      if (c === quote) quote = 0;
    } else if (depth > 0 && c === 0x3c && text.startsWith("<!--", j)) {
      const e = text.indexOf("-->", j + 4);
      if (e < 0) break;
      j = e + 2;
    } else if (depth > 0 && c === 0x3c && text.startsWith("<?", j)) {
      const e = text.indexOf("?>", j + 2);
      if (e < 0) break;
      j = e + 1;
    } else if (depth > 0 && (c === 0x22 || c === 0x27)) quote = c;
    else if (c === 0x5b) depth++;
    else if (c === 0x5d && depth > 0) depth--;
    else if (c === 0x3e && depth === 0) {
      return { kind: "doctype", end: j + 1, name: "", selfClose: false };
    }
  }
  return { kind: "unclosed", end: text.length, name: "", selfClose: false };
}

const HTML_VOID = new Set([
  "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr",
]);
const HTML_RAW = new Set(["pre", "textarea", "script", "style"]);

/** HTML: `</name` ile eşleşen ilk bitiş etiketinin başlangıcı (küçük/büyük harf duyarsız). */
function findHtmlRawEnd(text: string, from: number, name: string): number {
  let j = from;
  for (;;) {
    j = text.indexOf("</", j);
    if (j < 0) return -1;
    if (text.substr(j + 2, name.length).toLowerCase() === name) {
      const after = text.charCodeAt(j + 2 + name.length);
      if (Number.isNaN(after) || isWs(after) || after === 0x3e || after === 0x2f) return j;
    }
    j += 2;
  }
}

/** `xml:space="preserve"` taşıyan başlangıç etiketi mi? */
function hasXmlSpacePreserve(tag: string): boolean {
  let p = tag.indexOf("xml:space");
  while (p >= 0) {
    let j = p + 9;
    while (isWs(tag.charCodeAt(j))) j++;
    if (tag.charCodeAt(j) === 0x3d) {
      j++;
      while (isWs(tag.charCodeAt(j))) j++;
      const q = tag.charCodeAt(j);
      if ((q === 0x22 || q === 0x27) && tag.startsWith("preserve", j + 1) && tag.charCodeAt(j + 9) === q) {
        return true;
      }
    }
    p = tag.indexOf("xml:space", p + 9);
  }
  return false;
}

/** XML: eşleşen bitiş etiketinin sonu (iç içe aynı adlı öğeler sayılır); yoksa -1. */
function findXmlElementEnd(text: string, from: number, name: string, rules: Rules): number {
  let depth = 1;
  let i = from;
  while (i < text.length) {
    const t = nextToken(text, i, rules);
    if (t.kind === "unclosed") return -1;
    if (t.name === name) {
      if (t.kind === "start" && !t.selfClose) depth++;
      else if (t.kind === "end" && --depth === 0) return t.end;
    }
    i = t.end;
  }
  return -1;
}

const MAX_INDENT = 32;

function format(text: string, rules: Rules): string {
  const lines: string[] = [];
  let depth = 0;
  let i = 0;
  // Görsel girinti MAX_INDENT ile sınırlıdır (çıktı boyutu O(n)); `depth` gerçek iç içeliği izler.
  const pads: string[] = [""];
  for (let k = 1; k <= MAX_INDENT; k++) pads.push(pads[k - 1] + "  ");
  const pad = () => pads[Math.min(depth, MAX_INDENT)];
  while (i < text.length) {
    const t = nextToken(text, i, rules);
    const raw = text.slice(i, t.end);
    switch (t.kind) {
      case "unclosed":
        lines.push(raw);
        break;
      case "text": {
        const s = raw.trim();
        if (s !== "") lines.push(pad() + s);
        break;
      }
      case "end":
        depth = Math.max(0, depth - 1);
        lines.push(pad() + raw);
        break;
      case "start": {
        if (t.selfClose || (!rules.xml && HTML_VOID.has(t.name))) {
          lines.push(pad() + raw);
          break;
        }
        if (!rules.xml && HTML_RAW.has(t.name)) {
          const e = findHtmlRawEnd(text, t.end, t.name);
          if (e < 0) {
            lines.push(pad() + text.slice(i));
            i = text.length;
            continue;
          }
          const close = text.indexOf(">", e);
          const end = close < 0 ? text.length : close + 1;
          lines.push(pad() + text.slice(i, end));
          i = end;
          continue;
        }
        if (rules.xml) {
          if (hasXmlSpacePreserve(raw)) {
            const end = findXmlElementEnd(text, t.end, t.name, rules);
            const stop = end < 0 ? text.length : end;
            lines.push(pad() + text.slice(i, stop));
            i = stop;
            continue;
          }
          // Yalnız metin içeren öğe tek satırda kalır.
          const a = nextToken(text, t.end, rules);
          if (a.kind === "end" && a.name === t.name && t.end < text.length) {
            lines.push(pad() + text.slice(i, a.end));
            i = a.end;
            continue;
          }
          if (a.kind === "text" && a.end < text.length) {
            const b = nextToken(text, a.end, rules);
            if (b.kind === "end" && b.name === t.name) {
              lines.push(pad() + text.slice(i, b.end));
              i = b.end;
              continue;
            }
          }
        }
        lines.push(pad() + raw);
        depth++;
        break;
      }
      default:
        lines.push(pad() + raw);
    }
    i = t.end;
  }
  return lines.join("\n");
}

/** Saf metin HTML biçimleyici (spec §2.3); hiçbir şey ayrıştırılmaz ya da çalıştırılmaz. */
export function formatHtml(text: string): string {
  return format(text, { xml: false });
}

/** Saf metin XML biçimleyici (spec §2.3 "XML"); `formatHtml` ile ortak belirteçleyici. */
export function formatXml(text: string): string {
  return format(text, { xml: true });
}

// ---------------------------------------------------------------------------
// Önizleme belgesi ve kaydetme adı

export const PREVIEW_CSP = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:";

/** Başa CSP `<meta>` ekler; değer `<!DOCTYPE …>` ile başlıyorsa doctype'tan sonra. */
export function previewDoc(html: string): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${PREVIEW_CSP}">`;
  const m = /^[\t\n\f\r ]*<!doctype[^>]*>/i.exec(html);
  if (m) return html.slice(0, m[0].length) + meta + html.slice(m[0].length);
  return meta + html;
}

export function suggestSaveName(columnName: string, ext: string): string {
  // eslint-disable-next-line no-control-regex
  const cleaned = columnName.replace(/[<>:"/\\|?*\x00-\x1f]/g, "_").trim();
  // 100 kod noktası (vekil çiftleri bölünmez); sonra sondaki nokta/boşluk kırpılır.
  const cut = Array.from(cleaned === "" ? "hucre" : cleaned).slice(0, 100).join("").replace(/[. ]+$/, "");
  let base = cut === "" ? "hucre" : cut;
  // Windows ayrılmış aygıt adları (uzantı benzeri sonekle de): CON, CON.txt, com1 ...
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.|$)/i.test(base)) base = base.replace(/^([^.]*)/, "$1_");
  return `${base}.${ext}`;
}

export function saveExt(f: { pdf: boolean; image: ImageKind | null }): string {
  if (f.pdf) return "pdf";
  if (f.image) return f.image === "jpeg" ? "jpg" : f.image;
  return "bin";
}
