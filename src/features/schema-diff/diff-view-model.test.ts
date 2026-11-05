import { describe, expect, it } from "vitest";
import type { DiffItem } from "../../ipc/bindings/DiffItem";
import { LINE_HEIGHT, ROW_HEIGHT, bulkBox, lineClass, noteBadge, pickerLists, rowAria } from "./diff-view-model";

function item(o: Partial<DiffItem> = {}): DiffItem {
  return {
    key: "k", kind: "table", targetKind: null, name: "t", table: null, signature: null, status: "different",
    problem: null, risk: "data_loss", notes: [], errors: [], selectable: true, defaultSelected: true, ...o,
  };
}

describe("sabitler", () => {
  it("satır yükseklikleri", () => {
    expect(ROW_HEIGHT).toBe(26);
    expect(LINE_HEIGHT).toBe(18);
  });
});

describe("rowAria", () => {
  it("seçili öğe: tür, ad, durum, risk, seçili", () => {
    expect(rowAria(item(), true, false)).toEqual({ label: "Tablo t, Farklı, Veri kaybı olabilir, seçili", selected: false });
  });
  it("seçili değil ve odaklı: `selected` odağı taşır, metinde seçili yok", () => {
    expect(rowAria(item({ risk: "safe" }), false, true)).toEqual({ label: "Tablo t, Farklı, Güvenli", selected: true });
  });
  it("seçilemez öğe ek alır; sorunlu öğede durum sorun metnidir", () => {
    const r = rowAria(item({ problem: "manual", selectable: false, risk: "destructive" }), false, false);
    expect(r.label).toBe("Tablo t, Elle gerekiyor, Yıkıcı, betiğe eklenemez");
  });
  it("tablo üyesi ve imzalı ad", () => {
    expect(rowAria(item({ kind: "index", table: "t", name: "ix" }), false, false).label.startsWith("İndeks t.ix, ")).toBe(true);
    expect(rowAria(item({ kind: "function", name: "f", signature: "integer" }), false, false).label.startsWith("Fonksiyon f(integer), ")).toBe(
      true,
    );
  });
});

describe("lineClass", () => {
  it("same boş; added yeşil; removed kırmızı zemin", () => {
    expect(lineClass("same")).toBe("");
    expect(lineClass("added")).toBe("bg-[color-mix(in_srgb,var(--srv-green)_14%,transparent)]");
    expect(lineClass("removed")).toBe("bg-[color-mix(in_srgb,var(--danger)_14%,transparent)]");
  });
});

describe("noteBadge", () => {
  it("hata varsa error; sayı hata sayısı; başlık önce hatalar, sonra notlar", () => {
    expect(noteBadge(item({ errors: ["e1", "e2"], notes: ["n1"] }))).toEqual({ count: 2, tone: "error", title: "e1\ne2\nn1" });
  });
  it("yalnız not → info", () => {
    expect(noteBadge(item({ notes: ["n1", "n2"] }))).toEqual({ count: 2, tone: "info", title: "n1\nn2" });
  });
  it("ikisi de yok → null", () => {
    expect(noteBadge(item())).toBeNull();
  });
});

describe("bulkBox", () => {
  const a = item({ key: "a" });
  const b = item({ key: "b" });
  const x = item({ key: "x", selectable: false });
  it("seçilebilir öğe yoksa devre dışı ve işaretsiz", () => {
    expect(bulkBox(new Set(), [x])).toEqual({ checked: false, indeterminate: false, disabled: true, next: true });
    expect(bulkBox(new Set(["x"]), [])).toEqual({ checked: false, indeterminate: false, disabled: true, next: true });
  });
  it("hiçbiri / bazısı / tümü", () => {
    expect(bulkBox(new Set(), [a, b, x])).toEqual({ checked: false, indeterminate: false, disabled: false, next: true });
    expect(bulkBox(new Set(["a"]), [a, b, x])).toEqual({ checked: false, indeterminate: true, disabled: false, next: true });
    expect(bulkBox(new Set(["a", "b"]), [a, b, x])).toEqual({ checked: true, indeterminate: false, disabled: false, next: false });
  });
});

describe("pickerLists", () => {
  const base = {
    serverId: "s", databaseOid: null, schema: null, databases: null, databasesError: null, schemas: null,
    schemasError: null, loadingLists: false,
  };
  it("sunucu yok: iki liste de gerekçeli devre dışı, yükleniyor yok", () => {
    const r = pickerLists({ ...base, serverId: null });
    expect(r.databases).toEqual({ loading: false, reason: "Önce sunucu seçin." });
    expect(r.schemas).toEqual({ loading: false, reason: "Önce veritabanı seçin." });
  });
  it("veritabanları yüklenirken yalnız veritabanı seçicisi yükleniyor", () => {
    const r = pickerLists({ ...base, loadingLists: true });
    expect(r.databases).toEqual({ loading: true, reason: null });
    expect(r.schemas).toEqual({ loading: false, reason: "Önce veritabanı seçin." });
  });
  it("şemalar yüklenirken yalnız şema seçicisi yükleniyor; liste gelince gerekçe yok", () => {
    const loading = pickerLists({ ...base, databases: [{ oid: 1, name: "d" }], databaseOid: 1, loadingLists: true });
    expect(loading.databases).toEqual({ loading: false, reason: null });
    expect(loading.schemas).toEqual({ loading: true, reason: null });
    const done = pickerLists({ ...base, databases: [{ oid: 1, name: "d" }], databaseOid: 1, schemas: ["public"] });
    expect(done.schemas).toEqual({ loading: false, reason: null });
  });
  it("sunucu seçili ama liste yok ve yüklenmiyor (hata): sunucu gerekçesi değil, boş gerekçe", () => {
    const r = pickerLists({ ...base, databasesError: "e" });
    expect(r.databases).toEqual({ loading: false, reason: null });
  });
});
