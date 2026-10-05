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

const REL_KEYWORDS = new Set(["from", "join", "update", "into"]);
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
    if (isKw(before)) return { kind: "schemaMember", schema: a.v };
    return { kind: "relationMember", relation: a.v };
  }
  if (isKw(last)) return { kind: "relation" };
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

export function createCompletionSource(deps: CompletionSourceDeps): CompletionSource {
  const keywords = deps.keywords ?? keywordCompletionSource(PostgreSQL, true);
  const debounceMs = deps.debounceMs ?? 150;
  return async (ctx) => {
    const c = ctx as unknown as CompletionCtxLike;
    const text = c.state.doc.sliceString(0, c.pos);
    const { context, partial } = analyzeBefore(text);
    if (context.kind === "none") return null;
    const from = c.pos - partial.length;
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
      items = await requestItems(deps.fetch, context);
    } catch {
      items = [];
    }
    if (aborted || c.aborted) return null;
    if (items.length === 0) return kwFallback();
    const options: Completion[] = items.map((it) => ({
      label: it.quoted,
      detail: it.detail ?? undefined,
      type: TYPE_OF[it.kind],
    }));
    return { from, options };
  };
}

async function requestItems(
  fetch: CompletionSourceDeps["fetch"],
  ctx: Exclude<CompletionContextKind, { kind: "keyword" | "none" }>,
): Promise<CompletionItem[]> {
  switch (ctx.kind) {
    case "schemaMember":
      return fetch({ type: "relations", schema: ctx.schema });
    case "relationMember":
      return fetch({ type: "columns", schema: ctx.schema ?? null, relation: ctx.relation });
    case "relation": {
      const [rels, schemas] = await Promise.all([
        fetch({ type: "relations", schema: null }),
        fetch({ type: "schemas", includeSystem: false }),
      ]);
      return [...rels, ...schemas];
    }
  }
}
