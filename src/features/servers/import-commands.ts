// pgAdmin içe aktarma diyaloğuna (ImportDialog tembel parçası) özgü komut sarmalayıcıları. Ana paketteki commands.ts'ten
// taşındı (Plan 32 telafisi: nesne arama kancasının baytları).
import type { ImportItem } from "../../ipc/bindings/ImportItem";
import { call } from "../../ipc/commands";

export const importCommands = {
  previewPgadminImport: (path: string) => call<ImportItem[]>("preview_pgadmin_import", { path }),
  applyPgadminImport: (path: string, indices: number[]) => call<number>("apply_pgadmin_import", { path, indices }),
};
