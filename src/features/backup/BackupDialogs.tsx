// Yedekleme / geri yükleme diyalogları (tembel parça; Plan 27). Tek örnek: `job.mode` hangi diyaloğun açık olduğunu söyler.
// Ana modülleri (`app/strings`, `explorer/*`, `tree-model`, `designer/*`, `maintenance/*`) içe aktarmaz: api/ağaç/bildirim App'ten
// props ile gelir. Durum denetleyicilerdedir; bileşenler yalnız çizer.
import { open, save } from "@tauri-apps/plugin-dialog";
import { createEffect, on, onCleanup, untrack, type Accessor } from "solid-js";
import type { ToastKind } from "../../app/toast-model";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { TreeNode } from "../../ipc/bindings/TreeNode";
import type { Api } from "../../ipc/commands";
import { createBackupController, createRestoreController, type BackupControllerDeps } from "./backup-controller";
import { BackupDialog } from "./BackupDialog";
import { makePicker } from "./picker";
import { RestoreDialog } from "./RestoreDialog";

/** `ExplorerController`'ın yalnız ihtiyaç duyulan kısmı (tür içe aktarımı; çalışma zamanı bağımlılığı yok). */
type ExplorerRefresh = { refresh: (id: string) => Promise<void> };

export interface BackupDialogDeps {
  api: Pick<
    Api,
    | "backupTools" | "backupPlan" | "backupStart" | "backupCancel" | "backupReveal"
    | "restoreInspect" | "restorePlan" | "restoreStart" | "settingsGet" | "settingsSetPgBinDir" | "listServers"
    | "ddlTargetOf" | "ddlLookup" | "listChildren"
  >;
  /** Explorer ağacı: düğüm kimliğinden sunucu/veritabanı/kapsam, sunucu `detail`'i sürüm, `flags.locked` kilit. */
  tree: Accessor<{ nodes: Map<string, TreeNode>; parent: Map<string, string> }>;
  /** Sorgu denetleyicisinin sunucu listesi (üretim etiketi ve ad için). */
  query: { servers: () => ServerConfig[] };
  connect: (serverId: string) => Promise<boolean>;
  notify: (text: string, kind: ToastKind) => void;
  refreshInspector?: () => void;
  explorer: ExplorerRefresh;
}

export function BackupDialogs(props: { job: { mode: "backup" | "restore"; nodeId: string } | null; deps: BackupDialogDeps; onClose: () => void }) {
  const deps = untrack(() => props.deps);
  const common: BackupControllerDeps = {
    api: deps.api,
    tree: () => deps.tree(),
    servers: () => deps.query.servers(),
    connect: deps.connect,
    refreshNode: (id) => {
      // Yenileme hatası işi etkilemez.
      void deps.explorer.refresh(id).catch(() => {});
    },
    refreshInspector: deps.refreshInspector,
    notify: deps.notify,
    pick: makePicker({ save: (o) => save(o as Parameters<typeof save>[0]), open: (o) => open(o as Parameters<typeof open>[0]) }),
    onClosed: () => props.onClose(),
    newId: () => crypto.randomUUID(),
    now: () => Date.now(),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  };
  const backupCtl = createBackupController(common);
  const restoreCtl = createRestoreController(common);

  createEffect(
    on(
      () => props.job,
      (job) => {
        if (job?.mode === "backup") {
          restoreCtl.close();
          backupCtl.open(job.nodeId);
        } else if (job?.mode === "restore") {
          backupCtl.close();
          restoreCtl.open(job.nodeId);
        } else {
          backupCtl.close();
          restoreCtl.close();
        }
      },
    ),
  );
  onCleanup(() => {
    backupCtl.close();
    restoreCtl.close();
  });

  return (
    <>
      <BackupDialog ctl={backupCtl} />
      <RestoreDialog ctl={restoreCtl} tree={() => deps.tree()} />
    </>
  );
}
