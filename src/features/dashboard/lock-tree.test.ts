import { describe, expect, it } from "vitest";
import type { BlockSession } from "../../ipc/bindings/BlockSession";
import { buildLockTree, countWaiters, type LockNode } from "./lock-tree";

const s = (pid: number, blockedBy: number[] = []): BlockSession => ({
  pid, user: "u", application: "app", state: "active", query: null, blockedBy, waitSecs: null,
});
const pids = (n: LockNode[]) => n.map((x) => x.pid);

describe("buildLockTree", () => {
  it("boş girdi boş orman", () => {
    expect(buildLockTree([])).toEqual([]);
  });
  it("zincir: 3 → 2 → 1 (1 kök)", () => {
    const t = buildLockTree([s(1), s(2, [1]), s(3, [2])]);
    expect(pids(t)).toEqual([1]);
    expect(pids(t[0].children)).toEqual([2]);
    expect(pids(t[0].children[0].children)).toEqual([3]);
    expect(t[0].kind).toBe("session");
    expect(t[0].session?.pid).toBe(1);
    expect(t[0].cycle).toBe(false);
  });
  it("aynı engelleyiciyi bekleyen çoğu gruplanır; countWaiters tekrarsız", () => {
    const t = buildLockTree([s(1), s(5, [1]), s(3, [1]), s(4, [1]), s(6, [3])]);
    expect(pids(t)).toEqual([1]);
    expect(pids(t[0].children)).toEqual([3, 4, 5]);
    expect(countWaiters(t[0])).toBe(4);
    expect(countWaiters(t[0].children[2])).toBe(0);
  });
  it("birden çok kök pid sırasıyla", () => {
    const t = buildLockTree([s(9), s(2), s(4, [9]), s(5, [2])]);
    expect(pids(t)).toEqual([2, 9]);
  });
  it("kimseyi engellemeyen, beklemeyen oturum kök değil", () => {
    expect(buildLockTree([s(1), s(2)])).toEqual([]);
  });
  it("döngü (1↔2) sonsuz döngüye girmez; cycle işaretlenir; en küçük pid kök", () => {
    const t = buildLockTree([s(2, [1]), s(1, [2])]);
    expect(pids(t)).toEqual([1]);
    expect(t[0].cycle).toBe(true);
    expect(pids(t[0].children)).toEqual([2]);
    expect(t[0].children[0].children).toEqual([]);
    expect(t[0].children[0].cycle).toBe(true);
    expect(countWaiters(t[0])).toBe(1);
  });
  it("kendini bekleyen oturum da sonlanır", () => {
    const t = buildLockTree([s(7, [7])]);
    expect(pids(t)).toEqual([7]);
    expect(t[0].cycle).toBe(true);
    expect(t[0].children).toEqual([]);
  });
  it("döngüye bağlı kuyruk: döngü kökü seçilir, kuyruk altında kalır", () => {
    const t = buildLockTree([s(1, [2]), s(2, [1]), s(3, [1]), s(4, [3])]);
    expect(pids(t)).toEqual([1]);
    expect(t[0].cycle).toBe(true);
    expect(countWaiters(t[0])).toBe(3);
  });
  it("kuyruk pid'i döngü pid'inden küçük olsa da kök döngünün içinden seçilir", () => {
    // 1 yalnızca bekleyen (kimseyi engellemiyor); 5↔6 döngüsü
    const t = buildLockTree([s(1, [5]), s(5, [6]), s(6, [5])]);
    expect(pids(t)).toEqual([5]);
    expect(countWaiters(t[0])).toBe(2);
  });
  it("blockedBy içinde 0 → 'prepared' düğümü, çocuğu bekleyen", () => {
    const t = buildLockTree([s(8, [0]), s(9, [0]), s(10, [8])]);
    expect(pids(t)).toEqual([0]);
    expect(t[0].kind).toBe("prepared");
    expect(t[0].session).toBeNull();
    expect(pids(t[0].children)).toEqual([8, 9]);
    expect(pids(t[0].children[0].children)).toEqual([10]);
    expect(countWaiters(t[0])).toBe(3);
  });
  it("listede olmayan engelleyici session:null kök olur", () => {
    const t = buildLockTree([s(2, [77])]);
    expect(pids(t)).toEqual([77]);
    expect(t[0].session).toBeNull();
    expect(t[0].kind).toBe("session");
    expect(pids(t[0].children)).toEqual([2]);
  });
  it("iki engelleyiciyi bekleyen oturum iki ağaçta görünür; countWaiters tekrar saymaz", () => {
    const t = buildLockTree([s(1), s(2), s(3, [1, 2]), s(4, [3])]);
    expect(pids(t)).toEqual([1, 2]);
    expect(pids(t[0].children)).toEqual([3]);
    expect(pids(t[1].children)).toEqual([3]);
    expect(countWaiters(t[0])).toBe(2);
    // aynı alt ağaç iki kez geçse de tekrarsız sayılır (elmas)
    const d = buildLockTree([s(1), s(2, [1]), s(3, [1]), s(4, [2, 3])]);
    expect(countWaiters(d[0])).toBe(3);
  });
});
