import { Dynamic } from "solid-js/web";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { iconFor } from "./node-icon";
import { isCollection } from "./tree-model";

export function NodeIcon(props: { node: TreeNode }) {
  const color = () => {
    if (props.node.kind === "server") return props.node.flags.connected ? "text-accent" : "text-muted";
    return isCollection(props.node.kind) ? "text-muted" : "";
  };
  return <Dynamic component={iconFor(props.node)} size={14} class={`shrink-0 ${color()}`} aria-hidden="true" />;
}
