import { describe, expect, it } from "vitest";
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import type { Privilege } from "../../ipc/bindings/Privilege";
import {
  addGrantee, allowGrantOption, cellPath, cellState, nextCellFocus, nextCellState, removeGrantee, setCell, unknownPrivileges,
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
