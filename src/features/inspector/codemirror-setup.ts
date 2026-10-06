import { PostgreSQL, sql } from "@codemirror/lang-sql";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { tags } from "@lezer/highlight";

export const sharedHighlight = HighlightStyle.define([
  { tag: tags.keyword, color: "var(--syn-keyword)" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--syn-string)" },
  { tag: tags.number, color: "var(--syn-number)" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: [tags.typeName, tags.standard(tags.name)], color: "var(--syn-type)" },
]);

export const sharedTheme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "var(--bg)", color: "var(--fg)", fontSize: "12.5px" },
  ".cm-scroller": { fontFamily: '"Cascadia Mono", Consolas, monospace', lineHeight: "1.5" },
  ".cm-gutters": { backgroundColor: "var(--panel)", color: "var(--muted)", border: "none" },
  "&.cm-focused": { outline: "none" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--selected)" },
});

/** SQL dili + vurgulama + tema: salt okunur DDL görünümü ile Sorgu Aracı editörünün ortak kaynağı. */
export function sqlLookExtensions(): Extension[] {
  return [sql({ dialect: PostgreSQL }), syntaxHighlighting(sharedHighlight), sharedTheme];
}

export function createReadOnlyEditor(parent: HTMLElement, doc: string): EditorView {
  return new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        sqlLookExtensions(),
      ],
    }),
  });
}

export function replaceDoc(view: EditorView, doc: string): void {
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: doc },
    selection: { anchor: 0 },
    effects: EditorView.scrollIntoView(0, { y: "start" }),
  });
}
