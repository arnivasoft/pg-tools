import { describe, expect, it } from "vitest";
import type { ConfigSetting } from "../../ipc/bindings/ConfigSetting";
import {
  addSetting, isListSetting, listItemCaseWarning, removeSetting, removedSettings, restoreSetting, settingNameWarning, updateSetting,
} from "./settings-model";

const s = (name: string, value = "1"): ConfigSetting => ({ name, value });

describe("settings-model", () => {
  it("addSetting sona boş satır ekler; değişmez", () => {
    const l = [s("a")];
    expect(addSetting(l)).toEqual([s("a"), { name: "", value: "" }]);
    expect(l).toEqual([s("a")]);
  });

  it("updateSetting/removeSetting değişmez; sınır dışı aynı dizi", () => {
    const l = [s("a"), s("b")];
    expect(updateSetting(l, 1, { value: "9" })).toEqual([s("a"), s("b", "9")]);
    expect(updateSetting(l, 5, { value: "9" })).toBe(l);
    expect(updateSetting(l, -1, { value: "9" })).toBe(l);
    expect(removeSetting(l, 0)).toEqual([s("b")]);
    expect(removeSetting(l, 2)).toBe(l);
    expect(l).toHaveLength(2);
  });

  it("removedSettings: taslakta olmayan yüklü ayarlar, özgün sırayla", () => {
    const o = [s("a"), s("b"), s("c")];
    expect(removedSettings(o, [s("b")])).toEqual([s("a"), s("c")]);
    expect(removedSettings(null, [s("b")])).toEqual([]);
    expect(removedSettings(o, o)).toEqual([]);
  });

  it("restoreSetting özgün sıraya ve özgün değerle geri koyar", () => {
    const o = [s("a", "1"), s("b", "2"), s("c", "3")];
    expect(restoreSetting([s("a", "1"), s("c", "3")], o, "b")).toEqual(o);
    expect(restoreSetting([s("c", "3")], o, "a")).toEqual([s("a", "1"), s("c", "3")]);
    expect(restoreSetting([s("a", "1"), s("c", "3"), s("x")], o, "b")).toEqual([...o, s("x")]);
    const l = [s("a", "1")];
    expect(restoreSetting(l, o, "yok")).toBe(l);
    expect(restoreSetting(o, o, "b")).toBe(o);
  });

  it("settingNameWarning: boş, büyük harf, geçersiz karakter, yinelenen", () => {
    expect(settingNameWarning([s("")], 0)).toBeTruthy();
    expect(settingNameWarning([s("Work_Mem")], 0)).toBeTruthy();
    expect(settingNameWarning([s("a-b")], 0)).toBeTruthy();
    expect(settingNameWarning([s("work_mem")], 0)).toBeNull();
    expect(settingNameWarning([s("work_mem"), s("work_mem")], 1)).toBeTruthy();
    expect(settingNameWarning([s("App.Mode")], 0)).toBeNull();
    expect(settingNameWarning([s("App.Mode"), s("app.mode")], 1)).toBeTruthy();
    expect(settingNameWarning([s("a"), s("b")], 1)).toBeNull();
    expect(settingNameWarning([s("a")], 3)).toBeNull();
  });

  it("isListSetting", () => {
    expect(isListSetting("search_path")).toBe(true);
    expect(isListSetting("temp_tablespaces")).toBe(true);
    expect(isListSetting("work_mem")).toBe(false);
  });

  it("listItemCaseWarning: tırnaksız büyük harfli öğe", () => {
    expect(listItemCaseWarning("search_path", '"$user", MySchema')).toBeTruthy();
    expect(listItemCaseWarning("search_path", '"MySchema"')).toBeNull();
    expect(listItemCaseWarning("search_path", '"$user", public')).toBeNull();
    expect(listItemCaseWarning("work_mem", "MySchema")).toBeNull();
  });
});
