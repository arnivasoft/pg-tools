// Engelleme ağacı (saf): `pg_blocking_pids` ilişkisinden kök engelleyici → bekleyen ağaçları.
import type { BlockSession } from "../../ipc/bindings/BlockSession";
import type { LockRow } from "../../ipc/bindings/LockRow";

export interface LockNode {
  pid: number;
  /** null: hazır transaction (pid 0) ya da listede olmayan engelleyici. */
  session: BlockSession | null;
  kind: "session" | "prepared";
  children: LockNode[];
  /** Bu düğümde bir döngü kesildi (ya da düğüm döngüden seçilen kök). */
  cycle: boolean;
}

export interface LockForest {
  roots: LockNode[];
  /** Düğüm sınırına ulaşıldı: ağaç eksik çizildi. */
  truncated: boolean;
}

/** Tüm ormandaki en çok düğüm sayısı (çok büyük kilit kuyruğunda arayüzü korur). */
export const LOCK_TREE_CAP = 2000;
/** En çok derinlik: ağaç bileşeni iç içe çizildiğinden çok derin zincir arayüz yığınını taşırmasın. */
export const LOCK_TREE_MAX_DEPTH = 100;

/**
 * Kökler: kimseyi beklemeyen ama başkalarını engelleyen oturumlar (hazır transaction için pid 0 `prepared` düğümü, listede
 * olmayan engelleyici için `session: null`). Hiçbir kökten erişilemeyen bekleyenler döngü ya da döngü kuyruğudur: döngüdeki
 * en küçük pid kök olur (`cycle: true`).
 *
 * `pg_blocking_pids` aynı kuyrukta öndeki bekleyenleri de ("yumuşak" engelleyici) listeler: N oturum tek kilidi bekliyorsa
 * k. bekleyen {tutan, w1..wk-1} ile engellenir; her yolu açmak 2^(N-1) düğüm demektir. Bu yüzden her kökün ağacı genişlik
 * öncelikli (özyinelemesiz) kurulur ve bir pid o ağaçta en çok bir kez, en sığ derinliğinde yer alır (kuyruk tutanın altında
 * düz görünür). Zaten yerleşmiş pid'e giden kenar düğüm eklemez; o pid bir ataysa düğüm `cycle` işaretlenir. Yerleşim kök
 * başınadır: iki engelleyiciyi bekleyen oturum her ilgili ağaçta görünür. Toplam düğüm `cap`'i ya da derinlik `maxDepth`'i aşarsa
 * `truncated`.
 */
export function buildLockTree(
  sessions: readonly BlockSession[], cap = LOCK_TREE_CAP, maxDepth = LOCK_TREE_MAX_DEPTH,
): LockForest {
  const byPid = new Map<number, BlockSession>();
  for (const s of sessions) byPid.set(s.pid, s);
  // engelleyici pid → bekleyenler (pid sıralı, tekrarsız)
  const waiters = new Map<number, number[]>();
  for (const s of sessions) {
    for (const b of new Set(s.blockedBy)) {
      let l = waiters.get(b);
      if (!l) waiters.set(b, (l = []));
      l.push(s.pid);
    }
  }
  for (const l of waiters.values()) l.sort((a, b) => a - b);

  const reached = new Set<number>();
  let total = 0;
  let truncated = false;
  const node = (pid: number, cycle: boolean): LockNode => {
    reached.add(pid);
    total++;
    return { pid, session: byPid.get(pid) ?? null, kind: pid === 0 ? "prepared" : "session", children: [], cycle };
  };

  /** Bir kökün ağacı (BFS); sınır doluysa null. */
  const build = (rootPid: number, forceCycle: boolean): LockNode | null => {
    if (total >= cap) {
      truncated = true;
      return null;
    }
    const root = node(rootPid, forceCycle);
    const parent = new Map<number, number | null>([[rootPid, null]]); // yerleşen pid → ata pid
    const isAncestor = (pid: number, from: number): boolean => {
      for (let p: number | null | undefined = from; p != null; p = parent.get(p)) if (p === pid) return true;
      return false;
    };
    const queue: LockNode[] = [root];
    const depth: number[] = [0]; // queue ile paralel
    for (let head = 0; head < queue.length; head++) {
      const n = queue[head];
      const d = depth[head];
      for (const w of waiters.get(n.pid) ?? []) {
        if (parent.has(w)) {
          if (isAncestor(w, n.pid)) n.cycle = true;
          continue;
        }
        if (d >= maxDepth) {
          truncated = true;
          break;
        }
        if (total >= cap) {
          truncated = true;
          return root;
        }
        const c = node(w, false);
        parent.set(w, n.pid);
        n.children.push(c);
        queue.push(c);
        depth.push(d + 1);
      }
    }
    return root;
  };

  const roots: LockNode[] = [];
  const add = (r: LockNode | null) => {
    if (r) roots.push(r);
  };
  // Engelleyici olup kimseyi beklemeyenler (listede olmayanlar ve 0 dahil).
  const rootPids = [...waiters.keys()].filter((p) => (byPid.get(p)?.blockedBy.length ?? 0) === 0).sort((a, b) => a - b);
  for (const p of rootPids) add(build(p, false));

  // Erişilemeyen bekleyenler: döngüdeki en küçük pid'i kök yap, bitene kadar tekrarla.
  const onCycle = (start: number): boolean => {
    const seen = new Set<number>();
    const stack = [...(waiters.get(start) ?? [])];
    while (stack.length) {
      const p = stack.pop()!;
      if (p === start) return true;
      if (seen.has(p)) continue;
      seen.add(p);
      stack.push(...(waiters.get(p) ?? []));
    }
    return false;
  };
  while (!truncated) {
    const rest = sessions.map((s) => s.pid).filter((p) => !reached.has(p) && (byPid.get(p)?.blockedBy.length ?? 0) > 0);
    if (rest.length === 0) break;
    rest.sort((a, b) => a - b);
    const pick = rest.find(onCycle) ?? rest[0];
    add(build(pick, true));
  }
  return { roots, truncated };
}

/** Alt ağaçtaki toplam bekleyen sayısı; aynı oturum birden çok yoldan görünse de bir kez, düğümün kendisi ve hazır tx hariç. */
export function countWaiters(node: LockNode): number {
  const seen = new Set<number>();
  const stack = [...node.children];
  while (stack.length) {
    const c = stack.pop()!;
    seen.add(c.pid);
    for (const g of c.children) stack.push(g);
  }
  seen.delete(node.pid);
  seen.delete(0);
  return seen.size;
}

/**
 * Düğümün beklediği kilitler: `granted = false` satırlarından pid başına "mode object" metinleri (nesnesiz kilitte locktype).
 * Ağaç bir kez kurulur, düğümler `get(pid)` ile okur.
 */
export function pendingLocksByPid(locks: readonly LockRow[]): Map<number, string[]> {
  const out = new Map<number, string[]>();
  for (const r of locks) {
    if (r.granted) continue;
    let l = out.get(r.pid);
    if (!l) out.set(r.pid, (l = []));
    const text = `${r.mode} ${r.object ?? r.locktype}`;
    if (!l.includes(text)) l.push(text);
  }
  return out;
}
