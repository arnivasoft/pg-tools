import { describe, expect, it } from "vitest";
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import type { Privilege } from "../../ipc/bindings/Privilege";
import {
  addGrantee, addGranteeInput, allowGrantOption, granteeOptions, cellPath, cellState, cycleCell, granteeLabel, nextCellFocus, nextCellState,
  removeGrantee, setCell, planHasRevoke, unknownPrivileges,
} from "./acl-grid-model";

const avail: Privilege[] = ["select", "insert", "update", "delete"];
const g = (grantee: string | null, ...p: [Privilege, boolean?][]): AclGrant => ({
  grantee,
  privileges: p.map(([privilege, o]) => ({ privilege, grantOption: o ?? false })),
});

describe("döngü", () => {
  it("GRANT OPTION izinliyse üç adım", () => {
    expect(nextCellState("none", true)).toBe("granted");
    expect(nextCellState("granted", true)).toBe("grantable");
    expect(nextCellState("grantable", true)).toBe("none");
  });
  it("izinli değilse grantable atlanır", () => {
    expect(nextCellState("none", false)).toBe("granted");
    expect(nextCellState("granted", false)).toBe("none");
    expect(nextCellState("grantable", false)).toBe("none");
  });
  it("cellState", () => {
    const r = g("a", ["select"], ["insert", true]);
    expect(cellState(r, "select")).toBe("granted");
    expect(cellState(r, "insert")).toBe("grantable");
    expect(cellState(r, "update")).toBe("none");
  });
});

describe("setCell", () => {
  it("kanonik sırada ekler ve girdiyi değiştirmez", () => {
    const src = [g("a", ["update"], ["select"])];
    const snap = JSON.stringify(src);
    const out = setCell(src, 0, "delete", "granted", avail);
    expect(out[0].privileges.map((x) => x.privilege)).toEqual(["select", "update", "delete"]);
    expect(JSON.stringify(src)).toBe(snap);
    expect(out).not.toBe(src);
    expect(out[0]).not.toBe(src[0]);
  });
  it("grantable ve geri alma", () => {
    const a = setCell([g("a")], 0, "select", "grantable", avail);
    expect(a[0].privileges).toEqual([{ privilege: "select", grantOption: true }]);
    expect(setCell(a, 0, "select", "granted", avail)[0].privileges[0].grantOption).toBe(false);
  });
  it("boşalan satır kalır", () => {
    const out = setCell([g("a", ["select"])], 0, "select", "none", avail);
    expect(out).toHaveLength(1);
    expect(out[0].privileges).toEqual([]);
  });
  it("bilinmeyen yetki korunur ve sona gider", () => {
    const out = setCell([g("a", ["usage"], ["select"])], 0, "insert", "granted", avail);
    expect(out[0].privileges.map((x) => x.privilege)).toEqual(["select", "insert", "usage"]);
  });
  it("geçersiz satır: değişmemiş kopya", () => {
    const src = [g("a", ["select"])];
    expect(setCell(src, 5, "insert", "granted", avail)).toEqual(src);
  });
});

describe("alıcılar", () => {
  it("sona boş yetkiyle ekler", () => {
    const out = addGrantee([g("a")], "b")!;
    expect(out.map((x) => x.grantee)).toEqual(["a", "b"]);
    expect(out[1].privileges).toEqual([]);
  });
  it("yinelenen (PUBLIC dahil) ve boş ad reddedilir", () => {
    expect(addGrantee([g("a")], "a")).toBeNull();
    expect(addGrantee([g(null)], null)).toBeNull();
    expect(addGrantee([g("a")], null)).not.toBeNull();
    expect(addGrantee([g("a")], "  ")).toBeNull();
    expect(addGrantee([g("a")], " a ")).toBeNull();
  });
  it("removeGrantee", () => {
    expect(removeGrantee([g("a"), g("b")], 0).map((x) => x.grantee)).toEqual(["b"]);
    expect(removeGrantee([g("a")], 3)).toHaveLength(1);
  });
  it("allowGrantOption: PUBLIC ve sahip hayır", () => {
    expect(allowGrantOption(g(null), "o")).toBe(false);
    expect(allowGrantOption(g("o"), "o")).toBe(false);
    expect(allowGrantOption(g("x"), "o")).toBe(true);
  });
});

describe("unknownPrivileges / cellPath", () => {
  it("available dışını verir", () => {
    expect(unknownPrivileges(g("a", ["select"], ["usage"]), avail)).toEqual(["usage"]);
  });
  it("cellPath indeksleri", () => {
    const grants = [g("a", ["select"]), g("b", ["select"], ["update"])];
    expect(cellPath("grants", 1, grants, "update")).toBe("grants[1].privileges[1]");
    expect(cellPath("sections[2].grants", 0, grants, "select")).toBe("sections[2].grants[0].privileges[0]");
    expect(cellPath("grants", 0, grants, "update")).toBeNull();
    expect(cellPath("grants", 9, grants, "select")).toBeNull();
  });
});

describe("nextCellFocus", () => {
  it("kenarlarda sıkışır", () => {
    expect(nextCellFocus(0, 0, 3, 4, "ArrowUp")).toEqual([0, 0]);
    expect(nextCellFocus(2, 3, 3, 4, "ArrowDown")).toEqual([2, 3]);
    expect(nextCellFocus(1, 0, 3, 4, "ArrowLeft")).toEqual([1, 0]);
    expect(nextCellFocus(1, 3, 3, 4, "ArrowRight")).toEqual([1, 3]);
  });
  it("ortada hareket eder; Home/End", () => {
    expect(nextCellFocus(1, 1, 3, 4, "ArrowDown")).toEqual([2, 1]);
    expect(nextCellFocus(1, 1, 3, 4, "ArrowRight")).toEqual([1, 2]);
    expect(nextCellFocus(1, 2, 3, 4, "Home")).toEqual([1, 0]);
    expect(nextCellFocus(1, 2, 3, 4, "End")).toEqual([1, 3]);
  });
  it("boş ızgara", () => {
    expect(nextCellFocus(0, 0, 0, 0, "End")).toEqual([0, 0]);
  });
});

describe("alıcı ekleme girdisi (form)", () => {
  const base = [g(null, ["select"]), g("app", ["insert"])];
  it("boş/boşluk → empty", () => {
    expect(addGranteeInput(base, "")).toEqual({ error: "empty" });
    expect(addGranteeInput(base, "   ")).toEqual({ error: "empty" });
  });
  it("kırpılmış rol adı sona eklenir; girdi değişmez", () => {
    const r = addGranteeInput(base, "  report ");
    expect(r).toEqual({ grants: [...base, { grantee: "report", privileges: [] }] });
    expect(base).toHaveLength(2);
  });
  it("yinelenen alıcı → duplicate (büyük/küçük harf duyarlı rol adları)", () => {
    expect(addGranteeInput(base, "app")).toEqual({ error: "duplicate" });
    expect("grants" in addGranteeInput(base, "App")).toBe(true);
  });
  it("PUBLIC (harf duyarsız) → PUBLIC satırı (grantee null); zaten varsa duplicate", () => {
    const noPublic = [g("app")];
    expect(addGranteeInput(noPublic, "PUBLIC")).toEqual({ grants: [g("app"), { grantee: null, privileges: [] }] });
    expect(addGranteeInput(noPublic, " public ")).toEqual({ grants: [g("app"), { grantee: null, privileges: [] }] });
    expect(addGranteeInput(base, "Public")).toEqual({ error: "duplicate" });
  });
});

describe("planHasRevoke", () => {
  const st = (sql: string) => ({ sql, risk: "safe" as const, transactional: true, note: null, redacted: false });
  const view = (...sql: string[]) => ({ statements: sql.map(st), warnings: [], errors: [], hash: "h", risk: "safe" as const, empty: sql.length === 0 });
  it("REVOKE ile başlayan ifade varsa true", () => {
    expect(planHasRevoke(view("GRANT SELECT ON TABLE t TO a", "REVOKE INSERT ON TABLE t FROM b"))).toBe(true);
    expect(planHasRevoke(view("REVOKE GRANT OPTION FOR SELECT ON TABLE t FROM a"))).toBe(true);
  });
  it("yoksa / görünüm yoksa false", () => {
    expect(planHasRevoke(view("GRANT SELECT ON TABLE t TO a"))).toBe(false);
    expect(planHasRevoke(view())).toBe(false);
    expect(planHasRevoke(null)).toBe(false);
  });
});

describe("PUBLIC adlı gerçek roller (spec E2)", () => {
  const TOKEN = "PUBLIC (herkes)";
  const roles = ["PUBLIC", "Public", "app"];
  it("rol listesinde birebir eşleşen ad gerçek roldür", () => {
    expect(addGranteeInput([], "PUBLIC", { roles, publicToken: TOKEN })).toEqual({ grants: [{ grantee: "PUBLIC", privileges: [] }] });
    expect(addGranteeInput([], " Public ", { roles, publicToken: TOKEN })).toEqual({ grants: [{ grantee: "Public", privileges: [] }] });
  });
  it("eşleşmeyen harf duyarsız `public` ve sözde seçenek PUBLIC satırıdır", () => {
    expect(addGranteeInput([], "public", { roles, publicToken: TOKEN })).toEqual({ grants: [{ grantee: null, privileges: [] }] });
    expect(addGranteeInput([], "PUBLIC", { roles: ["app"], publicToken: TOKEN })).toEqual({ grants: [{ grantee: null, privileges: [] }] });
    expect(addGranteeInput([], TOKEN, { roles, publicToken: TOKEN })).toEqual({ grants: [{ grantee: null, privileges: [] }] });
  });
  it("gerçek PUBLIC rolü ve PUBLIC satırı ayrı alıcıdır (yinelenen sayılmaz)", () => {
    expect(addGranteeInput([g(null)], "PUBLIC", { roles, publicToken: TOKEN })).toEqual({
      grants: [g(null), { grantee: "PUBLIC", privileges: [] }],
    });
    expect(addGranteeInput([g("PUBLIC")], TOKEN, { roles, publicToken: TOKEN })).toEqual({
      grants: [g("PUBLIC"), { grantee: null, privileges: [] }],
    });
    expect(addGranteeInput([g("PUBLIC")], "PUBLIC", { roles, publicToken: TOKEN })).toEqual({ error: "duplicate" });
  });
  it("öneriler: sözde seçenek ayrı değerle; kullanılmış roller ve mevcut PUBLIC satırı düşer; tekrar yok", () => {
    expect(granteeOptions([g("app")], [...roles, "app", "PUBLIC"], TOKEN)).toEqual([TOKEN, "PUBLIC", "Public"]);
    expect(granteeOptions([g(null), g("PUBLIC")], roles, TOKEN)).toEqual(["Public", "app"]);
  });
});

describe("cycleCell (salt okunur ızgara odaklanabilir kalır, döngü etkisiz)", () => {
  it("düzenlenebilir: bir sonraki duruma geçer (PUBLIC/sahipte GRANT OPTION adımı yok)", () => {
    expect(cycleCell([g("app")], 0, "select", "own", avail, false)).toEqual([g("app", ["select"])]);
    expect(cycleCell([g("app", ["select"])], 0, "select", "own", avail, false)).toEqual([g("app", ["select", true])]);
    expect(cycleCell([g(null, ["select"])], 0, "select", "own", avail, false)).toEqual([g(null)]);
    expect(cycleCell([g("own", ["select"])], 0, "select", "own", avail, false)).toEqual([g("own")]);
  });
  it("salt okunur ya da geçersiz satır → null (değişiklik yok)", () => {
    expect(cycleCell([g("app")], 0, "select", "own", avail, true)).toBeNull();
    expect(cycleCell([g("app")], 3, "select", "own", avail, false)).toBeNull();
  });
});

describe("granteeLabel (PUBLIC sözde satırı gerçek \"PUBLIC\" rolünden ayrı)", () => {
  const PSEUDO = "PUBLIC (herkes)";
  it("PUBLIC satırı sözde adla, gerçek roller olduğu gibi", () => {
    expect(granteeLabel(g(null), PSEUDO)).toBe(PSEUDO);
    expect(granteeLabel(g("PUBLIC"), PSEUDO)).toBe("PUBLIC");
    expect(granteeLabel(g("Public"), PSEUDO)).toBe("Public");
    expect(granteeLabel(g(null), PSEUDO)).not.toBe(granteeLabel(g("PUBLIC"), PSEUDO));
  });
});
