import { describe, expect, it } from "vitest";
import type { DefaultAclDef } from "../../ipc/bindings/DefaultAclDef";
import { defaultAclSections, roleSwitchDecision } from "./default-acl-model";

const def = (p: Partial<DefaultAclDef> = {}): DefaultAclDef => ({
  role: "alice", schema: null, viewer: "alice", editable: true, unsupported: [],
  sections: [
    { objectType: "tables", grants: [{ grantee: "app", privileges: [] }], baseline: [], available: ["select"] },
    { objectType: "functions", grants: [], baseline: [{ grantee: null, privileges: [{ privilege: "execute", grantOption: false }] }], available: ["execute"] },
  ],
  ...p,
});

describe("roleSwitchDecision", () => {
  it("aynı rol ya da meşgul sekme → yok say", () => {
    expect(roleSwitchDecision({ current: "a", next: "a", dirty: true, busy: false })).toBe("ignore");
    expect(roleSwitchDecision({ current: "a", next: "b", dirty: false, busy: true })).toBe("ignore");
    expect(roleSwitchDecision({ current: "a", next: "  ", dirty: false, busy: false })).toBe("ignore");
  });
  it("kirli taslakta onay, temizde doğrudan geçiş", () => {
    expect(roleSwitchDecision({ current: "a", next: "b", dirty: true, busy: false })).toBe("confirm");
    expect(roleSwitchDecision({ current: "a", next: "b", dirty: false, busy: false })).toBe("switch");
  });
});

describe("defaultAclSections", () => {
  it("genel kapsam: baseline ızgarası yok; taban yolu `sections[i].grants`, sahip = rol", () => {
    const s = defaultAclSections(def(), false);
    expect(s.map((x) => [x.objectType, x.basePath, x.owner, x.readOnly, x.baseline])).toEqual([
      ["tables", "sections[0].grants", "alice", false, null],
      ["functions", "sections[1].grants", "alice", false, null],
    ]);
  });
  it("şema kapsamı: her bölümün altında baseline her zaman salt okunur", () => {
    const s = defaultAclSections(def({ schema: "app" }), false);
    expect(s[0]!.baseline).toEqual({ grants: [], readOnly: true, basePath: "sections[0].baseline" });
    expect(s[1]!.baseline!.readOnly).toBe(true);
    expect(s[1]!.baseline!.grants).toEqual(def().sections[1]!.baseline);
    expect(s[0]!.readOnly).toBe(false);
  });
  it("salt okunur tanım ya da form → bölüm ızgaraları salt okunur", () => {
    expect(defaultAclSections(def({ editable: false }), false).every((x) => x.readOnly)).toBe(true);
    expect(defaultAclSections(def(), true).every((x) => x.readOnly)).toBe(true);
  });
  it("düzenleme sonrası her indeks aynı bölümü taşır (`<Index>` bölüm bileşenlerini korur; yalnız `grants` değişir)", () => {
    const before = defaultAclSections(def(), false);
    const edited = def();
    edited.sections[1]!.grants = [{ grantee: "bob", privileges: [{ privilege: "execute", grantOption: false }] }];
    const after = defaultAclSections(edited, false);
    const shape = (xs: typeof before) => xs.map((x) => [x.objectType, x.basePath, x.owner, x.readOnly, x.baseline?.basePath ?? null]);
    expect(shape(after)).toEqual(shape(before));
    expect(after[1]!.grants).toEqual(edited.sections[1]!.grants);
    expect(after[0]!.grants).toEqual(before[0]!.grants);
  });
});
