import { describe, expect, it } from "vitest";
import type { MatViewDef } from "../../ipc/bindings/MatViewDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { ViewDef } from "../../ipc/bindings/ViewDef";
import {
  asMatView, asView, checkOptionFromSelect, lossList, matviewEditPatch, matviewRebuild, queryChanged, recreateVisible, refreshChoices,
  viewEditPatch, withDataPatch, withQuery,
} from "./view-model";

const view = (p: Partial<ViewDef> = {}): ViewDef => ({
  schema: "public", name: "v", owner: null, query: "SELECT 1", checkOption: null, securityBarrier: false, securityInvoker: false,
  comment: null, recreate: false, columns: [], unsupported: [], ...p,
});
const mv = (p: Partial<MatViewDef> = {}): MatViewDef => ({
  schema: "public", name: "m", owner: null, query: "SELECT 1", tablespace: null, fillfactor: null, withData: true, comment: null,
  refresh: null, columns: [], unsupported: [], ...p,
});
const vd = (p: Partial<ViewDef> = {}): ObjectDef => ({ kind: "view", ...view(p) });
const md = (p: Partial<MatViewDef> = {}): ObjectDef => ({ kind: "materialized_view", ...mv(p) });

describe("view-model", () => {
  it("asView/asMatView tür denetimi; edit patch tür uyuşmazlığında aynı nesne", () => {
    expect(asView(vd())?.kind).toBe("view");
    expect(asView(md())).toBeNull();
    expect(asView(null)).toBeNull();
    expect(asMatView(md())?.kind).toBe("materialized_view");
    expect(asMatView(vd())).toBeNull();
    const m = md();
    expect(viewEditPatch(m, { name: "x" })).toBe(m);
    const v = vd();
    expect(matviewEditPatch(v, { name: "x" })).toBe(v);
    expect(viewEditPatch(v, { name: "x" })).toMatchObject({ kind: "view", name: "x" });
    expect(matviewEditPatch(m, { name: "x" })).toMatchObject({ kind: "materialized_view", name: "x" });
  });

  it("queryChanged: create (original null) → false; metin farkı → true; aynı metin → false", () => {
    expect(queryChanged(null, view({ query: "SELECT 2" }))).toBe(false);
    expect(queryChanged(view(), view({ query: "SELECT 2" }))).toBe(true);
    expect(queryChanged(view(), view({ query: "SELECT 1 " }))).toBe(true);
    expect(queryChanged(view(), view())).toBe(false);
  });

  it("recreateVisible: yalnız alter + view + sorgu değişmiş; withQuery sorgu özgüne dönünce recreate=false", () => {
    const o = view();
    const changed = view({ query: "SELECT 2" });
    expect(recreateVisible("alter", o, changed)).toBe(true);
    expect(recreateVisible("alter", o, o)).toBe(false);
    expect(recreateVisible("create", null, changed)).toBe(false);
    expect(recreateVisible("create", o, changed)).toBe(false);
    const r = withQuery(view({ query: "SELECT 2", recreate: true }), o, "SELECT 1");
    expect(r.recreate).toBe(false);
    expect(r.query).toBe("SELECT 1");
    expect(withQuery(view({ recreate: true }), o, "SELECT 2").recreate).toBe(true);
    expect(withQuery(mv(), mv(), "SELECT 9").query).toBe("SELECT 9");
  });

  it("lossList: unsupported + 'GRANT yetkileri'; create → []", () => {
    expect(lossList(null)).toEqual([]);
    expect(lossList(view({ unsupported: ["TRIGGER t"] }))).toEqual(["TRIGGER t", "GRANT yetkileri"]);
    expect(lossList(mv())).toEqual(["GRANT yetkileri"]);
  });

  it("refreshChoices: Plan 14 tablosunun her satırı; tanım değişince yalnız [null]", () => {
    expect(refreshChoices(null, mv())).toEqual([null]);
    expect(refreshChoices(mv(), mv({ query: "SELECT 2", withData: true }))).toEqual([null]);
    expect(refreshChoices(mv({ withData: false }), mv({ withData: true }))).toEqual([null, "normal"]);
    expect(refreshChoices(mv({ withData: true }), mv({ withData: false }))).toEqual([null]);
    expect(refreshChoices(mv({ withData: true }), mv({ withData: true }))).toEqual([null, "normal", "concurrently"]);
    expect(refreshChoices(mv({ withData: false }), mv({ withData: false }))).toEqual([null]);
  });

  it("withDataPatch: refresh yeni seçeneklerde yoksa null'a çekilir (true,true,concurrently → false: null)", () => {
    const o = mv({ withData: true });
    expect(withDataPatch(o, mv({ withData: true, refresh: "concurrently" }), false)).toMatchObject({ withData: false, refresh: null });
    expect(withDataPatch(o, mv({ withData: true, refresh: "concurrently" }), true)).toMatchObject({ withData: true, refresh: "concurrently" });
    expect(withDataPatch(mv({ withData: false }), mv({ withData: false }), true)).toMatchObject({ withData: true, refresh: null });
  });

  it("matviewRebuild: yalnız alter + sorgu farkı", () => {
    expect(matviewRebuild(null, mv({ query: "x" }))).toBe(false);
    expect(matviewRebuild(mv(), mv())).toBe(false);
    expect(matviewRebuild(mv(), mv({ query: "SELECT 2" }))).toBe(true);
  });

  it("checkOptionFromSelect: '' → null; local/cascaded", () => {
    expect(checkOptionFromSelect("")).toBeNull();
    expect(checkOptionFromSelect("local")).toBe("local");
    expect(checkOptionFromSelect("cascaded")).toBe("cascaded");
    expect(checkOptionFromSelect("zzz")).toBeNull();
  });
});
