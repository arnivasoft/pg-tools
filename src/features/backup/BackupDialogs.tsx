// Yedekleme / geri yükleme diyalogları (tembel parça; Plan 27). Task 1 iskeleti: başlık, "Hazırlanıyor…" ve Kapat.
// Gerçek içerik Task 4'te yazılır; imza (props) değişmez. Ana modülleri (`app/strings`, `explorer/*`, `tree-model`,
// `designer/*`, `maintenance/*`) içe aktarmaz: api/ağaç/bildirim App'ten props ile gelir.
import type { Accessor } from "solid-js";
import type { ToastKind } from "../../app/toast-model";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import type { Api } from "../../ipc/commands";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";

/** `ExplorerController`'ın yalnız ihtiyaç duyulan kısmı (tür içe aktarımı; çalışma zamanı bağımlılığı yok). */
type ExplorerRefresh = { refresh: (id: string) => Promise<void> };

export interface BackupDialogDeps {
  api: Pick<
    Api,
    | "backupTools" | "backupPlan" | "backupStart" | "backupCancel" | "backupReveal"
    | "restoreInspect" | "restorePlan" | "restoreStart" | "settingsGet" | "settingsSetPgBinDir" | "listServers"
  >;
  /** Explorer ağacı: düğüm kimliğinden sunucu/veritabanı/kapsam, sunucu `detail`'i sürüm, `flags.locked` kilit. */
  tree: Accessor<{ nodes: Map<string, TreeNode>; parent: Map<string, string> }>;
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
  refreshInspector?: () => void;
  explorer: ExplorerRefresh;
}

export function BackupDialogs(props: { job: { mode: "backup" | "restore"; nodeId: string } | null; deps: BackupDialogDeps; onClose: () => void }) {
  return (
    <Dialog
      open={props.job !== null}
      onOpenChange={(o) => {
        if (!o) props.onClose();
      }}
      title={props.job?.mode === "restore" ? "Geri yükle" : "Yedekle"}
      width="640px"
      footer={<Button onClick={() => props.onClose()}>Kapat</Button>}
    >
      <p role="status">Hazırlanıyor…</p>
    </Dialog>
  );
}
