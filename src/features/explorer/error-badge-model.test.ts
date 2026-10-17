import { describe, expect, it, vi } from "vitest";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { createBadgeCardLoader, badgeLabel, cardActionLabel, cardTitle, isGone } from "./error-badge-model";

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

// createCardLoader testleri src/ui/lazy-load.test.ts'e taşındı (createLoader ile birleştirildi).

describe("createBadgeCardLoader", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));

  it("başarısızlıktan sonra start yeniden denemez, retry dener; ret sessizdir", async () => {
    const load = vi.fn(() => Promise.reject(new Error("ağ")));
    const l = createBadgeCardLoader(load);
    l.start();
    await flush();
    expect(load).toHaveBeenCalledTimes(1);
    l.start();
    l.start();
    await flush();
    expect(load).toHaveBeenCalledTimes(1);
    l.retry();
    await flush();
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("retry başarılı olunca başarısızlık temizlenir; start sonrası yeniden yüklemez (yüklü)", async () => {
    let ok = false;
    const load = vi.fn(() => (ok ? Promise.resolve() : Promise.reject(new Error("x"))));
    const l = createBadgeCardLoader(load);
    l.start();
    await flush();
    ok = true;
    l.retry();
    await flush();
    l.start();
    await flush();
    expect(load).toHaveBeenCalledTimes(2);
  });
});
