import type { TreeNode } from "../../ipc/bindings/TreeNode";

/**
 * "Dashboard'u aç" menüsünün göründüğü düğümler: bağlı sunucu (sunucu düzeyi) ve veritabanı. Gezgin bunu statik içe
 * aktarır; bu dosya bilerek bağımlılıksızdır (dashboard parçasının geri kalanını ana pakete çekmez).
 */
export function canOpenDashboard(node: Pick<TreeNode, "kind" | "flags">): boolean {
  return node.kind === "database" || (node.kind === "server" && node.flags.connected);
}
