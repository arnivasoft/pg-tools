import { createMemo, createSignal } from "solid-js";
import { ExplorerController, type ExplorerDeps } from "./controller";
import { emptyTree, flatten } from "./tree-model";

export function createExplorer(deps: ExplorerDeps) {
  const [state, setState] = createSignal(emptyTree());
  const controller = new ExplorerController(state, setState, deps);
  const rows = createMemo(() => flatten(state()));
  const selectedNode = createMemo(() => {
    const s = state();
    return s.selected === null ? null : s.nodes.get(s.selected) ?? null;
  });
  return { state, rows, selectedNode, controller };
}

export type Explorer = ReturnType<typeof createExplorer>;
