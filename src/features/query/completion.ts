/**
 * Katalog tamamlama: saf bağlam çözümleme (`contextAt`) + enjekte edilen `fetch`/`isBusy` ile
 * CodeMirror `CompletionSource` fabrikası (düğümde test edilebilir).
 *
 * Bağlam kuralları:
 * - `a.` ve `a` FROM/JOIN/UPDATE/INTO'dan hemen sonra gelmişse `a` bir şemadır (`schemaMember`);
 *   aksi halde `a` bir ilişki/takma ad (`relationMember`; `s.t.` biçiminde şema da taşır).
 * - FROM/JOIN/UPDATE/INTO sonrası (boşlukla ya da yazılmış kısmi kelimeyle) `relation`.
 * - Yorum / tek tırnaklı dize / dolar-tırnaklı dize içinde `none` (hiç öneri yok).
 * - Diğer her yerde `keyword`.
 * Sözcük analizi arka uçla aynı fikre dayanır: tırnak ve yorumlar metin boyunca izlenir.
 */
import { keywordCompletionSource, PostgreSQL } from "@codemirror/lang-sql";
import type { Completion, CompletionResult, CompletionSource } from "@codemirror/autocomplete";
import type { CompletionItem } from "../../ipc/bindings/CompletionItem";
import type { CompletionKind } from "../../ipc/bindings/CompletionKind";
import type { CompletionRequest } from "../../ipc/bindings/CompletionRequest";

export type CompletionContextKind =
  | { kind: "schemaMember"; schema: string }
  | { kind: "relationMember"; relation: string; schema?: string }
  | { kind: "relation" }
  | { kind: "keyword" }
  | { kind: "none" };

type Tok = { t: "word" | "ident" | "dot" | "punct"; v: string };

const REL_KEYWORDS = new Set(["from", "join", "update", "into", "truncate", "table"]);
/** Virgülle sürdürülen FROM listesini bitiren sözcükler. */
const LIST_STOP = new Set([
  "select", "where", "on", "group", "order", "having", "limit", "offset", "set", "values",
  "union", "intersect", "except", "returning", "using", "fetch", "when", "then", "and", "or",
]);
const WORD_CH = /[\p{L}\p{N}_$]/u;
const DOLLAR_TAG = /^\$(?:[\p{L}_][\p{L}\p{N}_]*)?\$/u;

/** Metnin sonunda yazılmakta olan kısmi ad (açık tırnaklı ad dahil) ve bağlam. */
export function analyzeBefore(text: string): { context: CompletionContextKind; partial: string } {
  const toks: Tok[] = [];
  let partialStart = -1; // sondaki açık tırnaklı kısmi adın başlangıcı
  let i = 0;
  const n = text.length;
  const none = { context: { kind: "none" } as CompletionContextKind, partial: "" };
  while (i < n) {
    const c = text[i]!;
    if (c === "-" && text[i + 1] === "-") {
      const nl = text.indexOf("\n", i);
      if (nl < 0) return none;
      i = nl + 1;
    } else if (c === "/" && text[i + 1] === "*") {
      let depth = 1;
      i += 2;
      while (i < n && depth > 0) {
        if (text[i] === "/" && text[i + 1] === "*") {
          depth++;
          i += 2;
        } else if (text[i] === "*" && text[i + 1] === "/") {
          depth--;
          i += 2;
        } else i++;
      }
      if (depth > 0) return none;
      toks.push({ t: "punct", v: "/**/" });
    } else if (c === "'") {
      i++;
      let closed = false;
      while (i < n) {
        if (text[i] === "'") {
          if (text[i + 1] === "'") {
            i += 2;
            continue;
          }
          i++;
          closed = true;
          break;
        }
        i++;
      }
      if (!closed) return none;
      toks.push({ t: "punct", v: "'" });
    } else if (c === '"') {
      const start = i;
      i++;
      let v = "";
      let closed = false;
      while (i < n) {
        if (text[i] === '"') {
          if (text[i + 1] === '"') {
            v += '"';
            i += 2;
            continue;
          }
          i++;
          closed = true;
          break;
        }
        v += text[i];
        i++;
      }
      if (!closed) {
        partialStart = start;
        break;
      }
      toks.push({ t: "ident", v });
    } else if (c === "$") {
      const m = DOLLAR_TAG.exec(text.slice(i, i + 80));
      if (m) {
        const end = text.indexOf(m[0], i + m[0].length);
        if (end < 0) return none;
        i = end + m[0].length;
        toks.push({ t: "punct", v: "$$" });
      } else {
        let j = i;
        while (j < n && WORD_CH.test(text[j]!)) j++;
        toks.push({ t: "word", v: text.slice(i, j) });
        i = j;
      }
    } else if (WORD_CH.test(c)) {
      let j = i;
      while (j < n && WORD_CH.test(text[j]!)) j++;
      toks.push({ t: "word", v: text.slice(i, j) });
      i = j;
    } else if (c === ".") {
      toks.push({ t: "dot", v: "." });
      i++;
    } else {
      if (!/\s/.test(c)) toks.push({ t: "punct", v: c });
      i++;
    }
  }
  let partial = "";
  if (partialStart >= 0) {
    partial = text.slice(partialStart);
  } else if (toks.length > 0 && toks[toks.length - 1]!.t === "word" && WORD_CH.test(text[n - 1] ?? "")) {
    partial = toks.pop()!.v;
  }
  return { context: classify(toks), partial };
}

const isKw = (t: Tok | undefined): boolean => t?.t === "word" && REL_KEYWORDS.has(t.v.toLowerCase());

/** `toks[idx]` bir ilişki adı konumundan hemen önce mi (FROM/JOIN/... ya da `from a, b,` virgülü)? */
function relPos(toks: Tok[], idx: number): boolean {
  const t = toks[idx];
  if (isKw(t)) return true;
  if (t?.t !== "punct" || t.v !== ",") return false;
  for (let k = idx - 1; k >= 0; k--) {
    const u = toks[k]!;
    if (u.t === "word") {
      const w = u.v.toLowerCase();
      if (w === "from" || w === "join") return true;
      if (LIST_STOP.has(w)) return false;
    } else if (u.t === "punct" && u.v !== ",") return false;
  }
  return false;
}
const isName = (t: Tok | undefined): t is Tok => t !== undefined && (t.t === "word" || t.t === "ident");

function classify(toks: Tok[]): CompletionContextKind {
  const last = toks[toks.length - 1];
  if (last?.t === "dot") {
    const a = toks[toks.length - 2];
    if (!isName(a)) return { kind: "keyword" };
    const before = toks[toks.length - 3];
    if (before?.t === "dot") {
      const s = toks[toks.length - 4];
      if (isName(s)) return { kind: "relationMember", relation: a.v, schema: s.v };
    }
    if (before && relPos(toks, toks.length - 3)) return { kind: "schemaMember", schema: a.v };
    return { kind: "relationMember", relation: a.v };
  }
  if (last && relPos(toks, toks.length - 1)) return { kind: "relation" };
  return { kind: "keyword" };
}

export function contextAt(textBefore: string): CompletionContextKind {
  return analyzeBefore(textBefore).context;
}

/** CodeMirror `CompletionContext`'in kullandığımız alt kümesi (testte sahte verilebilir). */
export interface CompletionCtxLike {
  pos: number;
  explicit: boolean;
  readonly aborted: boolean;
  state: { doc: { sliceString(from: number, to?: number): string } };
  addEventListener(type: "abort", listener: () => void): void;
}

export interface CompletionSourceDeps {
  fetch: (request: CompletionRequest) => Promise<CompletionItem[]>;
  isBusy: () => boolean;
  /** Anahtar kelime kaynağı (varsayılan: PostgreSQL). */
  keywords?: CompletionSource;
  debounceMs?: number;
}

const TYPE_OF: Record<CompletionKind, string> = {
  schema: "namespace",
  table: "class",
  view: "class",
  matView: "class",
  column: "property",
  function: "function",
};

/**
 * Değiştirilecek aralık: yazılmış kısmi ad `"` ile başlıyorsa ve imleçten hemen sonra `closeBrackets`in
 * eklediği kapanış `"` varsa onu da kapsar (yoksa kabul sonrası `""` kalır).
 */
export function replaceRange(pos: number, partial: string, nextChar: string): { from: number; to: number } {
  const to = partial.startsWith('"') && nextChar === '"' ? pos + 1 : pos;
  return { from: pos - partial.length, to };
}

/** Yazılan kısmi adı (başındaki `"` atılır) ham etikete göre süzer: önce önek, sonra içerme. */
export function matchItems(items: CompletionItem[], partial: string): CompletionItem[] {
  const bare = partial.replace(/^"/, "").toLowerCase();
  if (bare === "") return items;
  const prefix: CompletionItem[] = [];
  const rest: CompletionItem[] = [];
  for (const it of items) {
    const l = it.label.toLowerCase();
    if (l.startsWith(bare)) prefix.push(it);
    else if (l.includes(bare)) rest.push(it);
  }
  return [...prefix, ...rest];
}

export function createCompletionSource(deps: CompletionSourceDeps): CompletionSource {
  const keywords = deps.keywords ?? keywordCompletionSource(PostgreSQL, true);
  const debounceMs = deps.debounceMs ?? 150;
  return async (ctx) => {
    const c = ctx as unknown as CompletionCtxLike;
    const text = c.state.doc.sliceString(0, c.pos);
    const { context, partial } = analyzeBefore(text);
    if (context.kind === "none") return null;
    const kwFallback = (): CompletionResult | null | Promise<CompletionResult | null> =>
      context.kind === "schemaMember" || context.kind === "relationMember" ? null : keywords(ctx);

    if (context.kind === "keyword" || deps.isBusy()) return kwFallback();

    // Yazarken debounce: bu sürede iptal geldiyse (yeni tuş vuruşu) hiç istek atma.
    let aborted = c.aborted;
    c.addEventListener("abort", () => {
      aborted = true;
    });
    await new Promise<void>((r) => setTimeout(r, debounceMs));
    if (aborted || c.aborted) return null;
    if (deps.isBusy()) return kwFallback(); // beklerken çalıştırma başladı

    let items: CompletionItem[];
    try {
      items = await requestItems(deps, context, () => aborted || c.aborted);
    } catch {
      items = [];
    }
    if (aborted || c.aborted) return null;
    items = matchItems(items, partial);
    if (items.length === 0) return kwFallback();
    const options: Completion[] = items.map((it) => ({
      label: it.quoted,
      detail: it.detail ?? undefined,
      type: TYPE_OF[it.kind],
    }));
    const next = c.state.doc.sliceString(c.pos, c.pos + 1);
    const { from, to } = replaceRange(c.pos, partial, next);
    // filter:false -> süzme/sıralama yukarıda ham etikete göre yapıldı (`"ord` -> `orders` eşleşir).
    return { from, to, options, filter: false };
  };
}

/**
 * Her istek arka uçta oturum kilidini (en çok 5 sn) tutar: istekler art arda yapılır, aralarda
 * iptal/meşgul denetlenir (paralel istek kilit penceresini ikiye katlar).
 */
async function requestItems(
  deps: CompletionSourceDeps,
  ctx: Exclude<CompletionContextKind, { kind: "keyword" | "none" }>,
  isAborted: () => boolean,
): Promise<CompletionItem[]> {
  const { fetch } = deps;
  switch (ctx.kind) {
    case "schemaMember":
      return fetch({ type: "relations", schema: ctx.schema });
    case "relationMember": {
      const cols = await fetch({ type: "columns", schema: ctx.schema ?? null, relation: ctx.relation });
      if (cols.length > 0 || ctx.schema !== undefined) return cols;
      // `select app.`: `app` aslında şema olabilir.
      if (isAborted() || deps.isBusy()) return [];
      return fetch({ type: "relations", schema: ctx.relation });
    }
    case "relation": {
      const rels = await fetch({ type: "relations", schema: null });
      if (isAborted() || deps.isBusy()) return rels;
      const schemas = await fetch({ type: "schemas", includeSystem: false });
      return [...rels, ...schemas];
    }
  }
}
