import { describe, expect, it, vi } from "vitest";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { createCardLoader, badgeLabel, cardActionLabel, cardTitle, isGone } from "./error-badge-model";

const err = (over: Partial<ErrorPayload>): ErrorPayload => ({ code: "x", message: "boom", detail: null, hint: null, ...over }) as ErrorPayload;

describe("error-badge-model", () => {
  it("rozet etiketi her hatada aynı, tek kaynaktan", () => {
    expect(badgeLabel()).toBe(t.error.badge);
    expect(badgeLabel()).toBe(t.error.badge);
  });
  it("not_found: kayıp metni ve üst düğümü yenile", () => {
    const e = err({ code: "not_found" });
    expect(isGone(e)).toBe(true);
    expect(cardTitle(e)).toBe(t.error.gone);
    expect(cardActionLabel(e)).toBe(t.error.refreshParent);
  });
  it("diğer hatalar: ileti ve tekrar dene", () => {
    const e = err({});
    expect(isGone(e)).toBe(false);
    expect(cardTitle(e)).toBe("boom");
    expect(cardActionLabel(e)).toBe(t.error.retry);
  });
});

describe("createCardLoader", () => {
  it("eşzamanlı istekler tek yüklemeyi paylaşır, başarıda bir kez bildirir", async () => {
    const load = vi.fn(async () => "mod");
    const loaded = vi.fn();
    const ensure = createCardLoader(load, loaded, vi.fn());
    expect(await Promise.all([ensure(), ensure(), ensure()])).toEqual([true, true, true]);
    await ensure();
    expect(load).toHaveBeenCalledTimes(1);
    expect(loaded).toHaveBeenCalledWith("mod");
  });
  it("ret: onFail rozet sayısından bağımsız bir kez; sonraki istek yeniden dener", async () => {
    let fail = true;
    const load = vi.fn(async () => { if (fail) throw new Error("x"); return "mod"; });
    const onFail = vi.fn();
    const loaded = vi.fn();
    const ensure = createCardLoader(load, loaded, onFail);
    expect(await Promise.all([ensure(), ensure(), ensure()])).toEqual([false, false, false]);
    expect(onFail).toHaveBeenCalledTimes(1);
    expect(loaded).not.toHaveBeenCalled();
    fail = false;
    expect(await ensure()).toBe(true);
    expect(loaded).toHaveBeenCalledWith("mod");
  });
});
