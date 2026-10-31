import { describe, expect, it, vi } from "vitest";
import { makePicker } from "./picker";

function fake(openResult: string | string[] | null = null, saveResult: string | null = null) {
  const save = vi.fn(async (_o: unknown) => saveResult);
  const open = vi.fn(async (_o: unknown) => openResult);
  return { save, open, picker: makePicker({ save, open }) };
}

describe("makePicker", () => {
  it("save bağımsız değişkenleri olduğu gibi geçer", async () => {
    const f = fake(null, "C:\Yedek\a.backup");
    const o = { defaultPath: "a.backup", filters: [{ name: "x", extensions: ["backup"] }] };
    expect(await f.picker.save(o)).toBe("C:\Yedek\a.backup");
    expect(f.save).toHaveBeenCalledWith(o);
  });

  it("save iptali null döner", async () => {
    const f = fake(null, null);
    expect(await f.picker.save({ defaultPath: "a", filters: [] })).toBeNull();
  });

  it("openFile dosya süzgeçleriyle tek seçim ister", async () => {
    const f = fake("C:\a.dump");
    expect(await f.picker.openFile()).toBe("C:\a.dump");
    expect(f.open).toHaveBeenCalledWith({
      multiple: false,
      directory: false,
      filters: [
        { name: "PostgreSQL yedeği", extensions: ["backup", "dump", "sql", "tar", "gz"] },
        { name: "Tüm dosyalar", extensions: ["*"] },
      ],
    });
  });

  it("openDir klasör seçtirir", async () => {
    const f = fake("C:\klasor");
    expect(await f.picker.openDir()).toBe("C:\klasor");
    expect(f.open).toHaveBeenCalledWith({ multiple: false, directory: true });
  });

  it("dizi dönüşü ilk öğeye, boş dizi ve null null'a iner", async () => {
    expect(await fake(["a", "b"]).picker.openFile()).toBe("a");
    expect(await fake([]).picker.openDir()).toBeNull();
    expect(await fake(null).picker.openFile()).toBeNull();
  });

  it("eklenti hatası fırlatma olarak geçer", async () => {
    const open = vi.fn(async () => {
      throw new Error("boom");
    });
    const picker = makePicker({ save: async () => null, open });
    await expect(picker.openFile()).rejects.toThrow("boom");
    await expect(picker.openDir()).rejects.toThrow("boom");
  });
});
