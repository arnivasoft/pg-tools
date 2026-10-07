import { describe, expect, it } from "vitest";
import type { PlanView } from "../../ipc/bindings/PlanView";
import {
  applyOptions, canSubmit, dataGate, riskNeedsConfirm, shortenStatement,
  statementDisplay, visibleControls, type DataGateInput,
} from "./data-apply-model";

const view = (o: Partial<PlanView> = {}): PlanView => ({
  statements: [], warnings: [], errors: [], hash: "h1", risk: "safe", empty: false, ...o,
});
const base = (o: Partial<DataGateInput> = {}): DataGateInput => ({
  locked: false, production: false, editable: true, view: view(), viewIsCurrent: true,
  pending: false, applying: false, connected: true, loading: false, needsReload: false,
  outcomeUnknown: false, dirty: true, ...o,
});
const blocked = (reason: string) => ({ kind: "blocked", reason });

describe("dataGate", () => {
  it("hazır", () => expect(dataGate(base())).toEqual({ kind: "ready" }));

  // Her kapı, kendisinden SONRA gelen tüm kapılardan önceliklidir (çiftler).
  const on: [string, Partial<DataGateInput>][] = [
    ["disconnected", { connected: false }],
    ["loading", { loading: true }],
    ["locked", { locked: true }],
    ["readOnly", { editable: false }],
    ["unknown", { outcomeUnknown: true }],
    ["stale", { needsReload: true }],
    ["empty", { dirty: false }],
    ["invalid", { view: view({ errors: [{ path: "changes", code: "x", message: "m" } as never] }) }],
    ["pending", { pending: true }],
    ["applying", { applying: true }],
  ];
  for (let i = 0; i < on.length; i++) {
    it(`tek başına ${on[i][0]}`, () => expect(dataGate(base(on[i][1]))).toEqual(blocked(on[i][0])));
    for (let j = i + 1; j < on.length; j++) {
      it(`${on[i][0]} > ${on[j][0]}`, () =>
        expect(dataGate(base({ ...on[j][1], ...on[i][1] }))).toEqual(blocked(on[i][0])));
    }
  }

  it("boş plan (dirty ama view.empty) -> empty", () =>
    expect(dataGate(base({ view: view({ empty: true }) }))).toEqual(blocked("empty")));
  it("view null + dirty -> pending", () =>
    expect(dataGate(base({ view: null }))).toEqual(blocked("pending")));
  it("bayat görünüm -> pending", () =>
    expect(dataGate(base({ viewIsCurrent: false }))).toEqual(blocked("pending")));

  it("üretim -> needsConfirm production", () =>
    expect(dataGate(base({ production: true }))).toEqual({ kind: "needsConfirm", production: true, destructive: false }));
  it("yıkıcı -> needsConfirm destructive", () =>
    expect(dataGate(base({ view: view({ risk: "destructive" }) }))).toEqual({ kind: "needsConfirm", production: false, destructive: true }));
  it("data_loss + üretim -> ikisi de", () =>
    expect(dataGate(base({ production: true, view: view({ risk: "data_loss" }) }))).toEqual({ kind: "needsConfirm", production: true, destructive: true }));
  it("üretim + boş plan onay istemez (empty)", () =>
    expect(dataGate(base({ production: true, view: view({ empty: true }) }))).toEqual(blocked("empty")));
});

describe("canSubmit", () => {
  it("ready -> true", () => expect(canSubmit({ kind: "ready" }, false)).toBe(true));
  it("needsConfirm işaretsiz -> false, işaretli -> true", () => {
    const g = { kind: "needsConfirm", production: true, destructive: false } as const;
    expect(canSubmit(g, false)).toBe(false);
    expect(canSubmit(g, true)).toBe(true);
  });
  it("blocked -> her zaman false", () =>
    expect(canSubmit({ kind: "blocked", reason: "empty" }, true)).toBe(false));
});

describe("riskNeedsConfirm", () => {
  it("safe + üretim dışı", () => expect(riskNeedsConfirm(view(), false)).toBe(false));
  it("safe + üretim", () => expect(riskNeedsConfirm(view(), true)).toBe(true));
  it("destructive", () => expect(riskNeedsConfirm(view({ risk: "destructive" }), false)).toBe(true));
  it("data_loss", () => expect(riskNeedsConfirm(view({ risk: "data_loss" }), false)).toBe(true));
  it("boş plan + üretim", () => expect(riskNeedsConfirm(view({ empty: true }), true)).toBe(false));
});

describe("applyOptions", () => {
  it("alanlar", () => {
    expect(applyOptions("id-1", view({ hash: "abc" }), true)).toEqual(
      { applyId: "id-1", lockTimeoutMs: 5000, expectedHash: "abc", confirmed: true });
    expect(applyOptions("id-2", view(), false, 0)).toEqual(
      { applyId: "id-2", lockTimeoutMs: 0, expectedHash: "h1", confirmed: false });
  });
});

describe("visibleControls", () => {
  it("düz metin değişmez", () => expect(visibleControls("select 1; ğüşiöç")).toBe("select 1; ğüşiöç"));
  it("RLO", () => expect(visibleControls("a\u202Eb")).toBe("a⟦U+202E⟧b"));
  it("ZWSP", () => expect(visibleControls("a\u200Bb")).toBe("a⟦U+200B⟧b"));
  it("BOM", () => expect(visibleControls("\uFEFFx")).toBe("⟦U+FEFF⟧x"));
  it("NEL (C1)", () => expect(visibleControls("a\u0085b")).toBe("a⟦U+0085⟧b"));
  it("C0", () => expect(visibleControls("a\u0001b\u007F")).toBe("a⟦U+0001⟧b⟦U+007F⟧"));
  it("U+2028/2029/061C/2066/2069", () =>
    expect(visibleControls("\u2028\u2029\u061C\u2066\u2069")).toBe("⟦U+2028⟧⟦U+2029⟧⟦U+061C⟧⟦U+2066⟧⟦U+2069⟧"));
  it("U+2060 (WJ), U+206A, U+206F, U+00AD (SHY), U+034F, U+180E kaçışlanır", () =>
    expect(visibleControls("a⁠b⁪c⁯d­e͏f᠎g")).toBe(
      "a⟦U+2060⟧b⟦U+206A⟧c⟦U+206F⟧d⟦U+00AD⟧e⟦U+034F⟧f⟦U+180E⟧g",
    ));
  it("NBSP (U+00A0), varyasyon seçicisi ve etiket karakterleri korunur", () => {
    expect(visibleControls("a b")).toBe("a b");
    expect(visibleControls("❤️")).toBe("❤️");
    expect(visibleControls("🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}")).toBe(
      "🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}",
    );
    expect(visibleControls("a😀\u{1F3FD}b")).toBe("a😀\u{1F3FD}b");
  });
  it("\t \n \r korunur", () => expect(visibleControls("a\tb\nc\rd")).toBe("a\tb\nc\rd"));
  it("emoji korunur", () => expect(visibleControls("a😀b")).toBe("a😀b"));
  it("yalnız vekil", () => {
    expect(visibleControls("a\uD800b")).toBe("a⟦U+D800⟧b");
    expect(visibleControls("a\uDC00")).toBe("a⟦U+DC00⟧");
  });
});

describe("shortenStatement", () => {
  it("boş", () => expect(shortenStatement("")).toEqual({ text: "", omitted: 0 }));
  it("tam sınır değişmez", () => {
    const s = "a".repeat(4000);
    expect(shortenStatement(s)).toEqual({ text: s, omitted: 0 });
  });
  it("sınır+1", () => {
    const r = shortenStatement("a".repeat(4001));
    expect(r.text.length).toBe(4000);
    expect(r.omitted).toBe(1);
  });
  it("özel max", () => expect(shortenStatement("abcdef", 4)).toEqual({ text: "abcd", omitted: 2 }));
  it("emoji kesim noktasında bölünmez", () => {
    const r = shortenStatement("abc😀def", 4);
    expect(r).toEqual({ text: "abc", omitted: 5 });
  });
  it("emoji tam sığarsa korunur", () =>
    expect(shortenStatement("abc😀def", 5)).toEqual({ text: "abc😀", omitted: 3 }));
});

describe("statementDisplay", () => {
  it("kısa metin: yalnız kaçış", () =>
    expect(statementDisplay("a\u202Eb")).toEqual({ text: "a⟦U+202E⟧b", omitted: 0 }));
  it("kaçışlı metin 4000'i aşmaz ve omitted özgün karakter sayar", () => {
    const sql = "\u202E".repeat(1000); // her biri 8 karakter
    const r = statementDisplay(sql);
    expect(r.text.length).toBeLessThanOrEqual(4000);
    expect(r.text.length).toBe(4000); // 500 kaçış x 8
    expect(r.omitted).toBe(500);
    expect(r.text.endsWith("⟧")).toBe(true);
  });
  it("kaçış ortadan bölünmez", () => {
    const r = statementDisplay("a".repeat(3996) + "\u202E");
    expect(r.text).toBe("a".repeat(3996));
    expect(r.omitted).toBe(1);
  });
  it("emoji bölünmez", () => {
    const r = statementDisplay("a".repeat(3999) + "😀");
    expect(r.text).toBe("a".repeat(3999));
    expect(r.omitted).toBe(2);
  });
});
