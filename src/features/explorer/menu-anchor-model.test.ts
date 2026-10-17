import { describe, expect, it } from "vitest";
import { anchorFromEvent, createMenuOpener, isKeyboardContextMenu, isMenuKey, sameAnchor, separatorBefore, type MenuAnchor } from "./menu-anchor-model";

const rect = (left: number, top: number, bottom: number) => ({ left, top, bottom });
const rects: Record<string, ReturnType<typeof rect>> = { a: rect(10, 100, 122), b: rect(10, 122, 144) };
const rowRect = (id: string) => rects[id] ?? null;
const mouse = { clientX: 300, clientY: 400, button: 2 };

describe("anchorFromEvent", () => {
  it("fare satırda: satır düğümü, işaretçi koordinatı", () => {
    expect(anchorFromEvent(mouse, "b", "a", rowRect, false)).toEqual({ x: 300, y: 400, nodeId: "b", source: "pointer" });
  });

  it("fare boş alanda + seçili var: seçili düğüm, işaretçi koordinatı", () => {
    expect(anchorFromEvent(mouse, null, "a", rowRect, false)).toEqual({ x: 300, y: 400, nodeId: "a", source: "pointer" });
  });

  it("satır da seçili de yoksa null (menü açılmaz)", () => {
    expect(anchorFromEvent(mouse, null, null, rowRect, false)).toBeNull();
    expect(anchorFromEvent(mouse, null, null, rowRect, true)).toBeNull();
  });

  it("klavye: seçili satırın sol-alt köşesi (işaretçi koordinatı yok sayılır)", () => {
    expect(anchorFromEvent({ clientX: 0, clientY: 0 }, null, "a", rowRect, true)).toEqual({ x: 10, y: 122, nodeId: "a", source: "keyboard" });
  });

  it("klavye: satır dikdörtgeni yok (sanal kaydırma dışında) → ağaç kapsayıcısının sol üstü", () => {
    const container = () => rect(5, 50, 900);
    expect(anchorFromEvent({ clientX: 0, clientY: 0 }, null, "zz", rowRect, true, container)).toEqual({ x: 5, y: 50, nodeId: "zz", source: "keyboard" });
  });

  it("klavye: satır görünür alan dışında (overscan) → kapsayıcının sol üstü; içindeyse satır", () => {
    const container = () => rect(5, 50, 900);
    expect(anchorFromEvent({ clientX: 0, clientY: 0 }, null, "o", (id) => (id === "o" ? rect(10, 920, 942) : null), true, container)).toEqual({ x: 5, y: 50, nodeId: "o", source: "keyboard" });
    expect(anchorFromEvent({ clientX: 0, clientY: 0 }, null, "o", (id) => (id === "o" ? rect(10, 10, 32) : null), true, container)).toEqual({ x: 5, y: 50, nodeId: "o", source: "keyboard" });
    expect(anchorFromEvent({ clientX: 0, clientY: 0 }, null, "a", rowRect, true, container)).toEqual({ x: 10, y: 122, nodeId: "a", source: "keyboard" });
  });

  it("klavye: satır da kapsayıcı da yoksa null", () => {
    expect(anchorFromEvent({ clientX: 0, clientY: 0 }, null, "zz", rowRect, true)).toBeNull();
    expect(anchorFromEvent({ clientX: 0, clientY: 0 }, null, "zz", rowRect, true, () => null)).toBeNull();
  });
});

describe("isKeyboardContextMenu", () => {
  it("işaretçi türü boş ya da pointerId -1 → klavye", () => {
    expect(isKeyboardContextMenu({ button: 0, pointerType: "", pointerId: -1 })).toBe(true);
    expect(isKeyboardContextMenu({ button: 2, pointerType: "" })).toBe(true);
    expect(isKeyboardContextMenu({ button: 0, pointerId: -1 })).toBe(true);
  });

  it("fare sağ tıkı → klavye değil", () => {
    expect(isKeyboardContextMenu({ button: 2, pointerType: "mouse", pointerId: 1 })).toBe(false);
    expect(isKeyboardContextMenu({ button: 2 })).toBe(false);
  });

  it("işaretçi bilgisi yoksa: sağ düğme dışı → klavye", () => {
    expect(isKeyboardContextMenu({ button: 0 })).toBe(true);
  });
});

describe("isMenuKey", () => {
  const k = (key: string, extra: Partial<Parameters<typeof isMenuKey>[0]> = {}) => ({
    key,
    shiftKey: false,
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    isComposing: false,
    keyCode: 0,
    ...extra,
  });

  it("ContextMenu tuşu ve Shift+F10", () => {
    expect(isMenuKey(k("ContextMenu"))).toBe(true);
    expect(isMenuKey(k("F10", { shiftKey: true }))).toBe(true);
  });

  it("yalın F10, Ctrl/Alt+Shift+F10 ve diğer tuşlar değil", () => {
    expect(isMenuKey(k("F10"))).toBe(false);
    expect(isMenuKey(k("F10", { shiftKey: true, ctrlKey: true }))).toBe(false);
    expect(isMenuKey(k("F10", { shiftKey: true, altKey: true }))).toBe(false);
    expect(isMenuKey(k("Enter"))).toBe(false);
  });

  it("IME bileşimi sırasında yok sayılır", () => {
    expect(isMenuKey(k("ContextMenu", { isComposing: true }))).toBe(false);
    expect(isMenuKey(k("ContextMenu", { keyCode: 229 }))).toBe(false);
  });
});

describe("sameAnchor", () => {
  const a: MenuAnchor = { x: 1, y: 2, nodeId: "a", source: "keyboard" };
  it("aynı konum ve düğüm → eşit; farklıysa değil; null güvenli", () => {
    expect(sameAnchor(a, { ...a })).toBe(true);
    expect(sameAnchor(a, { ...a, x: 3 })).toBe(false);
    expect(sameAnchor(a, { ...a, nodeId: "b" })).toBe(false);
    expect(sameAnchor(null, a)).toBe(false);
  });
});

describe("createMenuOpener", () => {
  const anchor = (nodeId: string, x = 0): MenuAnchor => ({ x, y: 0, nodeId, source: "pointer" });
  const setup = (results: boolean[]) => {
    let current: MenuAnchor | null = null;
    const resolvers: ((ok: boolean) => void)[] = [];
    let calls = 0;
    const ensure = () => {
      calls++;
      return new Promise<boolean>((r) => resolvers.push(r));
    };
    const open = createMenuOpener(ensure, () => current, (a) => (current = a));
    const settle = async () => {
      for (const r of resolvers.splice(0)) r(results.shift() ?? true);
      await Promise.resolve();
      await Promise.resolve();
    };
    return { open, settle, get: () => current, calls: () => calls };
  };

  it("parça yüklenirken çapa hemen kaydedilir; yükleme bitince aynı çapa kalır (olay kaybolmaz)", async () => {
    const s = setup([true]);
    const a = anchor("a");
    s.open(a);
    expect(s.get()).toBe(a);
    await s.settle();
    expect(s.get()).toBe(a);
  });

  it("yükleme sırasında ikinci sağ tık: son çapa kazanır", async () => {
    const s = setup([true, true]);
    const b = anchor("b");
    s.open(anchor("a"));
    s.open(b);
    await s.settle();
    expect(s.get()).toBe(b);
  });

  it("yükleme reddedilirse çapa temizlenir (menü açılmaz); sonraki istek yeniden yükler", async () => {
    const s = setup([false, true]);
    s.open(anchor("a"));
    await s.settle();
    expect(s.get()).toBeNull();
    const c = anchor("c");
    s.open(c);
    await s.settle();
    expect(s.get()).toBe(c);
    expect(s.calls()).toBe(2);
  });

  it("ret, sonradan açılan farklı çapayı silmez", async () => {
    const s = setup([false, true]);
    s.open(anchor("a"));
    const b = anchor("b");
    // a'nın reddi b kaydedildikten sonra gelir.
    s.open(b);
    await s.settle();
    expect(s.get()).toBe(b);
  });

  it("aynı çapa (keydown ardından gelen klavye contextmenu) yeniden açmaz", async () => {
    const s = setup([true]);
    const a = anchor("a", 5);
    s.open(a);
    s.open({ ...a });
    expect(s.get()).toBe(a);
    expect(s.calls()).toBe(1);
  });
});

describe("separatorBefore", () => {
  const group = (a: string) => (a.startsWith("g1") ? 1 : 2);
  it("ilk öğeden önce ayırıcı yok; grup değişince var, aynı grupta yok", () => {
    const list = ["g1a", "g1b", "g2a", "g2b"];
    expect(list.map((_, i) => separatorBefore(list, i, group))).toEqual([false, false, true, false]);
  });
});
