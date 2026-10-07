import { describe, expect, it } from "vitest";
import { tdata } from "./strings-data";

describe("strings-data", () => {
  it("parametreli metinler", () => {
    expect(tdata.tab.title("musteri")).toContain("musteri");
    expect(tdata.status.rows(1000, true)).toContain("1000");
    expect(tdata.status.rows(1000, true)).not.toBe(tdata.status.rows(1000, false));
    expect(tdata.status.changes(3)).toContain("3");
    expect(tdata.status.applied(2)).toContain("2");
    expect(tdata.preview.truncated(120)).toBe("…(+120)");
  });

  it("boyut denetiminde aranan sabit metinler", () => {
    expect(tdata.panel.title).toBe("Değişiklik paneli");
    expect(tdata.toolbar.deleteRow).toBe("Satırı sil");
    expect(tdata.dirty.reload).toBe("Yeniden yükle (düzenlemeler atılır)");
    expect(tdata.open.none).toBe("Bu nesne için veri açılamıyor");
  });

  it("salt okunur gerekçeleri ve risk etiketleri eksiksiz", () => {
    for (const k of ["no_key", "no_privilege", "system_object", "unsupported_kind"] as const) expect(tdata.ro.reason[k]).toBeTruthy();
    for (const k of ["safe", "destructive", "data_loss"] as const) expect(tdata.risk[k]).toBeTruthy();
  });
});
