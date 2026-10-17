import { describe, expect, it } from "vitest";
import { canMove, insertRow, moveRow, removeRow } from "./row-list-model";

describe("row-list-model", () => {
  it("moveRow: yukarı/aşağı komşuyla yer değiştirir; yeni dizi döner, girdi değişmez", () => {
    const list = Object.freeze(["a", "b", "c"]);
    expect(moveRow(list, 1, -1)).toEqual(["b", "a", "c"]);
    expect(moveRow(list, 1, 1)).toEqual(["a", "c", "b"]);
    expect(list).toEqual(["a", "b", "c"]);
    expect(moveRow(list, 0, 1)).not.toBe(list);
  });

  it("moveRow: sınır dışı taşıma ve geçersiz indeks değişiklik yapmaz (kopya döner)", () => {
    const list = ["a", "b", "c"];
    expect(moveRow(list, 0, -1)).toEqual(list);
    expect(moveRow(list, 2, 1)).toEqual(list);
    expect(moveRow(list, -1, 1)).toEqual(list);
    expect(moveRow(list, 3, -1)).toEqual(list);
    expect(moveRow(list, 0, -1)).not.toBe(list);
  });

  it("canMove: sınırlar", () => {
    expect(canMove(0, -1, 3)).toBe(false);
    expect(canMove(0, 1, 3)).toBe(true);
    expect(canMove(2, 1, 3)).toBe(false);
    expect(canMove(2, -1, 3)).toBe(true);
    expect(canMove(5, -1, 3)).toBe(false);
    expect(canMove(0, 1, 1)).toBe(false);
  });

  it("canMove: kilitli satır taşınamaz, kilitli satırla yer de değiştirilemez", () => {
    const locked = (i: number) => i === 1;
    expect(canMove(1, -1, 3, locked)).toBe(false);
    expect(canMove(1, 1, 3, locked)).toBe(false);
    expect(canMove(0, 1, 3, locked)).toBe(false);
    expect(canMove(2, -1, 3, locked)).toBe(false);
    expect(canMove(2, 1, 4, locked)).toBe(true);
  });

  it("moveRow kilit varsa canMove'a uyar", () => {
    const locked = (i: number) => i === 0;
    expect(moveRow(["a", "b"], 1, -1, locked)).toEqual(["a", "b"]);
    expect(moveRow(["a", "b", "c"], 1, 1, locked)).toEqual(["a", "c", "b"]);
  });

  it("removeRow: indeksteki satırı çıkarır; geçersiz indekste kopya döner; girdi değişmez", () => {
    const list = Object.freeze([1, 2, 3]);
    expect(removeRow(list, 1)).toEqual([1, 3]);
    expect(removeRow(list, 5)).toEqual([1, 2, 3]);
    expect(removeRow(list, -1)).toEqual([1, 2, 3]);
    expect(list).toEqual([1, 2, 3]);
  });

  it("insertRow: indekse ekler, sınır dışı indeks sıkıştırılır (başa/sona); girdi değişmez", () => {
    const list = Object.freeze(["a", "b"]);
    expect(insertRow(list, 1, "x")).toEqual(["a", "x", "b"]);
    expect(insertRow(list, 0, "x")).toEqual(["x", "a", "b"]);
    expect(insertRow(list, 2, "x")).toEqual(["a", "b", "x"]);
    expect(insertRow(list, 99, "x")).toEqual(["a", "b", "x"]);
    expect(insertRow(list, -4, "x")).toEqual(["x", "a", "b"]);
    expect(list).toEqual(["a", "b"]);
  });
});
