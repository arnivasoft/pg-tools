/**
 * Katalog tamamlama: saf bağlam çözümleme (`contextAt`) + enjekte edilen `fetch`/`isBusy` ile
 * CodeMirror `CompletionSource` fabrikası (düğümde test edilebilir).
 *
 * Bağlam kuralları:
 * - `a.` ve `a` FROM/JOIN/UPDATE/INTO'dan hemen sonra gelmişse `a` bir şemadır (`schemaMember`);
 *   aksi halde `a` bir ilişki/takma ad (`relationMember`; `s.t.` biçiminde şema da taşır).
 *   `relationMember` önce arka uçtan kapsamlı (`scoped`: alias, CTE, alt sorgu) kolonları ister; boşsa Plan 5 yolu.
 * - FROM/JOIN/UPDATE/INTO sonrası (boşlukla ya da yazılmış kısmi kelimeyle) `relation`.
 * - `select ` listesinde (`select`, `distinct`, `all` ya da listedeki `,` sonrası) `selectList`: kapsamdaki
 *   ilişkilerin `alias.kolon` önerileri + anahtar kelimeler.
 * - Yorum / tek tırnaklı dize / dolar-tırnaklı dize içinde `none` (hiç öneri yok).
 * - Diğer her yerde `keyword`.
 * Sözcük analizi arka uçla aynı çözümleyiciyi kullanır (`sql-lex.ts`, `lex.rs`'nin karşılığı).
 */
import { keywordCompletionSource, PostgreSQL } from "@codemirror/lang-sql";
import type { Completion, CompletionResult, CompletionSource } from "@codemirror/autocomplete";
import type { CompletionItem } from "../../ipc/bindings/CompletionItem";
import type { CompletionKind } from "../../ipc/bindings/CompletionKind";
import type { CompletionRequest } from "../../ipc/bindings/CompletionRequest";
import { utf16ToScalar } from "./offsets";
import { lexSql, separatorTokens } from "./sql-lex";

export type CompletionContextKind =
  | { kind: "schemaMember"; schema: string }
  | { kind: "relationMember"; relation: string; schema?: string; quoted?: true }
  | { kind: "relation" }
  | { kind: "selectList" }
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

/**
 * Metnin sonunda yazılmakta olan kısmi ad (açık tırnaklı ad dahil) ve bağlam. Dize/yorum/dolar gövdesi sınırları
 * arka uçla aynı sözcük çözümleyiciden (`lexSql`) gelir (`E'…'` kaçış dizeleri dahil); sözcükler bağlam için
 * `WORD_CH` dizilerine bölünür.
 */
export function analyzeBefore(text: string): { context: CompletionContextKind; partial: string } {
  const toks: Tok[] = [];
  let partialStart = -1; // sondaki açık tırnaklı kısmi adın başlangıcı
  const n = text.length;
  const none = { context: { kind: "none" } as CompletionContextKind, partial: "" };
  /** `[from, to)` aralığını `WORD_CH` dizileri (sözcük) ve tek karakterlere (boşluk dışı: noktalama) böler. */
  const pushWords = (from: number, to: number): void => {
    let i = from;
    while (i < to) {
      const c = text[i]!;
      if (WORD_CH.test(c)) {
        let j = i;
        while (j < to && WORD_CH.test(text[j]!)) j++;
        toks.push({ t: "word", v: text.slice(i, j) });
        i = j;
      } else {
        if (c === ".") toks.push({ t: "dot", v: "." });
        else if (!/\s/.test(c)) toks.push({ t: "punct", v: c });
        i++;
      }
    }
  };
  const lexed = lexSql(text);
  for (let k = 0; k < lexed.length; k++) {
    const t = lexed[k]!;
    switch (t.kind) {
      case "comment":
        if (!t.closed) return none;
        if (text[t.start + 1] === "*") toks.push({ t: "punct", v: "/**/" });
        break;
      case "str":
        if (!t.closed) return none;
        toks.push({ t: "punct", v: "'" });
        break;
      case "dollar":
        if (!t.closed) return none;
        toks.push({ t: "punct", v: "$$" });
        break;
      case "ident":
        if (!t.closed) {
          partialStart = t.start;
          break;
        }
        toks.push({ t: "ident", v: text.slice(t.start + 1, t.end - 1).replace(/""/g, '"') });
        break;
      case "word":
        pushWords(t.start, t.end);
        break;
      case "punct": {
        // Etiket açmayan `$` ardındaki bitişik sözcükle tek sözcüktür (`$1`).
        const next = lexed[k + 1];
        if (text[t.start] === "$" && next?.kind === "word" && next.start === t.end) {
          pushWords(t.start, next.end);
          k++;
        } else pushWords(t.start, t.end);
        break;
      }
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

/** Select listesini bitiren (ya da select listesi dışında olduğunu gösteren) sözcükler. */
const SELECT_STOP = new Set([
  "from", "where", "on", "using", "join", "group", "order", "by", "having", "limit", "offset", "set", "values",
  "returning", "into", "union", "intersect", "except", "window", "fetch", "for", "update", "insert", "delete",
  "table", "with", "when", "then", "else",
]);

const isWord = (t: Tok | undefined, w: string): boolean => t?.t === "word" && t.v.toLowerCase() === w;

/** İmleç (son belirteçten sonra) bir select listesinde yeni bir öğenin başında mı? */
function inSelectList(toks: Tok[]): boolean {
  const last = toks[toks.length - 1];
  if (last?.t === "word") {
    if (isWord(last, "select")) return true;
    return (isWord(last, "distinct") || isWord(last, "all")) && isWord(toks[toks.length - 2], "select");
  }
  if (last?.t !== "punct" || last.v !== ",") return false;
  let depth = 0;
  for (let k = toks.length - 2; k >= 0; k--) {
    const u = toks[k]!;
    if (u.t === "punct") {
      if (u.v === ")") depth++;
      else if (u.v === "(") {
        if (depth === 0) return false; // fonksiyon argümanı ya da parantez içi liste
        depth--;
      } else if (u.v === ";") return false;
    } else if (u.t === "word" && depth === 0) {
      const w = u.v.toLowerCase();
      if (w === "select") return true;
      if (SELECT_STOP.has(w)) return false;
    }
  }
  return false;
}

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
    return a.t === "ident"
      ? { kind: "relationMember", relation: a.v, quoted: true }
      : { kind: "relationMember", relation: a.v };
  }
  if (last && relPos(toks, toks.length - 1)) return { kind: "relation" };
  if (inSelectList(toks)) return { kind: "selectList" };
  return { kind: "keyword" };
}

export function contextAt(textBefore: string): CompletionContextKind {
  return analyzeBefore(textBefore).context;
}

/**
 * İmlecin (`pos`, UTF-16) ifadesi: ayırıcı `;`'ler arası `[from, to)`. Ayırıcı kuralı arka uçla aynıdır
 * (`separatorTokens`: derinlik 0, `BEGIN ATOMIC` gövdesi dışı; yorum, dize — `E'…'` kaçışları dahil —, tırnaklı ad ve
 * dolar-tırnak içindeki `;` sayılmaz, kapanmamışsa metin sonuna kadar sürer). Arka uç yine de yeniden böler.
 */
export function statementBounds(text: string, pos: number): { from: number; to: number } {
  let from = 0;
  for (const t of separatorTokens(text)) {
    if (t.start < pos) from = t.end;
    else return { from, to: t.start };
  }
  return { from, to: text.length };
}

/** `scoped` isteğinin ifade metni ve imlecin bu metindeki Unicode scalar ofseti. */
export function scopedTarget(text: string, pos: number): { sql: string; offset: number } {
  const { from, to } = statementBounds(text, pos);
  const sql = text.slice(from, to);
  return { sql, offset: utf16ToScalar(sql, pos - from) };
}

/** Niteleyiciyi yazıldığı biçime çevirir (tırnaklı ad yeniden tırnaklanır). */
function rawQualifier(ctx: { relation: string; quoted?: true }): string {
  return ctx.quoted ? `"${ctx.relation.replace(/"/g, '""')}"` : ctx.relation;
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

/** İstekten önceki bekleme: `x.`/`şema.` hemen sonrası 0, kısmi ad ve select listesi için `debounceMs`. */
export function completionWaitMs(context: CompletionContextKind, partial: string, debounceMs: number): number {
  return partial === "" && (context.kind === "relationMember" || context.kind === "schemaMember") ? 0 : debounceMs;
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
    const scoped = () => scopedTarget(c.state.doc.sliceString(0), c.pos);

    if (context.kind === "keyword" || deps.isBusy()) return kwFallback();

    // Yazarken debounce: bu sürede iptal geldiyse (yeni tuş vuruşu) hiç istek atma. `.`'nın hemen ardında (üye
    // bağlamı, kısmi ad boş) bekleme yok: nokta başına tek istek; CodeMirror kaynak başına aynı anda tek sorgu
    // çalıştırdığından istekler gidiş-dönüşle sınırlıdır (bkz. `completionWaitMs`).
    let aborted = c.aborted;
    c.addEventListener("abort", () => {
      aborted = true;
    });
    const wait = completionWaitMs(context, partial, debounceMs);
    if (wait > 0) {
      await new Promise<void>((r) => setTimeout(r, wait));
      if (aborted || c.aborted) return null;
      if (deps.isBusy()) return kwFallback(); // beklerken çalıştırma başladı
    }

    let items: CompletionItem[];
    try {
      items = await requestItems(deps, context, () => aborted || c.aborted, scoped);
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
    if (context.kind === "selectList") {
      // Kolon önerileri önce, ardından yazılan önekle başlayan anahtar kelimeler.
      const kw = await keywords(ctx);
      if (aborted || c.aborted) return null;
      const bare = partial.replace(/^"/, "").toLowerCase();
      const kwOptions = (kw?.options ?? []).filter((o) => o.label.toLowerCase().startsWith(bare));
      return { from, to, options: [...options, ...kwOptions], filter: false };
    }
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
  scoped: () => { sql: string; offset: number },
): Promise<CompletionItem[]> {
  const { fetch } = deps;
  switch (ctx.kind) {
    case "schemaMember":
      return fetch({ type: "relations", schema: ctx.schema });
    case "selectList":
      return fetch({ type: "scoped", ...scoped(), prefix: "" });
    case "relationMember": {
      if (ctx.schema === undefined) {
        // Önce kapsam (alias, CTE, alt sorgu); boşsa Plan 5 yolu.
        const found = await fetch({ type: "scoped", ...scoped(), prefix: rawQualifier(ctx) });
        if (found.length > 0) return found;
        if (isAborted() || deps.isBusy()) return [];
      }
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
