import { Columns3, Database, Folder, KeyRound, Layers, Server, Table2, TableProperties } from "lucide-solid";
import { Dynamic } from "solid-js/web";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import { isCollection } from "./tree-model";

function iconFor(node: TreeNode) {
  switch (node.kind) {
    case "server":
      return Server;
    case "database":
      return Database;
    case "schema":
      return Layers;
    case "table":
      return node.flags.partitioned ? TableProperties : Table2;
    case "column":
      return node.flags.primaryKey ? KeyRound : Columns3;
    default:
      return Folder;
  }
}

export function NodeIcon(props: { node: TreeNode }) {
  const color = () => {
    if (props.node.kind === "server") return props.node.flags.connected ? "text-accent" : "text-muted";
    return isCollection(props.node.kind) ? "text-muted" : "";
  };
  return <Dynamic component={iconFor(props.node)} size={14} class={`shrink-0 ${color()}`} aria-hidden="true" />;
}
