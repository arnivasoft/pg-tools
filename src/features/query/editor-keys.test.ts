import { describe, expect, it, vi } from "vitest";
import { editorKeys } from "./editor-keys";

function press(key: string) {
  const h = { onRun: vi.fn(), onCancel: vi.fn(), onFormat: vi.fn(), onExplain: vi.fn() };
  const b = editorKeys(h).find((x) => x.key === key);
  expect(b, key).toBeDefined();
  expect(b!.run()).toBe(true);
  expect(b!.preventDefault).toBe(true);
  return h;
}

describe("editör kısayolları", () => {
  it("F7 planı (ANALYZE'sız), Shift+F7 ANALYZE ile planı ister", () => {
    expect(press("F7").onExplain).toHaveBeenCalledWith("plain");
    expect(press("Shift-F7").onExplain).toHaveBeenCalledWith("analyze");
    expect(press("F7").onRun).not.toHaveBeenCalled();
  });
  it("mevcut kısayollar korunur", () => {
    expect(press("Mod-Enter").onRun).toHaveBeenCalledWith("stmt");
    expect(press("F5").onRun).toHaveBeenCalledWith("all");
    expect(press("Ctrl-.").onCancel).toHaveBeenCalled();
    expect(press("Mod-Shift-f").onFormat).toHaveBeenCalled();
  });
  it("işleyici yoksa F7 yine yutulur (WebView istemi açılmaz)", () => {
    const b = editorKeys({ onRun: vi.fn(), onCancel: vi.fn() }).find((x) => x.key === "F7")!;
    expect(b.run()).toBe(true);
  });
  it("tuşlar benzersiz", () => {
    const keys = editorKeys({ onRun: vi.fn(), onCancel: vi.fn() }).map((x) => x.key);
    expect(new Set(keys).size).toBe(keys.length);
  });
});
