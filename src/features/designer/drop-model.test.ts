import { describe, expect, it } from "vitest";
import { tdes } from "../../app/strings-designer";
import type { ApplyOutcome } from "../../ipc/bindings/ApplyOutcome";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import { planRequest } from "./designer-controller";
import { dropRequest, dropStatusText, matchesObject } from "./drop-model";

const def = (): ObjectDef => ({ kind: "schema", name: "app", owner: "alice", comment: "c" });
const err = (code: string, message = `m:${code}`, detail: string | null = null) => ({ code, message, detail, hint: null });
const failed = (p: Partial<Extract<ApplyOutcome, { outcome: "failed" }>> = {}): Extract<ApplyOutcome, { outcome: "failed" }> => ({
  outcome: "failed", failedIndex: 0, committed: 0, sqlstate: null, position: null, error: err("query", "boom"), ...p,
});

describe("dropRequest", () => {
  it("object + yüklenen tanımın derin kopyası + cascade; girdiyle nesne paylaşmaz", () => {
    const d = def();
    const object = { kind: "schema" as const, oid: 7 };
    const r = dropRequest(object, d, true);
    expect(r).toEqual({ op: "drop", object, def: d, cascade: true, force: false });
    expect(r.def).not.toBe(d);
    expect(r.object).not.toBe(object);
  });

  it("designer planRequest(m, 'drop') aynı tanımı kullanır", () => {
    const original = def();
    const m = {
      mode: "alter" as const, kind: "schema" as const, object: { kind: "schema" as const, oid: 7 }, original,
      draft: { ...original, name: "taslak" }, concurrently: false, cascade: false,
    };
    expect(planRequest(m, "drop")).toEqual(dropRequest(m.object, original, false));
  });
});

describe("dropStatusText", () => {
  it("Failed 2BP01 → bağımlı nesne metni + PG detail; detail yoksa PG iletisi", () => {
    const out = failed({ sqlstate: "2BP01", error: err("query", "cannot drop schema app", "table app.t depends on schema app") });
    expect(dropStatusText(null, out)).toEqual({ kind: "dependents", text: tdes.drop.dependents, detail: "table app.t depends on schema app" });
    const bare = failed({ sqlstate: "2BP01", error: err("query", "cannot drop") });
    expect(dropStatusText(null, bare)?.detail).toBe("cannot drop");
  });

  it("metin üzerinden eşleşme yok: 'depends on' içeren başka sqlstate genel hata", () => {
    const out = failed({ sqlstate: "42501", error: err("query", "depends on", "other objects depend on it") });
    expect(dropStatusText(null, out)?.kind).toBe("other");
  });

  it("Failed 55P03 → kilit zaman aşımı; kısmen kalıcı → stale + uyarı", () => {
    expect(dropStatusText(null, failed({ sqlstate: "55P03" }))?.text).toContain(tdes.apply.lockTimeout);
    expect(dropStatusText(null, failed({ committed: 1 }))).toMatchObject({ kind: "stale", detail: tdes.apply.committed(1) });
  });

  it("kodlar: plan_changed/not_found → stale; bağlantı → disconnected; locked; diğer → designerErrorText", () => {
    expect(dropStatusText(err("plan_changed"))).toEqual({ kind: "stale", text: tdes.drop.stale, detail: null });
    expect(dropStatusText(err("not_found"))?.kind).toBe("stale");
    for (const c of ["not_connected", "connection_lost", "unknown_server"]) expect(dropStatusText(err(c))?.kind).toBe("disconnected");
    expect(dropStatusText(err("locked"))).toMatchObject({ kind: "locked", text: tdes.gate.locked });
    expect(dropStatusText(err("confirmation_required"))).toMatchObject({ kind: "other", text: tdes.gate.confirmationRequired });
    expect(dropStatusText(err("query", "x", "d"))).toMatchObject({ kind: "other", text: "x — d" });
    expect(dropStatusText(null)).toBeNull();
  });
});

describe("matchesObject", () => {
  const ref = { serverId: "S1", databaseOid: 5, object: { kind: "schema" as const, oid: 7 } };
  it("yalnızca aynı sunucu/db/tür/oid'li alter sekmesi", () => {
    const tab = { serverId: "S1", databaseOid: 5, mode: "alter" as const, object: { kind: "schema" as const, oid: 7 } };
    expect(matchesObject(tab, ref)).toBe(true);
    expect(matchesObject({ ...tab, serverId: "S2" }, ref)).toBe(false);
    expect(matchesObject({ ...tab, databaseOid: 6 }, ref)).toBe(false);
    expect(matchesObject({ ...tab, object: { kind: "sequence", oid: 7 } }, ref)).toBe(false);
    expect(matchesObject({ ...tab, object: { kind: "schema", oid: 8 } }, ref)).toBe(false);
    expect(matchesObject({ ...tab, mode: "create" }, ref)).toBe(false);
    expect(matchesObject({ ...tab, object: null }, ref)).toBe(false);
  });
});
