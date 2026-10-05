import { describe, expect, it } from "vitest";
import { MAX_TOASTS, dismissToast, durationFor, pushToast, type Toast } from "./toast-model";

const mk = (id: number, kind: Toast["kind"] = "info"): Toast => ({ id, kind, text: `t${id}` });

describe("toast model", () => {
  it("appends and keeps only the newest MAX_TOASTS", () => {
    let list: Toast[] = [];
    for (let i = 1; i <= MAX_TOASTS + 2; i++) list = pushToast(list, mk(i));
    expect(list.map((t) => t.id)).toEqual([3, 4, 5, 6]);
  });
  it("dismisses by id and ignores unknown ids", () => {
    const list = [mk(1), mk(2)];
    expect(dismissToast(list, 1).map((t) => t.id)).toEqual([2]);
    expect(dismissToast(list, 9)).toEqual(list);
  });
  it("errors stay longer than info/success", () => {
    expect(durationFor("error")).toBeGreaterThan(durationFor("info"));
    expect(durationFor("success")).toBe(durationFor("info"));
  });
});
