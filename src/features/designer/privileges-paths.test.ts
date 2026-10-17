import { describe, expect, it } from "vitest";
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { Privilege } from "../../ipc/bindings/Privilege";
import { groupIssues, issuesFor } from "./designer-model";
import { ACL_GENERAL_PATHS, aclGridPaths } from "./privileges-paths";

const avail: Privilege[] = ["select", "insert", "update"];
const grants: AclGrant[] = [
  { grantee: null, privileges: [{ privilege: "select", grantOption: false }] },
  {
    grantee: "app",
    privileges: [
      { privilege: "select", grantOption: false },
      { privilege: "update", grantOption: true },
      // `available` dışı (ör. eski sürüm/bilinmeyen): ızgara sahiplenmez.
      { privilege: "maintain", grantOption: false },
    ],
  },
];
const err = (path: string): FieldError => ({ path, message: path });

describe("privileges-paths", () => {
  const known = aclGridPaths("grants", grants, avail);

  it("ızgara yolları: satır alıcısı ve kanonik yetki hücreleri", () => {
    expect([...known].sort()).toEqual([
      "grants[0].grantee", "grants[0].privileges[0]",
      "grants[1].grantee", "grants[1].privileges[0]", "grants[1].privileges[1]",
    ]);
  });
  it("hücre/satır hataları ızgaraya, alt yollar (.privilege/.grantOption) dahil", () => {
    const i = groupIssues(
      [err("grants[1].grantee"), err("grants[1].privileges[1].grantOption"), err("grants[0].privileges[0].privilege")],
      known,
    );
    expect(i.general).toEqual([]);
    expect(issuesFor(i, "grants[1].privileges[1]")).toEqual(["grants[1].privileges[1].grantOption"]);
    expect(issuesFor(i, "grants[0].privileges[0]")).toEqual(["grants[0].privileges[0].privilege"]);
  });
  it("ızgaranın sahiplenmediği yollar üst şeride (catch-all)", () => {
    const paths = [
      ...ACL_GENERAL_PATHS, "old.grants[0].grantee", "grants", "grants[1]", "grants[1].privileges", "grants[1].privileges[2]",
      "grants[1].privileges[2].privilege", "grants[5].grantee", "grants[0].privileges[3]", "grants[1].privileges[10]", "x",
    ];
    const i = groupIssues(paths.map(err), known);
    expect(i.generalPaths).toEqual(paths);
    expect(i.byPath.size).toBe(0);
  });
  it("üst şerit yolları spec §8 listesini kapsar", () => {
    for (const p of ["object", "schema", "name", "signature", "foreign", "columns"]) expect(ACL_GENERAL_PATHS).toContain(p);
  });
  it("taban yolu parametredir (varsayılan yetki bölümleri)", () => {
    expect([...aclGridPaths("sections[2].grants", grants.slice(0, 1), avail)].sort()).toEqual([
      "sections[2].grants[0].grantee", "sections[2].grants[0].privileges[0]",
    ]);
  });
});
