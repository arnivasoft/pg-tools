import { describe, expect, it, vi } from "vitest";
import type { StmtPlan } from "../../ipc/bindings/StmtPlan";
import { decide, runWithGuards, type GuardDeps } from "./confirm-model";

const plan = (o: Partial<StmtPlan>, charOffset = 0, charLen = 5): StmtPlan =>
  ({
    index: 0,
    kind: "dml",
    danger: null,
    productionWrite: false,
    lockedReject: false,
    lockedReason: null,
    sensitive: false,
    charOffset,
    charLen,
    ...o,
  }) as StmtPlan;

describe("decide", () => {
  it("WHERE'siz UPDATE -> confirm(danger)", () => {
    const d = decide([plan({ danger: "updateWithoutWhere" })], "update t");
    expect(d).toMatchObject({ kind: "confirm", items: [{ reason: "danger", danger: "updateWithoutWhere", text: "updat" }] });
  });
  it("üretim + insert -> confirm(production)", () => {
    const d = decide([plan({ productionWrite: true })], "insert into t");
    expect(d).toMatchObject({ kind: "confirm", items: [{ reason: "production", production: true }] });
  });
  it("üretim + DO -> confirm", () => {
    expect(decide([plan({ productionWrite: true }, 0, 9)], "do $$ $$ x").kind).toBe("confirm");
  });
  it("kilitli + delete -> locked", () => {
    const d = decide([plan({ lockedReject: true, lockedReason: "write" })], "delete x");
    expect(d).toMatchObject({ kind: "locked", items: [{ reason: "write" }] });
  });
  it("zararsız -> run", () => {
    expect(decide([plan({})], "select 1").kind).toBe("run");
  });
});

const rej = (code: string) => ({ code, message: code, detail: null, hint: null });
const all = { type: "all" } as const;

function mk(over: Partial<GuardDeps> = {}) {
  const deps: GuardDeps = {
    analyze: vi.fn(async () => [plan({ danger: "truncate" }, 0, 8)]),
    run: vi.fn(async () => null),
    confirm: vi.fn(async () => true),
    onEvent: () => {},
    session: "s",
    limit: 100,
    ...over,
  };
  return deps;
}

describe("runWithGuards", () => {
  it("confirmation is bound to the analyzed snapshot", async () => {
    let editor = "truncate t";
    const snap = { text: editor, scope: all };
    const deps = mk({
      confirm: vi.fn(async () => {
        editor = "drop table t"; // diyalog açıkken editör değişir
        return true;
      }),
    });
    expect(await runWithGuards(snap, deps)).toEqual({ kind: "done" });
    expect(editor).toBe("drop table t");
    expect(deps.analyze).toHaveBeenCalledWith("s", "truncate t", all);
    expect(deps.run).toHaveBeenCalledWith("s", "truncate t", all, 100, true, deps.onEvent);
  });
  it("locked -> çalıştırma yok", async () => {
    const deps = mk({ analyze: vi.fn(async () => [plan({ lockedReject: true, lockedReason: "ddl" })]) });
    const out = await runWithGuards({ text: "create t", scope: all }, deps);
    expect(out.kind).toBe("locked");
    expect(deps.run).not.toHaveBeenCalled();
    expect(deps.confirm).not.toHaveBeenCalled();
  });
  it("onay gerekmezse confirmed=false ile çalışır", async () => {
    const deps = mk({ analyze: vi.fn(async () => [plan({})]) });
    await runWithGuards({ text: "select 1", scope: all }, deps);
    expect(deps.confirm).not.toHaveBeenCalled();
    expect(deps.run).toHaveBeenCalledWith("s", "select 1", all, 100, false, deps.onEvent);
  });
  it("diyalogda vazgeç -> çalıştırma yok", async () => {
    const deps = mk({ confirm: vi.fn(async () => false) });
    expect(await runWithGuards({ text: "truncate t", scope: all }, deps)).toEqual({ kind: "cancelled" });
    expect(deps.run).not.toHaveBeenCalled();
  });
  it("confirmation_required -> yeniden analiz ve bir kez daha sor", async () => {
    const run = vi.fn().mockRejectedValueOnce(rej("confirmation_required")).mockResolvedValueOnce(null);
    const deps = mk({ run });
    expect(await runWithGuards({ text: "truncate t", scope: all }, deps)).toEqual({ kind: "done" });
    expect(deps.analyze).toHaveBeenCalledTimes(2);
    expect(deps.confirm).toHaveBeenCalledTimes(2);
    expect(run).toHaveBeenCalledTimes(2);
  });
  it("confirmation_required ikinci kez -> hata yüzeye çıkar", async () => {
    const deps = mk({ run: vi.fn().mockRejectedValue(rej("confirmation_required")) });
    const out = await runWithGuards({ text: "truncate t", scope: all }, deps);
    expect(out).toMatchObject({ kind: "rejected", error: { code: "confirmation_required" } });
    expect(deps.run).toHaveBeenCalledTimes(2);
  });
  it("busy -> rejected", async () => {
    const deps = mk({ run: vi.fn().mockRejectedValue(rej("busy")) });
    expect(await runWithGuards({ text: "truncate t", scope: all }, deps)).toMatchObject({
      kind: "rejected",
      error: { code: "busy" },
    });
  });
});
