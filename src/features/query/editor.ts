/**
 * Düzenlenebilir Sorgu Aracı editörü (CodeMirror 6).
 *
 * ÖNEMLİ (ilk açılış maliyeti): CodeMirror ağır bir bağımlılıktır. Bu modül yalnızca tembel yüklenen
 * Sorgu Aracı bileşenlerinden (dynamic import) içe aktarılmalı; App.tsx ya da ana pakete giren herhangi
 * bir modül bunu STATİK olarak içe aktarmamalıdır.
 *
 * `setDoc` programatik bir değişikliktir: `onChange` çağrılmaz (sekme geçişi "kirli" işaretlemesin).
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
  keymap,
  lineNumbers,
  type DecorationSet,
} from "@codemirror/view";
import type { Scope } from "../../ipc/bindings/Scope";
import { t } from "../../app/strings";
import { sqlLookExtensions } from "../inspector/codemirror-setup";
import { scopeFromEditor } from "./offsets";

export type RunKind = "stmt" | "all";
export type ErrorRange = { from: number; to: number };

export interface QueryEditorHandlers {
  onRun(kind: RunKind): void;
  onCancel(): void;
  onChange(doc: string): void;
  complete: CompletionSource;
}

export interface QueryEditor {
  view: EditorView;
  /** UTF-16 aralığı (belge içi); `null` işareti kaldırır. Metin değişince kendiliğinden temizlenir. */
  setErrorMark(range: ErrorRange | null): void;
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

class ErrorGutterMarker extends GutterMarker {
  override toDOM(): Node {
    const el = document.createElement("span");
    el.textContent = "●";
    el.title = t.query.errorMark;
    return el;
  }
}
const errorMarker = new ErrorGutterMarker();

const errorGutter = gutter({
  class: "cm-error-gutter",
  markers: (view) => {
    const r = view.state.field(errorField);
    if (!r) return RangeSet.empty;
    return RangeSet.of([errorMarker.range(view.state.doc.lineAt(r.from).from)]);
  },
});

const queryTheme = EditorView.theme({
  ".cm-error-mark": { textDecoration: "underline wavy var(--danger)", textDecorationSkipInk: "none" },
  ".cm-error-gutter": { width: "14px", color: "var(--danger)" },
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
    autocompletion({ override: [handlers.complete], icons: false }),
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
    queryTheme,
    EditorView.updateListener.of((u) => {
      if (u.docChanged && !u.transactions.some((t) => t.annotation(programmatic))) {
        handlers.onChange(u.state.doc.toString());
      }
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
