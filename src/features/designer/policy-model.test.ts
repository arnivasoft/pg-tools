import { describe, expect, it } from "vitest";
import type { ObjectDef } from "../../ipc/bindings/ObjectDef";
import type { PolicyDef } from "../../ipc/bindings/PolicyDef";
import {
  addRole, asPolicy, expressionAvailability, policyCommandPatch, policyPatch, policyRebuildReasons, removeRole, roleChoices,
} from "./policy-model";

const pol = (p: Partial<PolicyDef> = {}): PolicyDef => ({
  schema: "app", table: "t", name: "p", command: "all", permissive: true, roles: [], using: "true", withCheck: "true",
  comment: null, editable: true, unsupported: [], tableRls: null, ...p,
});

describe("expressionAvailability", () => {
  it("beş komut tablosu", () => {
    expect(expressionAvailability("all")).toEqual({ using: true, withCheck: true });
    expect(expressionAvailability("update")).toEqual({ using: true, withCheck: true });
    expect(expressionAvailability("select")).toEqual({ using: true, withCheck: false });
    expect(expressionAvailability("delete")).toEqual({ using: true, withCheck: false });
    expect(expressionAvailability("insert")).toEqual({ using: false, withCheck: true });
  });
});

describe("policyCommandPatch", () => {
  it("insert: using silinir, withCheck korunur", () => {
    const n = policyCommandPatch(pol(), "insert");
    expect(n.command).toBe("insert");
    expect(n.using).toBeNull();
    expect(n.withCheck).toBe("true");
  });
  it("select/delete: withCheck silinir, using korunur", () => {
    for (const c of ["select", "delete"] as const) {
      const n = policyCommandPatch(pol(), c);
      expect(n.withCheck).toBeNull();
      expect(n.using).toBe("true");
    }
  });
  it("all/update hiçbirini silmez; girdi değişmez", () => {
    const d = pol();
    for (const c of ["all", "update"] as const) {
      const n = policyCommandPatch(d, c);
      expect(n.using).toBe("true");
      expect(n.withCheck).toBe("true");
      expect(n).not.toBe(d);
    }
    expect(d).toEqual(pol());
  });
  it("sözleşme alanları korunur", () => {
    const n = policyCommandPatch(pol({ tableRls: { enabled: true, forced: false } }), "insert");
    expect(n.tableRls).toEqual({ enabled: true, forced: false });
    expect(n.editable).toBe(true);
    expect(n.unsupported).toEqual([]);
  });
});

describe("policyRebuildReasons", () => {
  it("create → []", () => expect(policyRebuildReasons(null, pol())).toEqual([]));
  it("aynı → []", () => expect(policyRebuildReasons(pol(), pol())).toEqual([]));
  it("komut", () => expect(policyRebuildReasons(pol(), pol({ command: "select" }))).toEqual(["command"]));
  it("permissive", () => expect(policyRebuildReasons(pol(), pol({ permissive: false }))).toEqual(["permissive"]));
  it("using dolu → null", () => expect(policyRebuildReasons(pol(), pol({ using: null }))).toEqual(["usingRemoved"]));
  it("withCheck dolu → null", () => expect(policyRebuildReasons(pol(), pol({ withCheck: null }))).toEqual(["withCheckRemoved"]));
  it("boş metin null sayılır", () => {
    expect(policyRebuildReasons(pol(), pol({ using: "  " }))).toEqual(["usingRemoved"]);
    expect(policyRebuildReasons(pol({ using: null }), pol({ using: "" }))).toEqual([]);
  });
  it("null → dolu yeniden kurma değil", () => {
    expect(policyRebuildReasons(pol({ using: null, withCheck: null }), pol())).toEqual([]);
  });
  it("ad/rol/yorum değişimi yeniden kurma değil", () => {
    expect(policyRebuildReasons(pol(), pol({ name: "q", roles: ["a"], comment: "c" }))).toEqual([]);
  });
  it("birden çok neden", () => {
    expect(policyRebuildReasons(pol(), pol({ command: "insert", permissive: false, using: null, withCheck: null }))).toEqual([
      "command", "permissive", "usingRemoved", "withCheckRemoved",
    ]);
  });
});

describe("rol yardımcıları", () => {
  it("addRole: sona ekler; yinelenen, boş ve yalnız boşluk yok sayılır", () => {
    const d = pol({ roles: ["a"] });
    expect(addRole(d, "b").roles).toEqual(["a", "b"]);
    expect(addRole(d, "a")).toBe(d);
    expect(addRole(d, "")).toBe(d);
    expect(addRole(d, "  ")).toBe(d);
    expect(d.roles).toEqual(["a"]);
  });
  it("removeRole: indeksi siler; indeks dışı no-op", () => {
    const d = pol({ roles: ["a", "b", "c"] });
    expect(removeRole(d, 1).roles).toEqual(["a", "c"]);
    expect(removeRole(d, 3)).toBe(d);
    expect(removeRole(d, -1)).toBe(d);
    expect(d.roles).toEqual(["a", "b", "c"]);
  });
  it("roleChoices: mevcutlar ve public çıkar, sıralı", () => {
    expect(roleChoices(["zed", "public", "bob", "amy"], ["bob"])).toEqual(["amy", "zed"]);
  });
});

describe("policyPatch / asPolicy", () => {
  it("alanı günceller, kind korunur", () => {
    const d: ObjectDef = { kind: "policy", ...pol() };
    const n = policyPatch(d, { name: "x" });
    expect(n).toEqual({ kind: "policy", ...pol({ name: "x" }) });
    expect(n).not.toBe(d);
  });
  it("tür uyuşmazlığında aynı nesne", () => {
    const s: ObjectDef = { kind: "schema", name: "a", owner: null, comment: null };
    expect(policyPatch(s, { name: "x" })).toBe(s);
  });
  it("asPolicy başka türde null", () => {
    expect(asPolicy({ kind: "schema", name: "a", owner: null, comment: null })).toBeNull();
    expect(asPolicy(null)).toBeNull();
    expect(asPolicy({ kind: "policy", ...pol() })?.name).toBe("p");
  });
});
