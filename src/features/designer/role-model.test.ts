import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { RoleDef } from "../../ipc/bindings/RoleDef";
import type { RoleMembership } from "../../ipc/bindings/RoleMembership";
import { defsEqual } from "./designer-model";
import * as model from "./role-model";
import {
  ROLE_FLAGS, addMembership, asRole, availableRoles, passwordChoices, passwordIssue, privilegedGrants, removeMembership, removedMemberships,
  restoreMembership, roleEditPatch, selfLockoutWarnings, toggleAdmin, validUntilClearValue, validUntilFromParts, validUntilParts,
} from "./role-model";

const role = (p: Partial<RoleDef> = {}): RoleDef => ({
  name: "app", login: true, superuser: false, createdb: false, createrole: false, inherit: true, replication: false, bypassrls: false,
  connectionLimit: -1, validUntil: null, password: "keep", memberOf: [], settings: [], comment: null, unsupported: [], ...p,
});
const rd = (p: Partial<RoleDef> = {}): ObjectDef => ({ kind: "role", ...role(p) });
const mem = (r: string, p: Partial<RoleMembership> = {}): RoleMembership => ({ role: r, admin: false, inheritOption: null, setOption: null, ...p });
const schema = { kind: "schema", name: "s", owner: null, comment: null } as ObjectDef;

beforeAll(() => vi.stubEnv("TZ", "UTC"));
afterAll(() => vi.unstubAllEnvs());

describe("role-model", () => {
  it("asRole / roleEditPatch tür denetimi; ROLE_FLAGS", () => {
    expect(asRole(rd())?.kind).toBe("role");
    expect(asRole(null)).toBeNull();
    expect(asRole(schema)).toBeNull();
    expect(roleEditPatch(rd(), { login: false })).toMatchObject({ kind: "role", login: false });
    expect(roleEditPatch(schema, { name: "x" })).toBe(schema);
    expect(ROLE_FLAGS).toEqual(["login", "superuser", "createdb", "createrole", "inherit", "replication", "bypassrls"]);
  });

  it("privilegedGrants: create'te true olan ayrıcalıklılar; alter'da yalnız false→true", () => {
    expect(privilegedGrants(null, role())).toEqual([]);
    expect(privilegedGrants(null, role({ superuser: true, createdb: true, bypassrls: true, login: true }))).toEqual(["superuser", "bypassrls"]);
    expect(privilegedGrants(null, role({ createrole: true, replication: true }))).toEqual(["createrole", "replication"]);
    const o = role({ superuser: true, replication: false });
    expect(privilegedGrants(o, role({ superuser: true }))).toEqual([]);
    expect(privilegedGrants(o, role({ superuser: true, replication: true, createdb: true }))).toEqual(["replication"]);
    expect(privilegedGrants(o, role({ superuser: false }))).toEqual([]);
  });

  it("passwordChoices ve passwordIssue: boş, eşleşmeyen, eşleşen", () => {
    expect(passwordChoices("create")).toEqual(["keep", "set"]);
    expect(passwordChoices("alter")).toEqual(["keep", "set", "clear"]);
    expect(passwordIssue("", "")).toBe("empty");
    expect(passwordIssue("", "x")).toBe("empty");
    expect(passwordIssue("abc", "")).toBe("mismatch");
    expect(passwordIssue("abc", "abd")).toBe("mismatch");
    expect(passwordIssue("abc", "abc")).toBeNull();
    expect(passwordIssue(" ", " ")).toBeNull(); // boşluk parola sayılır (değer olduğu gibi)
  });

  it("addMembership: boş/yinelenen/kendisi eklenmez; inheritOption/setOption null; removeMembership/toggleAdmin değişmez", () => {
    const d = role({ memberOf: [mem("a", { admin: true, inheritOption: true, setOption: false })] });
    expect(addMembership(d, "")).toBe(d);
    expect(addMembership(d, "   ")).toBe(d);
    expect(addMembership(d, "a")).toBe(d);
    expect(addMembership(d, "app")).toBe(d);
    const n = addMembership(d, "b");
    expect(n.memberOf).toEqual([mem("a", { admin: true, inheritOption: true, setOption: false }), mem("b")]);
    expect(d.memberOf).toHaveLength(1);
    expect(removeMembership(n, 0).memberOf).toEqual([mem("b")]);
    expect(removeMembership(n, 5)).toBe(n);
    expect(removeMembership(n, -1)).toBe(n);
    expect(n.memberOf).toHaveLength(2);
    const t = toggleAdmin(n, 1, true);
    expect(t.memberOf[1]).toEqual(mem("b", { admin: true }));
    expect(n.memberOf[1]!.admin).toBe(false);
    expect(toggleAdmin(n, 9, true)).toBe(n);
    // PG16 seçenekleri admin değişiminde korunur
    expect(toggleAdmin(n, 0, false).memberOf[0]).toEqual(mem("a", { admin: false, inheritOption: true, setOption: false }));
  });

  it("removedMemberships/restoreMembership: özgün nesnenin kopyası (PG16 seçenekleri dahil) ad sırasına; defsEqual(original) → true", () => {
    const o = role({ memberOf: [mem("a"), mem("b", { admin: true, inheritOption: false, setOption: true }), mem("c")] });
    expect(removedMemberships(null, o)).toEqual([]);
    expect(removedMemberships(o, o)).toEqual([]);
    let d = removeMembership(o, 1);
    d = removeMembership(d, 0);
    expect(removedMemberships(o, d).map((m) => m.role)).toEqual(["a", "b"]);
    d = restoreMembership(d, o, "b");
    expect(d.memberOf.map((m) => m.role)).toEqual(["b", "c"]);
    expect(d.memberOf[0]).toEqual(o.memberOf[1]);
    expect(d.memberOf[0]).not.toBe(o.memberOf[1]); // kopya
    d = restoreMembership(d, o, "a");
    expect(defsEqual({ kind: "role", ...d }, { kind: "role", ...o })).toBe(true);
    expect(restoreMembership(d, o, "a")).toBe(d); // zaten var
    expect(restoreMembership(d, o, "zzz")).toBe(d); // özgünde yok
    // yeni eklenmiş üyelik varken de özgün sıradaki yerine
    let e = addMembership(removeMembership(o, 0), "z");
    e = restoreMembership(e, o, "a");
    expect(e.memberOf.map((m) => m.role)).toEqual(["a", "b", "c", "z"]);
  });

  it("availableRoles: mevcut üyelikler ve rolün kendisi hariç; pg_* önceden tanımlı roller dahil", () => {
    const items = [
      { value: "a", detail: null }, { value: "app", detail: null }, { value: "b", detail: null }, { value: "pg_read_all_data", detail: "önceden tanımlı" },
    ];
    expect(availableRoles(items, role({ memberOf: [mem("a")] })).map((i) => i.value)).toEqual(["b", "pg_read_all_data"]);
  });

  it("validUntil: 'infinity' ↔ never; '2030-01-01T10:00:00Z' ↔ parçalar (yerel saat dönüşümü test saat dilimi sabit UTC); geçersiz → null; validUntilClearValue", () => {
    expect(validUntilParts("infinity")).toEqual({ never: true, date: "", time: "" });
    expect(validUntilParts(null)).toEqual({ never: true, date: "", time: "" });
    expect(validUntilParts("2030-01-01T10:00:00Z")).toEqual({ never: false, date: "2030-01-01", time: "10:00" });
    expect(validUntilParts("2030-01-01T10:05:30.123456Z")).toEqual({ never: false, date: "2030-01-01", time: "10:05" });
    expect(validUntilParts("bozuk")).toBeNull();
    expect(validUntilFromParts({ never: true, date: "", time: "" })).toBe("infinity");
    expect(validUntilFromParts({ never: false, date: "2030-01-01", time: "10:00" })).toBe("2030-01-01T10:00:00Z");
    expect(validUntilFromParts({ never: false, date: "2030-01-01", time: "" })).toBe("2030-01-01T00:00:00Z");
    expect(validUntilFromParts({ never: false, date: "", time: "10:00" })).toBeNull();
    expect(validUntilFromParts({ never: false, date: "2030-02-31", time: "10:00" })).toBeNull();
    expect(validUntilFromParts({ never: false, date: "2030-01-01", time: "25:00" })).toBeNull();
    expect(validUntilFromParts({ never: false, date: "x", time: "10:00" })).toBeNull();
    // gidiş-dönüş
    const p = validUntilParts("2031-06-15T23:45:00Z")!;
    expect(validUntilFromParts(p)).toBe("2031-06-15T23:45:00Z");
    expect(validUntilClearValue(null)).toBeNull();
    expect(validUntilClearValue(role())).toBeNull();
    expect(validUntilClearValue(role({ validUntil: "2030-01-01T10:00:00Z" }))).toBe("infinity");
    expect(validUntilClearValue(role({ validUntil: "infinity" }))).toBe("infinity");
  });

  it("role-model hiçbir dışa aktarımında parola değeri tutmaz (modül durumsuz: aynı girdiler aynı çıktı)", () => {
    const SECRET = "Gizli-Parola-1";
    expect(passwordIssue(SECRET, SECRET)).toBeNull();
    expect(passwordIssue(SECRET, `${SECRET}x`)).toBe("mismatch");
    expect(passwordIssue(SECRET, SECRET)).toBeNull();
    const state = Object.entries(model).filter(([, v]) => typeof v !== "function");
    expect(JSON.stringify(state)).not.toContain(SECRET);
    // dönüş değerleri yalnızca sabit kodlar
    for (const r of [passwordIssue(SECRET, ""), passwordIssue(SECRET, SECRET)]) expect(r === null || !String(r).includes(SECRET)).toBe(true);
  });

  it("selfLockoutWarnings: yalnız alter + bağlı kullanıcı; nologin/connectionLimitZero/validUntilPast/ownPassword; başka rol ya da create → []", () => {
    const now = new Date("2026-10-07T12:00:00Z");
    const o = role({ name: "app", login: true, connectionLimit: -1, validUntil: null });
    expect(selfLockoutWarnings("app", o, role({ name: "app" }), now)).toEqual([]);
    const bad = role({ login: false, connectionLimit: 0, validUntil: "2020-01-01T00:00:00Z", password: "set" });
    expect(selfLockoutWarnings("app", o, bad, now)).toEqual(["nologin", "connectionLimitZero", "validUntilPast", "ownPassword"]);
    expect(selfLockoutWarnings("app", o, role({ password: "clear" }), now)).toEqual(["ownPassword"]);
    expect(selfLockoutWarnings("app", o, role({ validUntil: "2030-01-01T00:00:00Z" }), now)).toEqual([]);
    expect(selfLockoutWarnings("app", o, role({ validUntil: "infinity" }), now)).toEqual([]);
    // yüklenen durum zaten aynıysa uyarı yok (değişmeyen)
    const o2 = role({ login: false, connectionLimit: 0 });
    expect(selfLockoutWarnings("app", o2, role({ login: false, connectionLimit: 0 }), now)).toEqual([]);
    // başka rol / bağlı kullanıcı bilinmiyor / create
    expect(selfLockoutWarnings("other", o, bad, now)).toEqual([]);
    expect(selfLockoutWarnings(null, o, bad, now)).toEqual([]);
    expect(selfLockoutWarnings("app", null, bad, now)).toEqual([]);
    // yeniden adlandırılıyor olsa da özgün ada göre
    expect(selfLockoutWarnings("app", o, role({ name: "renamed", login: false }), now)).toEqual(["nologin"]);
  });
});
