// Hücre görüntüleyici saf modeli: algılama (yalnız önek), uygunluk, kendiliğinden seçim, sütun başına hatırlama,
// boyut sınırları ve sekme gezinmesi. DOM/IPC yok; hücre değeri hiçbir yere yazılmaz (gizlilik).
import type { Column } from "../../ipc/bindings/Column";
import { isJsonType } from "./grid-model";
import { tcell } from "./strings-cell";

export type CellFormat = "text" | "hex" | "image" | "json" | "xml" | "html" | "pdf";
export const CELL_FORMATS: readonly CellFormat[] = ["text", "hex", "image", "json", "xml", "html", "pdf"];
export type ImageKind = "png" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg";

export const MiB = 1024 * 1024;
export const LIMITS = {
  prefixBytes: 1024,
  hexBytes: 16 * MiB,
  imageBytes: 20 * MiB,
  codeChars: 4 * MiB,
  previewChars: 2 * MiB,
  fileBytes: 64 * MiB,
} as const;

/** `bytea` sütununda `\x` önekli (hex çıktı) değer mi? `escape` çıktısı metin sayılır. */
export function isByteaHex(value: string, typeName: string): boolean {
  return typeName === "bytea" && value.startsWith("\\x");
}

/** `\x` önekli değerin bayt sayısı (ham uzunluktan; çözmez). */
export function byteaLength(value: string): number {
  return (value.length - 2) >> 1;
}

function hexNibble(c: number): number {
  if (c >= 48 && c <= 57) return c - 48;
  if (c >= 97 && c <= 102) return c - 87;
  if (c >= 65 && c <= 70) return c - 55;
  return -1;
}

/** İlk `maxBytes` baytı çözer; geçersiz onaltılık ya da tek haneli kuyruk → null. */
export function decodePrefix(value: string, maxBytes: number): Uint8Array | null {
  const digits = value.length - 2;
  if (digits < 0 || (digits & 1) === 1) return null;
  const n = Math.min(maxBytes, digits >> 1);
  const out = new Uint8Array(n);
  for (let i = 0; i < n; i++) {
    const hi = hexNibble(value.charCodeAt(2 + 2 * i));
    const lo = hexNibble(value.charCodeAt(3 + 2 * i));
    if (hi < 0 || lo < 0) return null;
    out[i] = (hi << 4) | lo;
  }
  return out;
}

function startsWithBytes(b: Uint8Array, sig: readonly number[], at = 0): boolean {
  if (b.length < at + sig.length) return false;
  for (let i = 0; i < sig.length; i++) if (b[at + i] !== sig[i]) return false;
  return true;
}

/** İlk baytlardan görsel türü; SVG dahil. Kısa dizilerde taşmaz. */
export function sniffImage(bytes: Uint8Array): ImageKind | null {
  if (startsWithBytes(bytes, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return "png";
  if (startsWithBytes(bytes, [0xff, 0xd8, 0xff])) return "jpeg";
  if (startsWithBytes(bytes, [0x47, 0x49, 0x46, 0x38, 0x37, 0x61]) || startsWithBytes(bytes, [0x47, 0x49, 0x46, 0x38, 0x39, 0x61])) return "gif";
  if (startsWithBytes(bytes, [0x52, 0x49, 0x46, 0x46]) && startsWithBytes(bytes, [0x57, 0x45, 0x42, 0x50], 8)) return "webp";
  if (startsWithBytes(bytes, [0x42, 0x4d])) return "bmp";
  if (startsWithBytes(bytes, [0x00, 0x00, 0x01, 0x00])) return "ico";
  if (bytes.length >= 4 && sniffSvg(new TextDecoder("utf-8").decode(bytes.subarray(0, LIMITS.prefixBytes)))) return "svg";
  return null;
}

function isSpace(code: number): boolean {
  return code === 0xfeff || code === 0xa0 || (code >= 9 && code <= 13) || code === 32;
}

/** Baştaki boşluk ve BOM'u atlayıp ilk anlamlı karakterin dizinini verir (kopya üretmez). */
function skipLead(s: string, from = 0): number {
  let i = from;
  while (i < s.length && isSpace(s.charCodeAt(i))) i++;
  return i;
}

/** SVG koklaması: BOM, boşluk, `<?xml?>`, yorum ve `<!DOCTYPE>` atlanır; kalan `<svg` ile başlamalı. Çağıran önek verir. */
export function sniffSvg(text: string): boolean {
  let i = skipLead(text);
  for (;;) {
    if (text.startsWith("<?", i)) {
      const end = text.indexOf("?>", i + 2);
      if (end < 0) return false;
      i = skipLead(text, end + 2);
    } else if (text.startsWith("<!--", i)) {
      const end = text.indexOf("-->", i + 4);
      if (end < 0) return false;
      i = skipLead(text, end + 3);
    } else if (text.slice(i, i + 9).toLowerCase() === "<!doctype") {
      const end = text.indexOf(">", i + 9);
      if (end < 0) return false;
      i = skipLead(text, end + 1);
    } else break;
  }
  if (text.slice(i, i + 4).toLowerCase() !== "<svg") return false;
  const next = text.charCodeAt(i + 4);
  return Number.isNaN(next) || isSpace(next) || next === 0x3e || next === 0x2f;
}

export function isPdf(bytes: Uint8Array): boolean {
  return startsWithBytes(bytes, [0x25, 0x50, 0x44, 0x46, 0x2d]);
}

/** Metin türünde JSON adayı: kırpılmış değer `{`/`[` ile başlar; ≤ 4 MiB ise `JSON.parse` geçmeli, daha büyükse ilk karakter yeter. */
export function looksLikeJson(value: string): boolean {
  const i = skipLead(value);
  const c = value.charCodeAt(i);
  if (c !== 0x7b && c !== 0x5b) return false;
  if (value.length > LIMITS.codeChars) return true;
  try {
    JSON.parse(i === 0 ? value : value.slice(i));
    return true;
  } catch {
    return false;
  }
}

/** Kırpılmış değer `<` ile başlar ve `>` içerir (HTML/XML/SVG kaynağı). İyi biçimlilik denetlenmez. */
export function looksLikeMarkup(value: string): boolean {
  const i = skipLead(value);
  return value.charCodeAt(i) === 0x3c && value.indexOf(">", i + 1) >= 0;
}

/** UTF-8 bayt uzunluğu (kod birimi döngüsü; kopya üretmez). Eşleşmeyen vekil 3 bayt (U+FFFD) sayılır. */
export function utf8Length(s: string): number {
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c < 0x80) n += 1;
    else if (c < 0x800) n += 2;
    else if (c >= 0xd800 && c <= 0xdbff) {
      const d = s.charCodeAt(i + 1);
      if (d >= 0xdc00 && d <= 0xdfff) {
        n += 4;
        i++;
      } else n += 3;
    } else n += 3;
  }
  return n;
}

export interface CellFacts {
  isNull: boolean;
  bytea: boolean;
  jsonType: boolean;
  xmlType: boolean;
  /** bytea: çözülmüş bayt sayısı (ham uzunluktan). Metin: `charLength` ile aynı (O(1)); yalnız SVG görsel boyut korumasında kullanılır
   *  (UTF-16 birimi ≤ bayt; sınır kararları tutarlı biçimde `charLength`'e göre verilir). Metin için bayt toplamı gösterilecekse
   *  gösterildiği yerde tam hesaplanır (HexView, ImageView). */
  byteLength: number;
  charLength: number;
  image: ImageKind | null;
  pdf: boolean;
  json: boolean;
  markup: boolean;
  badHex: boolean;
}

export function cellFacts(value: string | null, column: Pick<Column, "typeName">): CellFacts {
  const jsonType = isJsonType(column.typeName);
  const xmlType = column.typeName === "xml";
  if (value === null) {
    return { isNull: true, bytea: false, jsonType, xmlType, byteLength: 0, charLength: 0, image: null, pdf: false, json: false, markup: false, badHex: false };
  }
  const bytea = isByteaHex(value, column.typeName);
  if (bytea) {
    const prefix = decodePrefix(value, LIMITS.prefixBytes);
    return {
      isNull: false,
      bytea,
      jsonType,
      xmlType,
      byteLength: byteaLength(value),
      charLength: value.length,
      image: prefix ? sniffImage(prefix) : null,
      pdf: prefix ? isPdf(prefix) : false,
      json: false,
      markup: false,
      badHex: prefix === null,
    };
  }
  return {
    isNull: false,
    bytea,
    jsonType,
    xmlType,
    byteLength: value.length,
    charLength: value.length,
    image: sniffSvg(value.slice(0, LIMITS.prefixBytes)) ? "svg" : null,
    pdf: false,
    json: jsonType || looksLikeJson(value),
    markup: looksLikeMarkup(value),
    badHex: false,
  };
}

export type Availability = { enabled: boolean; reason: string | null };
const ON: Availability = { enabled: true, reason: null };
const off = (reason: string): Availability => ({ enabled: false, reason });

export function availability(f: CellFacts): Record<CellFormat, Availability> {
  if (f.isNull) {
    const n = off(tcell.reason.isNull);
    return { text: n, hex: n, image: n, json: n, xml: n, html: n, pdf: n };
  }
  const markupOk = !f.bytea && f.markup;
  return {
    text: ON,
    hex: ON,
    image: f.image ? ON : off(f.badHex ? tcell.reason.badHex : tcell.reason.notImage),
    json: f.json ? ON : off(tcell.reason.notJson),
    xml: f.xmlType || markupOk ? ON : off(f.bytea ? tcell.reason.onlyText : tcell.reason.notXml),
    html: markupOk ? ON : off(f.bytea ? tcell.reason.onlyText : tcell.reason.notMarkup),
    pdf: f.pdf ? ON : off(!f.bytea ? tcell.reason.onlyBytea : f.badHex ? tcell.reason.badHex : tcell.reason.notPdf),
  };
}

/** Kendiliğinden seçim (hatırlanan yokken): boş dize → metin; json/jsonb → xml → bytea görsel → bytea PDF → metin. Hafızaya yazmaz. */
export function autoFormat(f: CellFacts): CellFormat {
  if (f.isNull || f.charLength === 0) return "text";
  if (f.jsonType) return "json";
  if (f.xmlType) return "xml";
  if (f.bytea && f.image) return "image";
  if (f.bytea && f.pdf) return "pdf";
  return "text";
}

/** Hatırlanan seçim uygulanabiliyorsa o; değilse kendiliğinden kural (hatırlanan silinmez; hafızaya yazılmaz). */
export function pickFormat(f: CellFacts, remembered: CellFormat | undefined): CellFormat {
  if (remembered && availability(f)[remembered].enabled) return remembered;
  return autoFormat(f);
}

export function columnKey(c: Pick<Column, "name" | "typeOid">): string {
  return `${c.typeOid}:${c.name}`;
}

export interface FormatMemory {
  get(key: string): CellFormat | undefined;
  set(key: string, f: CellFormat): void;
}
export function createFormatMemory(): FormatMemory {
  const m = new Map<string, CellFormat>();
  return { get: (k) => m.get(k), set: (k, f) => void m.set(k, f) };
}
/** Oturum boyu tekil hafıza (localStorage yok). */
export const formatMemory: FormatMemory = createFormatMemory();

export type SizeVerdict = { ok: true } | { ok: false; size: number; limit: number };

/** Sınır ham uzunluktan, çözmeden önce denetlenir. `hex` her boyutta ok (kısmi gösterim görünümün işidir). */
export function sizeVerdict(format: Exclude<CellFormat, "text">, f: CellFacts): SizeVerdict {
  let size: number;
  let limit: number;
  switch (format) {
    case "hex":
      return { ok: true };
    case "image":
      size = f.byteLength;
      limit = LIMITS.imageBytes;
      break;
    case "pdf":
      size = f.byteLength;
      limit = LIMITS.fileBytes;
      break;
    default:
      size = f.charLength;
      limit = LIMITS.codeChars;
  }
  return size > limit ? { ok: false, size, limit } : { ok: true };
}

export function previewAllowed(f: CellFacts): Availability {
  return f.charLength <= LIMITS.previewChars ? ON : off(tcell.html.previewTooLarge(formatMb(LIMITS.previewChars)));
}

export function fileActionAllowed(f: CellFacts): Availability {
  if (!f.bytea) return off(tcell.reason.onlyBytea);
  if (f.badHex) return off(tcell.reason.badHex);
  if (f.byteLength > LIMITS.fileBytes) return off(tcell.tooLarge(formatMb(f.byteLength), formatMb(LIMITS.fileBytes)));
  return ON;
}

/** Sekme çubuğu gezinmesi: devre dışı sekmeler dahil, dairesel. */
export function nextTab(current: CellFormat, key: "ArrowLeft" | "ArrowRight" | "Home" | "End"): CellFormat {
  const n = CELL_FORMATS.length;
  if (key === "Home") return CELL_FORMATS[0];
  if (key === "End") return CELL_FORMATS[n - 1];
  const i = CELL_FORMATS.indexOf(current);
  return CELL_FORMATS[(i + (key === "ArrowRight" ? 1 : n - 1)) % n];
}

/** MiB cinsinden, tr-TR, en çok 1 ondalık ("1,5"). */
export function formatMb(bytes: number): string {
  return (bytes / MiB).toLocaleString("tr-TR", { maximumFractionDigits: 1 });
}
