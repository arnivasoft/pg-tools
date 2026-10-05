import {
  Box, Columns3, Database, Eye, DatabaseZap, Folder, FunctionSquare, HardDrive, Hash, KeyRound, Layers, ListOrdered,
  Puzzle, Server, Shapes, Shield, ShieldCheck, SquareTerminal, Table2, TableProperties, User, Users, Zap,
} from "lucide-solid";
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
    case "constraint":
      return node.flags.primaryKey ? KeyRound : ShieldCheck;
    case "index":
      return ListOrdered;
    case "trigger":
    case "triggerFunction":
      return Zap;
    case "policy":
      return Shield;
    case "view":
      return Eye;
    case "materializedView":
      return DatabaseZap;
    case "function":
      return FunctionSquare;
    case "procedure":
      return SquareTerminal;
    case "sequence":
      return Hash;
    case "type":
      return Shapes;
    case "domain":
      return Box;
    case "extension":
      return Puzzle;
    case "role":
      return node.detail === "group" ? Users : User;
    case "tablespace":
      return HardDrive;
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
