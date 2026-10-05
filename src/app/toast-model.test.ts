import { describe, expect, it } from "vitest";
import { MAX_TOASTS, dismissToast, durationFor, pushToast, type Toast } from "./toast-model";

const mk = (id: number, kind: Toast["kind"] = "info"): Toast => ({ id, kind, text: `t${id}` });

describe("toast model", () => {
  it("appends and keeps only the newest MAX_TOASTS", () => {
    let list: Toast[] = [];
    for (let i = 1; i <= MAX_TOASTS + 2; i++) list = pushToast(list, mk(i)).list;
    expect(list.map((t) => t.id)).toEqual([3, 4, 5, 6]);
  });
  it("reports the ids evicted by the cap so their timers can be cleared", () => {
    let list: Toast[] = [];
    for (let i = 1; i <= MAX_TOASTS; i++) list = pushToast(list, mk(i)).list;
    const r = pushToast(list, mk(MAX_TOASTS + 1));
    expect(r.evicted).toEqual([1]);
    expect(r.mergedInto).toBeUndefined();
    expect(pushToast([], mk(1)).evicted).toEqual([]);
  });
  it("merges a toast identical (text + kind) to the newest one", () => {
    const list = [mk(1), mk(2)];
    const r = pushToast(list, { id: 3, kind: "info", text: "t2" });
    expect(r.mergedInto).toBe(2);
    expect(r.list).toBe(list);
    expect(r.evicted).toEqual([]);
  });
  it("does not merge when only the text or only the kind matches, or the match is older", () => {
    const list = [mk(1), mk(2)];
    expect(pushToast(list, { id: 3, kind: "error", text: "t2" }).mergedInto).toBeUndefined();
    expect(pushToast(list, { id: 3, kind: "info", text: "other" }).mergedInto).toBeUndefined();
    expect(pushToast(list, { id: 3, kind: "info", text: "t1" }).list.map((t) => t.id)).toEqual([1, 2, 3]);
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
