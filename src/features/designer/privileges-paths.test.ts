import { describe, expect, it } from "vitest";
import type { AclGrant } from "../../ipc/bindings/AclGrant";
import type { FieldError } from "../../ipc/bindings/FieldError";
import type { Privilege } from "../../ipc/bindings/Privilege";
import { groupIssues, issuesFor } from "./designer-model";
import type { DefaultAclSection } from "../../ipc/bindings/DefaultAclSection";
import { ACL_GENERAL_PATHS, aclGridPaths, defaultAclGridPaths } from "./privileges-paths";

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

describe("privileges-paths: yinelenen yetki", () => {
  it("satırdaki aynı yetkinin yalnız ilk konumu hücreye; ikincisi üst şeride", () => {
    const dup: AclGrant[] = [{
      grantee: "app",
      privileges: [{ privilege: "select", grantOption: false }, { privilege: "select", grantOption: true }],
    }];
    const known = aclGridPaths("grants", dup, avail);
    expect([...known].sort()).toEqual(["grants[0].grantee", "grants[0].privileges[0]"]);
    const i = groupIssues([err("grants[0].privileges[1]")], known);
    expect(i.generalPaths).toEqual(["grants[0].privileges[1]"]);
  });
});

describe("privileges-paths: varsayılan yetki bölümleri", () => {
  const sec = (grants: AclGrant[], available: Privilege[], baseline: AclGrant[] = []): DefaultAclSection =>
    ({ objectType: "tables", grants, baseline, available });
  const sections = [
    sec([{ grantee: "app", privileges: [{ privilege: "select", grantOption: false }] }], ["select", "insert"]),
    sec(
      [{ grantee: null, privileges: [{ privilege: "insert", grantOption: false }, { privilege: "maintain", grantOption: false }] }],
      ["select", "insert"],
      [{ grantee: "app", privileges: [{ privilege: "select", grantOption: false }] }],
    ),
  ];
  const known = defaultAclGridPaths(sections);
  it("her bölümün ızgarası kendi `sections[i].grants` yollarını sahiplenir; baseline sahiplenilmez", () => {
    expect([...known].sort()).toEqual([
      "sections[0].grants[0].grantee", "sections[0].grants[0].privileges[0]",
      "sections[1].grants[0].grantee", "sections[1].grants[0].privileges[0]",
    ]);
  });
  it("bölüm hücre hataları doğru bölüme/hücreye; diğerleri üst şeride", () => {
    const i = groupIssues(
      [
        err("sections[1].grants[0].privileges[0].grantOption"), err("sections[0].grants[0].grantee"),
        err("sections"), err("sections[1].baseline"), err("sections[1].grants[0].privileges[1]"), err("old.sections[0].grants[0].grantee"),
      ],
      known,
    );
    expect(issuesFor(i, "sections[1].grants[0].privileges[0]")).toEqual(["sections[1].grants[0].privileges[0].grantOption"]);
    expect(issuesFor(i, "sections[0].grants[0].privileges[0]")).toEqual([]);
    expect(issuesFor(i, "sections[0].grants[0].grantee")).toEqual(["sections[0].grants[0].grantee"]);
    expect(i.generalPaths).toEqual(["sections", "sections[1].baseline", "sections[1].grants[0].privileges[1]", "old.sections[0].grants[0].grantee"]);
  });
});
