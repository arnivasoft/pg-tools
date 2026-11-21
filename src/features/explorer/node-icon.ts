// Ağaç düğümü simgesi seçimi (Plan 38 Karar 7). Eski `switch` yerine tür → simge tablosu (ana paket telafisi);
// yalnız bayrağa bağlı özel durumlar (partitioned tablo, PK kolon/kısıt, group rol) tablodan önce denetlenir.
// JSX içermez; vitest (node ortamı) doğrudan test edebilsin diye NodeIcon.tsx'ten ayrıdır.
import {
  Box, Columns3, Database, Eye, DatabaseZap, Folder, FunctionSquare, HardDrive, Hash, KeyRound, Layers, ListOrdered,
  Puzzle, Server, Shapes, Shield, ShieldCheck, SquareTerminal, Table2, TableProperties, User, Users, Zap,
} from "lucide-solid";
import type { NodeKind } from "../../ipc/bindings/NodeKind";
import type { TreeNode } from "../../ipc/bindings/TreeNode";

// Koleksiyonlar ve tabloda olmayan türler `Folder` (varsayılan).
const ICONS: Partial<Record<NodeKind, typeof Folder>> = {
  server: Server,
  database: Database,
  schema: Layers,
  table: Table2,
  column: Columns3,
  constraint: ShieldCheck,
  index: ListOrdered,
  trigger: Zap,
  triggerFunction: Zap,
  policy: Shield,
  view: Eye,
  materializedView: DatabaseZap,
  function: FunctionSquare,
  procedure: SquareTerminal,
  sequence: Hash,
  type: Shapes,
  domain: Box,
  extension: Puzzle,
  role: User,
  tablespace: HardDrive,
  // Plan 38: FDW/replikasyon düğümleri, mevcut içe aktarılmış simgelerle (yeni lucide simgesi yok).
  foreignDataWrapper: Puzzle,
  foreignServer: Server,
  userMapping: User,
  foreignTable: Table2,
  publication: DatabaseZap,
  subscription: Database,
};

export function iconFor(node: TreeNode): typeof Folder {
  const k = node.kind;
  if (k === "table" && node.flags.partitioned) return TableProperties;
  if ((k === "column" || k === "constraint") && node.flags.primaryKey) return KeyRound;
  if (k === "role" && node.detail === "group") return Users;
  return ICONS[k] ?? Folder;
}
