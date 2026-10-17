import { describe, expect, it } from "vitest";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { badgeLabel, cardActionLabel, cardTitle, isGone } from "./error-badge-model";

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
