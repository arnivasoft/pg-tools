import { history, undo, undoDepth } from "@codemirror/commands";
import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { resyncState, syncDecision } from "./sql-body-sync";

/** `undo` komutunu görünümsüz çalıştırır; sonuç durumu ya da `null`. */
function runUndo(state: EditorState): EditorState | null {
  let out: EditorState | null = null;
  const ok = undo({ state, dispatch: (tr) => (out = tr.state) });
  return ok ? out : null;
}

describe("sql-body-sync: dış değer eşitlemesi", () => {
  const ext = [history()];

  it("aynı metin → null (durum korunur)", () => {
    const s = EditorState.create({ doc: "a\nb", extensions: ext });
    expect(resyncState(s, "a\nb", ext)).toBeNull();
  });

  it("geri alma geçmişi sıfırlanır: yeniden yükleme öncesi düzenleme yeni metne uygulanamaz", () => {
    let s = EditorState.create({ doc: "BEGIN\nEND;", extensions: ext });
    s = s.update({ changes: { from: 5, insert: "\n  x;" } }).state;
    expect(undoDepth(s)).toBe(1);
    const r = resyncState(s, "BEGIN\n  RETURN 1;\nEND;", ext)!.state;
    expect(r.doc.toString()).toBe("BEGIN\n  RETURN 1;\nEND;");
    expect(undoDepth(r)).toBe(0);
    expect(runUndo(r)).toBeNull();
  });

  it("seçim değişiklikle eşlenir (değişiklikten sonraki imleç kayar)", () => {
    const s = EditorState.create({ doc: "abc\ndef", selection: { anchor: 6 }, extensions: ext });
    const r = resyncState(s, "XXabc\ndef", ext)!.state;
    expect(r.selection.main.head).toBe(8);
    const s2 = EditorState.create({ doc: "abcdef", selection: { anchor: 6 }, extensions: ext });
    expect(resyncState(s2, "ab", ext)!.state.selection.main.head).toBe(2);
  });
});

describe("sql-body-sync: syncDecision", () => {
  it("belge değerin görüntüsüyle aynıysa noop (editörün kendi yankısı; bileşim sürse de)", () => {
    expect(syncDecision("a\nb", "a\nb", false)).toBe("noop");
    expect(syncDecision("a\nb", "a\nb", true)).toBe("noop");
    expect(syncDecision("", "", true)).toBe("noop");
  });

  it("gerçek dış değişiklik: bileşim sürüyorsa defer, aksi halde apply", () => {
    expect(syncDecision("abc", "abd", true)).toBe("defer");
    expect(syncDecision("abc", "abd", false)).toBe("apply");
    expect(syncDecision("ab", "abc", false)).toBe("apply");
  });
});
