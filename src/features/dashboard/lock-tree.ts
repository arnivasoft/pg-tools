// Engelleme ağacı (saf): `pg_blocking_pids` ilişkisinden kök engelleyici → bekleyen ağaçları.
import type { BlockSession } from "../../ipc/bindings/BlockSession";

export interface LockNode {
  pid: number;
  /** null: hazır transaction (pid 0) ya da listede olmayan engelleyici. */
  session: BlockSession | null;
  kind: "session" | "prepared";
  children: LockNode[];
  /** Bu düğümde bir döngü kesildi (ya da düğüm döngüden seçilen kök). */
  cycle: boolean;
}

/**
 * Kökler: kimseyi beklemeyen ama başkalarını engelleyen oturumlar (hazır transaction için pid 0 `prepared` düğümü, listede
 * olmayan engelleyici için `session: null`). Hiçbir kökten erişilemeyen bekleyenler döngü ya da döngü kuyruğudur: döngüdeki
 * en küçük pid kök olur (`cycle: true`); döngü, atalara dönen kenar kesilerek sonlandırılır. Birden çok engelleyiciyi bekleyen
 * oturum her ilgili ağaçta görünür.
 */
export function buildLockTree(sessions: readonly BlockSession[]): LockNode[] {
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
  const make = (pid: number, ancestors: Set<number>, forceCycle: boolean): LockNode => {
    reached.add(pid);
    const session = byPid.get(pid) ?? null;
    const node: LockNode = { pid, session, kind: pid === 0 ? "prepared" : "session", children: [], cycle: forceCycle };
    ancestors.add(pid);
    for (const w of waiters.get(pid) ?? []) {
      if (ancestors.has(w)) node.cycle = true;
      else node.children.push(make(w, ancestors, false));
    }
    ancestors.delete(pid);
    return node;
  };

  // Engelleyici olup kimseyi beklemeyenler (listede olmayanlar ve 0 dahil).
  const rootPids = [...waiters.keys()].filter((p) => (byPid.get(p)?.blockedBy.length ?? 0) === 0).sort((a, b) => a - b);
  const roots = rootPids.map((p) => make(p, new Set(), false));

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
  for (;;) {
    const rest = sessions.map((s) => s.pid).filter((p) => !reached.has(p) && (byPid.get(p)?.blockedBy.length ?? 0) > 0);
    if (rest.length === 0) break;
    rest.sort((a, b) => a - b);
    const pick = rest.find(onCycle) ?? rest[0];
    roots.push(make(pick, new Set(), true));
  }
  return roots;
}

/** Alt ağaçtaki toplam bekleyen sayısı; aynı oturum birden çok yoldan görünse de bir kez, düğümün kendisi ve hazır tx hariç. */
export function countWaiters(node: LockNode): number {
  const seen = new Set<number>();
  const walk = (n: LockNode) => {
    for (const c of n.children) {
      seen.add(c.pid);
      walk(c);
    }
  };
  walk(node);
  seen.delete(node.pid);
  seen.delete(0);
  return seen.size;
}
