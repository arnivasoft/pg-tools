// Ortam ve toplu işlem diyaloglarının saf kuralları: yalnız `tx-dialogs.tsx` (tembel parça) içe aktarır; ana paket
// modüllerinden (`workspace-model.ts`) ayrı tutulur ki ana pakete bayt eklemesin.
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import { suggestLock } from "../servers/server-form";
import { envLabel, type EnvChoice, type PendingTx } from "./workspace-model";

export const ENV_CHOICES: readonly EnvChoice[] = ["production", "test", "development"];

/** Seçime göre önerilen kilit (sunucu formundaki `suggestLock` ile aynı kural). */
export function envLockDefault(config: Pick<ServerConfig, "label" | "readOnlyLock">, choice: EnvChoice): boolean {
  return suggestLock(config.label ?? "", envLabel(choice), config.readOnlyLock, false);
}

/** "Tümünü kaydet" yalnızca en az bir sekme COMMIT edilebiliyorsa sunulur. */
export function canCommitAny(list: PendingTx[]): boolean {
  return list.some((p) => p.tx === "inTransaction" && !p.busy);
}
