import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { HighlightStyle, foldGutter, syntaxHighlighting, type Language } from "@codemirror/language";
import { EditorView, lineNumbers } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { sharedHighlight, sharedTheme } from "../inspector/codemirror-setup";

export type CodeLang = "json" | "xml" | "html";

export interface LoadedLanguage {
  extension: Extension;
  language: Language;
}

const cache = new Map<CodeLang, Promise<LoadedLanguage>>();

async function load(lang: CodeLang): Promise<LoadedLanguage> {
  // Dil paketleri yalnız burada, dinamik import() ile yüklenir (ayrı parça; ana pakete girmez).
  switch (lang) {
    case "json": {
      const m = await import("@codemirror/lang-json");
      return { extension: m.json(), language: m.jsonLanguage };
    }
    case "xml": {
      const m = await import("@codemirror/lang-xml");
      return { extension: m.xml(), language: m.xmlLanguage };
    }
    case "html": {
      const m = await import("@codemirror/lang-html");
      return { extension: m.html({ autoCloseTags: false }), language: m.htmlLanguage };
    }
  }
}

/** Aynı dil için tek Promise; reddedilirse önbellek temizlenir (yeniden denenebilir). */
export function loadLanguage(lang: CodeLang): Promise<LoadedLanguage> {
  const hit = cache.get(lang);
  if (hit) return hit;
  const p = load(lang);
  cache.set(lang, p);
  p.catch(() => {
    if (cache.get(lang) === p) cache.delete(lang);
  });
  return p;
}

/** Ortak etiketler + JSON/XML/HTML etiketleri; yalnız `--syn-*` ve `--muted` renkleri. */
export const viewerHighlight = HighlightStyle.define([
  ...sharedHighlight.specs,
  { tag: tags.propertyName, color: "var(--syn-type)" },
  { tag: [tags.bool, tags.null], color: "var(--syn-keyword)" },
  { tag: tags.tagName, color: "var(--syn-keyword)" },
  { tag: tags.attributeName, color: "var(--syn-type)" },
  { tag: tags.attributeValue, color: "var(--syn-string)" },
  { tag: tags.angleBracket, color: "var(--muted)" },
  { tag: [tags.processingInstruction, tags.documentMeta], color: "var(--syn-comment)" },
]);

const languageSlot = new WeakMap<EditorView, Compartment>();

/** Salt okunur kod görüntüleyici: satır numarası, katlama, yatay kaydırma (satır sarma yok). */
export function createCodeViewer(parent: HTMLElement, doc: string, language: Extension): EditorView {
  const slot = new Compartment();
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        lineNumbers(),
        foldGutter(),
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        syntaxHighlighting(viewerHighlight),
        sharedTheme,
        slot.of(language),
      ],
    }),
  });
  languageSlot.set(view, slot);
  return view;
}

/** Belgeyi ve dili birlikte değiştirir; kaydırma başa döner. */
export function replaceCode(view: EditorView, doc: string, language: Extension): void {
  const slot = languageSlot.get(view);
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: doc },
    selection: { anchor: 0 },
    effects: [
      ...(slot ? [slot.reconfigure(language)] : []),
      EditorView.scrollIntoView(0, { y: "start" }),
    ],
  });
}
