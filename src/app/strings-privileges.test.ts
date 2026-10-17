import { describe, expect, it } from "vitest";
import { tpriv } from "./strings-privileges";

describe("strings-privileges: erişilebilir adlar", () => {
  it("hücre adı: <alıcı> — <yetki>: <durum>", () => {
    expect(tpriv.grid.cell("app", "SELECT", "var")).toBe("app — SELECT: var");
  });
  it("ızgara adı: <bölüm> — <ızgara>", () => {
    expect(tpriv.gridName("Tablolar", tpriv.grid.grid)).toBe("Tablolar — Yetki ızgarası");
  });
  it("PUBLIC sözde satırı \"PUBLIC\" adlı gerçek rolden ayrı adla gösterilir (ızgara, salt okunur listeler)", () => {
    expect(tpriv.grid.publicName).not.toBe("PUBLIC");
    expect(tpriv.readOnly.publicName).toBe(tpriv.grid.publicName);
    expect(tpriv.grid.publicName).toBe(tpriv.add.publicOption);
    expect(tpriv.grid.cell(tpriv.grid.publicName, "SELECT", "var")).not.toBe(tpriv.grid.cell("PUBLIC", "SELECT", "var"));
  });
  it("kaybolan rol bilgisi rol adını içerir", () => {
    expect(tpriv.defaults.roleGone("eski")).toContain("eski");
  });
});
