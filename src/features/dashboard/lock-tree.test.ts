import { describe, expect, it } from "vitest";
import type { BlockSession } from "../../ipc/bindings/BlockSession";
import type { LockRow } from "../../ipc/bindings/LockRow";
import { buildLockTree as buildForest, countWaiters, pendingLocksByPid, type LockNode } from "./lock-tree";

const s = (pid: number, blockedBy: number[] = []): BlockSession => ({
  pid, user: "u", application: "app", state: "active", query: null, blockedBy, waitSecs: null,
});
const pids = (n: LockNode[]) => n.map((x) => x.pid);
const buildLockTree = (x: BlockSession[]) => buildForest(x).roots;
const size = (n: LockNode): number => {
  let k = 0;
  const st = [n];
  while (st.length) {
    const c = st.pop()!;
    k++;
    st.push(...c.children);
  }
  return k;
};

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
  it("kuyruk biçimi (yumuşak engelleyiciler) üstel büyümez: N=200 hızlı, ağaç başına ≤ N düğüm, bekleyenler tutanın altında düz", () => {
    const N = 200;
    const q = [s(1)];
    // k. bekleyen: tutan (1) + öndeki tüm bekleyenler
    for (let k = 2; k <= N; k++) q.push(s(k, Array.from({ length: k - 1 }, (_, i) => i + 1)));
    const t0 = performance.now();
    const f = buildForest(q);
    const ms = performance.now() - t0;
    expect(ms).toBeLessThan(500);
    expect(f.truncated).toBe(false);
    expect(pids(f.roots)).toEqual([1]);
    expect(size(f.roots[0])).toBeLessThanOrEqual(N);
    expect(f.roots[0].children).toHaveLength(N - 1);
    expect(f.roots[0].children.every((c) => c.children.length === 0 && !c.cycle)).toBe(true);
    expect(countWaiters(f.roots[0])).toBe(N - 1);
  });
  it("5000 derinlikte zincir yığını taşırmaz (özyinelemesiz)", () => {
    const n = 5000;
    const chain = [s(1)];
    for (let p = 2; p <= n; p++) chain.push(s(p, [p - 1]));
    const f = buildForest(chain, Infinity, Infinity);
    expect(f.truncated).toBe(false);
    expect(size(f.roots[0])).toBe(n);
    expect(countWaiters(f.roots[0])).toBe(n - 1);
    // varsayılan sınırlarla kesilir, yine taşmaz
    const d = buildForest(chain);
    expect(d.truncated).toBe(true);
    expect(size(d.roots[0])).toBeLessThanOrEqual(2000);
  });
  it("düğüm sınırı: truncated bayrağı, toplam düğüm sınırı aşmaz", () => {
    const many = [s(1), s(2)];
    for (let p = 10; p < 40; p++) many.push(s(p, [1, 2]));
    const f = buildForest(many, 20);
    expect(f.truncated).toBe(true);
    expect(f.roots.reduce((a, r) => a + size(r), 0)).toBeLessThanOrEqual(20);
    expect(buildForest(many).truncated).toBe(false);
    // derinlik sınırı da truncated
    expect(buildForest([s(1), s(2, [1]), s(3, [2])], Infinity, 1).truncated).toBe(true);
    expect(buildForest([s(1), s(2, [1]), s(3, [2])], Infinity, 2).truncated).toBe(false);
  });
  it("A→B→A ile kuyruk: ata kenarı cycle işaretler, düğüm eklemez", () => {
    const f = buildForest([s(1, [2]), s(2, [1]), s(3, [1, 2])]);
    expect(f.truncated).toBe(false);
    expect(pids(f.roots)).toEqual([1]);
    const r = f.roots[0];
    expect(r.cycle).toBe(true);
    expect(pids(r.children)).toEqual([2, 3]);
    expect(r.children[0].cycle).toBe(true); // 2 → 1 ata
    expect(r.children[1].cycle).toBe(false);
    expect(size(r)).toBe(3);
  });
  it("pid 0 kuyruğu: hazır transaction kökü altında düz", () => {
    const f = buildForest([s(5, [0]), s(6, [0, 5]), s(7, [0, 5, 6])]);
    expect(pids(f.roots)).toEqual([0]);
    expect(f.roots[0].kind).toBe("prepared");
    expect(pids(f.roots[0].children)).toEqual([5, 6, 7]);
    expect(countWaiters(f.roots[0])).toBe(3);
  });
});

describe("pendingLocksByPid", () => {
  const lk = (pid: number, mode: string, granted: boolean, object: string | null, locktype = "relation"): LockRow => ({
    pid, locktype, mode, granted, object, databaseOid: null,
  });
  it("yalnızca verilmemiş kilitler; nesnesizde locktype; tekrarsız", () => {
    const m = pendingLocksByPid([
      lk(1, "AccessShareLock", true, "t"),
      lk(2, "RowExclusiveLock", false, "public.t"),
      lk(2, "RowExclusiveLock", false, "public.t"),
      lk(3, "ShareLock", false, null, "transactionid"),
    ]);
    expect(m.get(1)).toBeUndefined();
    expect(m.get(2)).toEqual(["RowExclusiveLock public.t"]);
    expect(m.get(3)).toEqual(["ShareLock transactionid"]);
  });
});
