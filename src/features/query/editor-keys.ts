// Sorgu editörünün uygulama kısayolları (saf; CodeMirror'a bağımlı değil, test edilebilir). `editor.ts` bunları en
// yüksek öncelikli keymap'e koyar. Tuş adları CodeMirror sözdizimindedir (`Mod` = Windows'ta Ctrl).
import type { ExplainKind } from "./explain-model";

export type RunKind = "stmt" | "all";

export interface EditorKeyHandlers {
  onRun(kind: RunKind): void;
  onCancel(): void;
  onFormat?(): void;
  /** F7: plan (EXPLAIN); Shift+F7: ANALYZE ile plan. */
  onExplain?(kind: Exclude<ExplainKind, "menu">): void;
}

export type EditorKey = { key: string; run: () => boolean; preventDefault: true };

export function editorKeys(h: EditorKeyHandlers): EditorKey[] {
  const k = (key: string, fn: () => void): EditorKey => ({
    key,
    run: () => {
      fn();
      return true;
    },
    preventDefault: true,
  });
  return [
    k("Mod-Enter", () => h.onRun("stmt")),
    k("F5", () => h.onRun("all")), // WebView yeniden yüklemesin
    k("Ctrl-.", () => h.onCancel()),
    k("Mod-Shift-f", () => h.onFormat?.()),
    // WebView2'de F7 tarayıcının "imleçle gezinme" istemini açar: tuş her durumda yutulur.
    k("F7", () => h.onExplain?.("plain")),
    k("Shift-F7", () => h.onExplain?.("analyze")),
  ];
}
