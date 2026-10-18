import { defaultKeymap, history, historyKeymap, indentWithTab, temporarilySetTabFocusMode } from "@codemirror/commands";
import { PostgreSQL, sql } from "@codemirror/lang-sql";
import { HighlightStyle, bracketMatching, indentOnInput, indentUnit, syntaxHighlighting } from "@codemirror/language";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, drawSelection, keymap, lineNumbers } from "@codemirror/view";
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

/** `extra`: çağıranın ek uzantıları (ör. designer önizlemesinde başarısız ifade vurgusu). */
export function createReadOnlyEditor(parent: HTMLElement, doc: string, extra: Extension[] = []): EditorView {
  return new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        sqlLookExtensions(),
        extra,
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

/**
 * Düzenlenebilir gövde editörünün (designer `SqlBodyEditor`, Plan 23) temel uzantıları: satır numarası, geri alma
 * geçmişi, girinti (`indent` birimi; Tab/Shift+Tab), parantez eşleme; `closeBrackets` yok.
 *
 * Neden burada (paket): `@codemirror/commands` Sorgu Aracı ile ikinci bir tembel tüketiciden (RoutineForm) içe
 * aktarılınca Rolldown onu ayrı bir paylaşılan parçaya böler ve ana paketin `__vite__mapDeps` listesi bir girdi büyür
 * (ana +29 B, bütçe aşılır). Bu ortak modülden içe aktarıldığında commands bu parçayla aynı giriş kümesine düşer ve
 * mevcut CodeMirror parçasına katılır (yeni parça yok). Bu modül ana paket modüllerini içe aktarmamalıdır.
 *
 * Tuzaktan çıkış: Esc → 2 sn tab-odak kipi (`temporarilySetTabFocusMode`); bağ `false` döner ki `defaultKeymap`'teki
 * `simplifySelection` da çalışsın (seçim varken o Esc'i tüketir ve yerleşik Esc işleyicisine sıra gelmez — kip yine de
 * burada açılmış olur). Salt okunurda `indentMore` `false` döner → Tab tarayıcıya kalır, odak doğrudan çıkar.
 */
export function bodyEditingExtensions(indent: string): Extension[] {
  return [
    lineNumbers(),
    history(),
    drawSelection(),
    indentOnInput(),
    bracketMatching(),
    indentUnit.of(indent),
    keymap.of([
      {
        key: "Escape",
        run: (v) => {
          temporarilySetTabFocusMode(v);
          return false;
        },
      },
      ...defaultKeymap,
      ...historyKeymap,
      indentWithTab,
    ]),
  ];
}
