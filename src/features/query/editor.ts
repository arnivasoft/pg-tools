/**
 * Düzenlenebilir Sorgu Aracı editörü (CodeMirror 6).
 *
 * ÖNEMLİ (ilk açılış maliyeti): CodeMirror ağır bir bağımlılıktır. Bu modül yalnızca tembel yüklenen
 * Sorgu Aracı bileşenlerinden (dynamic import) içe aktarılmalı; App.tsx ya da ana pakete giren herhangi
 * bir modül bunu STATİK olarak içe aktarmamalıdır.
 *
 * `setDoc` programatik bir değişikliktir: `onChange` çağrılmaz (sekme geçişi "kirli" işaretlemesin).
 *
 * İki işaret katmanı vardır: çalıştırma hatası (`setErrorMark`, Plan 6; metin değişince silinir) ve yazarken denetim
 * işaretleri (`setLintMarks`, tasarım §4; hata kırmızı dalgalı, bilgi gri noktalı, gutter + üzerine gelince ileti;
 * düzenlemede ifadesine dokunulan işaret kalkar, diğerleri eşlenir — `check-model.mapMarks`). İkisi birbirinden
 * bağımsızdır; gutter'da aynı satırda öncelik: hata > bilgi.
 */
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, type CompletionSource } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab, temporarilySetTabFocusMode, toggleComment } from "@codemirror/commands";
import { bracketMatching, indentOnInput } from "@codemirror/language";
import { Annotation, EditorState, Prec, RangeSet, StateEffect, StateField, Transaction } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  GutterMarker,
  drawSelection,
  gutter,
  hoverTooltip,
  keymap,
  lineNumbers,
  type DecorationSet,
} from "@codemirror/view";
import type { Scope } from "../../ipc/bindings/Scope";
import { t } from "../../app/strings";
import { sqlLookExtensions } from "../inspector/codemirror-setup";
import { mapMarks, type CheckView, type LintMark } from "./check-model";
import { formatTransaction } from "./format-edit";
import { scopeFromEditor } from "./offsets";

export type RunKind = "stmt" | "all";
export type ErrorRange = { from: number; to: number };

export interface QueryEditorHandlers {
  onRun(kind: RunKind): void;
  onCancel(): void;
  onChange(doc: string): void;
  /** Ctrl+Shift+F. */
  onFormat?(): void;
  /** Kullanıcı düzenlemesi (`docChanged`, programatik değil), seçim ya da görünür alan değişimi. */
  onUpdate?(u: { edited: boolean; moved: boolean }): void;
  complete: CompletionSource;
}

export interface QueryEditor {
  view: EditorView;
  /** UTF-16 aralığı (belge içi); `null` işareti kaldırır. Metin değişince kendiliğinden temizlenir. */
  setErrorMark(range: ErrorRange | null): void;
  /** Denetim işaretlerini değiştirir (UTF-16, şimdiki metne göre); boş dizi temizler. */
  setLintMarks(marks: LintMark[]): void;
  /** Denetim için görünüm: metin, imleç ve görünür alan (UTF-16). */
  checkView(): CheckView;
  /**
   * `[from, to)` aralığını `insert` ile tek geri alma adımında değiştirir; imleç biçimlendirme eşlemesiyle taşınır.
   * Belge `expectDoc`'tan farklıysa (bu arada yazıldı) ya da salt okunursa hiçbir şey yapmaz ve `false` döner.
   */
  replaceFormatted(expectDoc: string, from: number, to: number, insert: string): boolean;
  /** Çalıştırma anındaki metin ve kapsam (ofsetler Unicode scalar). */
  getSnapshot(kind: RunKind): { text: string; scope: Scope };
  /** İlk/yedek içerik: geri alma geçmişine girmez. Sekme geçişi için `getState`/`setState` kullanın. */
  setDoc(text: string): void;
  /** Aynı uzantı dizisiyle yeni bir sekme durumu (sekme başına bir durum; ham `EditorState.create` kullanmayın). */
  createState(doc: string, opts?: { readOnly?: boolean }): EditorState;
  /** Sekmeye özgü tüm durum (metin, geri alma geçmişi, seçim). Kaydırma konumu DOM'da olduğundan dışarıda tutulur. */
  getState(): EditorState;
  /** Başka sekmenin durumunu yükler; `onChange` çağrılmaz. */
  setState(state: EditorState): void;
  focus(): void;
  destroy(): void;
}

const setError = StateEffect.define<ErrorRange | null>();
const programmatic = Annotation.define<boolean>();

const errorField = StateField.define<ErrorRange | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setError)) return e.value;
    return tr.docChanged ? null : value;
  },
  provide: (f) =>
    EditorView.decorations.compute([f], (state): DecorationSet => {
      const r = state.field(f);
      return r ? Decoration.set([Decoration.mark({ class: "cm-error-mark" }).range(r.from, r.to)]) : Decoration.none;
    }),
});

const setLint = StateEffect.define<LintMark[]>();
const lintErrorDeco = Decoration.mark({ class: "cm-lint-error" });
const lintInfoDeco = Decoration.mark({ class: "cm-lint-info" });

const lintField = StateField.define<LintMark[]>({
  create: () => [],
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setLint)) return e.value;
    return tr.docChanged && value.length > 0 ? mapMarks(value, tr.changes) : value;
  },
  provide: (f) =>
    EditorView.decorations.compute([f], (state): DecorationSet => {
      const len = state.doc.length;
      const ranges = [];
      for (const m of state.field(f)) {
        const r = normalizeRange(m, len);
        if (r) ranges.push((m.severity === "error" ? lintErrorDeco : lintInfoDeco).range(r.from, r.to));
      }
      return Decoration.set(ranges, true);
    }),
});

function lintText(m: LintMark): string {
  return m.sqlstate ? `${m.message} (${m.sqlstate})` : m.message;
}

class ErrorGutterMarker extends GutterMarker {
  constructor(
    readonly kind: "error" | "info",
    readonly label: string,
  ) {
    super();
  }
  override eq(other: GutterMarker): boolean {
    return other instanceof ErrorGutterMarker && other.kind === this.kind && other.label === this.label;
  }
  override toDOM(): Node {
    const el = document.createElement("span");
    el.textContent = "●";
    el.title = this.label;
    if (this.kind === "info") el.className = "cm-lint-info-marker";
    return el;
  }
}
const errorMarker = new ErrorGutterMarker("error", t.query.errorMark);

const errorGutter = gutter({
  class: "cm-error-gutter",
  markers: (view) => {
    const state = view.state;
    const len = state.doc.length;
    // Satır başı → işaret (hata > bilgi; aynı düzeyde ilk işaretin iletisi).
    const lines = new Map<number, ErrorGutterMarker>();
    const r = state.field(errorField);
    if (r) lines.set(state.doc.lineAt(Math.min(r.from, len)).from, errorMarker);
    for (const m of state.field(lintField)) {
      const at = state.doc.lineAt(Math.min(m.from, len)).from;
      const cur = lines.get(at);
      if (cur && (cur.kind === "error" || m.severity === "info")) continue;
      lines.set(at, new ErrorGutterMarker(m.severity, lintText(m)));
    }
    if (lines.size === 0) return RangeSet.empty;
    return RangeSet.of([...lines].sort((a, b) => a[0] - b[0]).map(([at, mk]) => mk.range(at)));
  },
});

/** Üzerine gelince denetim iletileri (o noktadaki tüm işaretler). */
const lintHover = hoverTooltip((view, pos) => {
  const hits = view.state.field(lintField).filter((m) => m.from <= pos && pos <= Math.max(m.to, m.from + 1));
  if (hits.length === 0) return null;
  return {
    pos: Math.min(...hits.map((m) => m.from)),
    end: Math.max(...hits.map((m) => m.to)),
    above: true,
    create: () => {
      const dom = document.createElement("div");
      dom.className = "cm-lint-tooltip";
      for (const m of hits) {
        const row = document.createElement("div");
        const label = document.createElement("span");
        label.className = m.severity === "error" ? "cm-lint-label-error" : "cm-lint-label-info";
        label.textContent = m.severity === "error" ? t.query.check.error : t.query.check.info;
        row.append(label, ` ${lintText(m)}`);
        if (m.severity === "info") {
          const hint = document.createElement("div");
          hint.className = "cm-lint-hint";
          hint.textContent = t.query.check.infoHint;
          row.append(hint);
        }
        dom.appendChild(row);
      }
      return { dom };
    },
  };
});

const queryTheme = EditorView.theme({
  // İmleç: `drawSelection` kendi imlecini çizer; temel temanın rengi siyahtır ve yalnızca `EditorView.darkTheme`
  // açıkken açık renge döner. Uygulama temayı CSS değişkenleriyle değiştirdiği için koyu temada imleç arka planla
  // aynı renkte kalıyordu: rengi ön plan değişkeninden alınır ve kalınlaştırılır.
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--fg)", borderLeftWidth: "2px", marginLeft: "-1px" },
  // Yerel imlece düşülen durumlar için (ör. widget içinde odak).
  ".cm-content": { caretColor: "var(--fg)" },
  ".cm-error-mark": { textDecoration: "underline wavy var(--danger)", textDecorationSkipInk: "none" },
  ".cm-error-gutter": { width: "14px", color: "var(--danger)" },
  ".cm-lint-error": { textDecoration: "underline wavy var(--danger)", textDecorationSkipInk: "none" },
  ".cm-lint-info": { textDecoration: "underline dotted var(--muted)", textDecorationSkipInk: "none", textUnderlineOffset: "3px" },
  ".cm-lint-info-marker": { color: "var(--muted)" },
  ".cm-lint-tooltip": { padding: "3px 6px", maxWidth: "560px", fontSize: "12px", whiteSpace: "pre-wrap" },
  ".cm-lint-label-error": { color: "var(--danger)", fontWeight: "600" },
  ".cm-lint-label-info": { color: "var(--muted)", fontWeight: "600" },
  ".cm-lint-hint": { color: "var(--muted)" },
  ".cm-tooltip": {
    backgroundColor: "var(--panel)",
    color: "var(--fg)",
    border: "1px solid var(--border, var(--muted))",
  },
  ".cm-tooltip-autocomplete ul li[aria-selected]": { backgroundColor: "var(--selected)", color: "var(--fg)" },
});

/** Boş aralık işaretlenemez: genişlet; sınır dışıysa kırp. */
function normalizeRange(r: ErrorRange, len: number): ErrorRange | null {
  let from = Math.max(0, Math.min(r.from, len));
  let to = Math.max(from, Math.min(r.to, len));
  if (from === to) {
    if (to < len) to += 1;
    else if (from > 0) from -= 1;
    else return null;
  }
  return { from, to };
}

export function createQueryEditor(parent: HTMLElement, doc: string, handlers: QueryEditorHandlers): QueryEditor {
  const run = (kind: RunKind) => () => {
    handlers.onRun(kind);
    return true;
  };
  const extensions = [
    Prec.highest(
      keymap.of([
        { key: "Mod-Enter", run: run("stmt"), preventDefault: true },
        { key: "F5", run: run("all"), preventDefault: true }, // WebView yeniden yüklemesin
        {
          key: "Ctrl-.",
          run: () => {
            handlers.onCancel();
            return true;
          },
          preventDefault: true,
        },
        { key: "Mod-/", run: toggleComment, preventDefault: true },
        {
          key: "Mod-Shift-f",
          run: () => {
            handlers.onFormat?.();
            return true;
          },
          preventDefault: true,
        },
      ]),
    ),
    lineNumbers(),
    errorGutter,
    history(),
    drawSelection(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    bracketMatching(),
    closeBrackets(),
    autocompletion({ override: [handlers.complete], icons: false, activateOnTypingDelay: 50 }),
    keymap.of([
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...historyKeymap,
      ...completionKeymap,
      indentWithTab,
      // Tab tuzağından çıkış: Escape sonra Tab (2 sn) ya da Ctrl+M.
      { key: "Escape", run: temporarilySetTabFocusMode },
    ]),
    sqlLookExtensions(),
    errorField,
    lintField,
    lintHover,
    queryTheme,
    EditorView.updateListener.of((u) => {
      const edited = u.docChanged && !u.transactions.some((t) => t.annotation(programmatic));
      if (edited) handlers.onChange(u.state.doc.toString());
      const moved = u.selectionSet || u.viewportChanged || u.geometryChanged;
      if (edited || moved) handlers.onUpdate?.({ edited, moved });
    }),
  ];
  const createState = (text: string, opts: { readOnly?: boolean } = {}) =>
    EditorState.create({ doc: text, extensions: opts.readOnly ? [extensions, EditorState.readOnly.of(true)] : extensions });
  const view = new EditorView({ parent, state: createState(doc) });

  return {
    view,
    createState,
    setErrorMark(range) {
      const r = range ? normalizeRange(range, view.state.doc.length) : null;
      view.dispatch({ effects: setError.of(r) });
    },
    setLintMarks(marks) {
      view.dispatch({ effects: setLint.of(marks) });
    },
    checkView() {
      const ranges = view.visibleRanges;
      const head = view.state.selection.main.head;
      return {
        text: view.state.doc.toString(),
        cursor: head,
        visibleFrom: ranges.length > 0 ? ranges[0]!.from : head,
        visibleTo: ranges.length > 0 ? ranges[ranges.length - 1]!.to : head,
      };
    },
    replaceFormatted(expectDoc, from, to, insert) {
      const spec = formatTransaction(view.state, expectDoc, from, to, insert);
      if (spec) view.dispatch(spec);
      return spec !== null;
    },
    getSnapshot(kind) {
      const text = view.state.doc.toString();
      const sel = view.state.selection.main;
      return { text, scope: scopeFromEditor(text, sel.from, sel.to, sel.head, kind === "stmt" ? "run" : "all") };
    },
    setDoc(text) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: text },
        selection: { anchor: 0 },
        annotations: [programmatic.of(true), Transaction.addToHistory.of(false)],
      });
    },
    getState: () => view.state,
    setState: (state) => view.setState(state),
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}
