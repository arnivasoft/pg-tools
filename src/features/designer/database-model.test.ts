import { describe, expect, it } from "vitest";
import type { DatabaseDef } from "../../ipc/bindings/DatabaseDef";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import {
  asDatabase, connectionLimitFromInput, createOnlyLocked, databaseEditPatch, localeFields, needsNoSessions, template0Hint, unlimitedToggle, withProvider,
} from "./database-model";

const db = (p: Partial<DatabaseDef> = {}): DatabaseDef => ({
  name: "d", owner: null, template: null, encoding: null, localeProvider: null, lcCollate: null, lcCtype: null, providerLocale: null,
  tablespace: null, connectionLimit: -1, isTemplate: false, allowConnections: true, settings: [], comment: null, ...p,
});
const dd = (p: Partial<DatabaseDef> = {}): ObjectDef => ({ kind: "database", ...db(p) });
const schema = { kind: "schema", name: "s", owner: null, comment: null } as ObjectDef;

describe("database-model", () => {
  it("asDatabase / databaseEditPatch tür denetimi", () => {
    expect(asDatabase(dd())?.kind).toBe("database");
    expect(asDatabase(null)).toBeNull();
    expect(asDatabase(schema)).toBeNull();
    expect(databaseEditPatch(dd(), { name: "x" })).toMatchObject({ kind: "database", name: "x" });
    expect(databaseEditPatch(schema, { name: "x" })).toBe(schema);
  });

  it("createOnlyLocked yalnız alter", () => {
    expect(createOnlyLocked("alter")).toBe(true);
    expect(createOnlyLocked("create")).toBe(false);
  });

  it("localeFields tablosu", () => {
    expect(localeFields(null)).toEqual({ libc: true, providerLocale: null });
    expect(localeFields("libc")).toEqual({ libc: true, providerLocale: null });
    expect(localeFields("icu")).toEqual({ libc: false, providerLocale: "icu" });
    expect(localeFields("builtin")).toEqual({ libc: false, providerLocale: "builtin" });
  });

  it("withProvider: libc/null'a dönünce providerLocale null", () => {
    expect(withProvider(db({ localeProvider: "icu", providerLocale: "tr-TR" }), "libc")).toMatchObject({ localeProvider: "libc", providerLocale: null });
    expect(withProvider(db({ localeProvider: "icu", providerLocale: "tr-TR" }), null)).toMatchObject({ localeProvider: null, providerLocale: null });
    expect(withProvider(db({ providerLocale: "x" }), "icu")).toMatchObject({ localeProvider: "icu", providerLocale: "x" });
  });

  it("template0Hint: kodlama/yerel ayar dolu ve şablon template0 değil", () => {
    expect(template0Hint(db())).toBe(false);
    expect(template0Hint(db({ encoding: "UTF8" }))).toBe(true);
    expect(template0Hint(db({ lcCollate: "tr_TR" }))).toBe(true);
    expect(template0Hint(db({ lcCtype: "tr_TR" }))).toBe(true);
    expect(template0Hint(db({ encoding: "UTF8", template: "template0" }))).toBe(false);
    expect(template0Hint(db({ encoding: "UTF8", template: "template1" }))).toBe(true);
  });

  it("connectionLimitFromInput", () => {
    expect(connectionLimitFromInput("")).toBeUndefined();
    expect(connectionLimitFromInput("5")).toBe(5);
    expect(connectionLimitFromInput("0")).toBe(0);
    expect(connectionLimitFromInput("-1")).toBeUndefined();
    expect(connectionLimitFromInput("1.5")).toBeUndefined();
    expect(connectionLimitFromInput("5", true)).toBeUndefined();
  });

  it("needsNoSessions: rename/tablespace evet; owner hayır; create hayır", () => {
    const o = db({ tablespace: "pg_default" });
    expect(needsNoSessions(o, db({ name: "x", tablespace: "pg_default" }))).toBe(true);
    expect(needsNoSessions(o, db({ tablespace: "ts1" }))).toBe(true);
    expect(needsNoSessions(o, db({ owner: "bob", tablespace: "pg_default" }))).toBe(false);
    expect(needsNoSessions(null, db({ name: "x" }))).toBe(false);
  });

  it("withProvider lcCollate/lcCtype'a dokunmaz (arka uç icu/builtin ile kabul eder)", () => {
    expect(withProvider(db({ lcCollate: "C", lcCtype: "C" }), "icu")).toMatchObject({ lcCollate: "C", lcCtype: "C" });
  });

  it("unlimitedToggle: kutu -1; kaldırınca yüklü >=0 (0 dahil) ya da 100", () => {
    expect(unlimitedToggle(null, true)).toBe(-1);
    expect(unlimitedToggle(db({ connectionLimit: 7 }), false)).toBe(7);
    expect(unlimitedToggle(db({ connectionLimit: 0 }), false)).toBe(0);
    expect(unlimitedToggle(db({ connectionLimit: -1 }), false)).toBe(100);
    expect(unlimitedToggle(null, false)).toBe(100);
  });
});
