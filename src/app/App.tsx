import { FileInput, Pencil, Plus } from "lucide-solid";
import { createSignal, onCleanup, onMount } from "solid-js";
import { createExplorer } from "../features/explorer/explorer-store";
import type { ActionDeps } from "../features/explorer/actions";
import { ExplorerMenu } from "../features/explorer/ExplorerMenu";
import { createDeleteConfirm } from "../ui/ConfirmDialog";
import { copyText } from "./clipboard";
import { Inspector } from "../features/inspector/Inspector";
import { ROOT } from "../features/explorer/tree-model";
import { createPasswordPrompt } from "../features/servers/PasswordPrompt";
import { ImportDialog } from "../features/servers/ImportDialog";
import { ServerDialog } from "../features/servers/ServerDialog";
import { groupsOf } from "../features/servers/server-form";
import type { ServerConfig } from "../ipc/bindings/ServerConfig";
import { commands } from "../ipc/commands";
import { toPayload } from "../ipc/errors";
import { Button } from "../ui/Button";
import { SplitPane } from "./SplitPane";
import { StatusBar } from "./StatusBar";
import { t } from "./strings";
import { Toaster } from "./Toaster";
import { toast } from "./toast";
import { Toolbar } from "./Toolbar";
import { applyTheme, loadThemePreference, nextPreference, saveThemePreference, watchSystemTheme } from "./theme";

export default function App() {
  const prompt = createPasswordPrompt();
  const explorer = createExplorer({ api: commands, requestPassword: prompt.ask });
  const [themePref, setThemePref] = createSignal(loadThemePreference());
  const [dialogOpen, setDialogOpen] = createSignal(false);
  const [editing, setEditing] = createSignal<ServerConfig | null>(null);
  const [importOpen, setImportOpen] = createSignal(false);
  const [groups, setGroups] = createSignal<string[]>([]);

  const openDialog = async (serverId: string | null) => {
    try {
      const servers = await commands.listServers();
      const target = serverId === null ? null : (servers.find((s) => s.id === serverId) ?? null);
      if (serverId !== null && target === null) return;
      setGroups(groupsOf(servers));
      setEditing(target);
      setDialogOpen(true);
    } catch (e) {
      toast.show(toPayload(e).message, "error");
    }
  };
  const deleteConfirm = createDeleteConfirm();
  const actionDeps: ActionDeps = {
    state: explorer.state,
    controller: explorer.controller,
    api: commands,
    copy: copyText,
    notify: toast.show,
    editServer: (serverId) => void openDialog(serverId),
    confirmDelete: deleteConfirm.ask,
  };
  const selectedServerId = () => {
    const node = explorer.selectedNode();
    return node?.kind === "server" ? node.serverId : null;
  };

  onMount(async () => {
    try {
      await explorer.controller.load(ROOT);
    } catch {
      // load errors are surfaced on the tree node itself
    } finally {
      const n = await commands.startupNotice().catch(() => null);
      if (n) toast.show(n, "error");
    }
  });
  onCleanup(watchSystemTheme(themePref));

  const cycleTheme = () => {
    const next = nextPreference(themePref());
    setThemePref(next);
    saveThemePreference(next);
    applyTheme(next);
  };

  let filterEl!: HTMLInputElement;
  const [treeEl, setTreeEl] = createSignal<HTMLElement | null>(null);
  const clearFilter = () => {
    explorer.controller.setFilter("");
    treeEl()?.focus();
  };
  const onWindowKey = (e: KeyboardEvent) => {
    if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      filterEl.focus();
      filterEl.select();
    }
  };
  window.addEventListener("keydown", onWindowKey);
  onCleanup(() => window.removeEventListener("keydown", onWindowKey));

  const refreshSelected = () => {
    const selected = explorer.state().selected;
    void (selected === null ? explorer.controller.reloadServers() : explorer.controller.refresh(selected));
  };

  return (
    <div class="flex h-full flex-col">
      <header class="flex h-9 shrink-0 items-center gap-1 border-b border-border bg-panel px-2">
        <Toolbar
          onRefresh={refreshSelected}
          filter={explorer.state().filter}
          onFilter={(q) => explorer.controller.setFilter(q)}
          onFilterEscape={clearFilter}
          onFilterDown={() => treeEl()?.focus()}
          filterRef={(el) => (filterEl = el)} themePref={themePref()} onCycleTheme={cycleTheme}>
          <Button variant="ghost" onClick={() => void openDialog(null)}>
            <Plus size={14} /> {t.toolbar.addServer}
          </Button>
          <Button variant="ghost" title={t.import.title} aria-label={t.import.title} onClick={() => setImportOpen(true)}>
            <FileInput size={14} /> {t.import.button}
          </Button>
          <Button
            variant="ghost"
            disabled={selectedServerId() === null}
            title={t.toolbar.editServer}
            aria-label={t.toolbar.editServer}
            onClick={() => void openDialog(selectedServerId())}
          >
            <Pencil size={14} />
          </Button>
        </Toolbar>
      </header>
      <SplitPane
        left={<ExplorerMenu explorer={explorer} deps={actionDeps} treeRef={setTreeEl} />}
        right={<Inspector explorer={explorer} />}
      />
      <footer class="flex h-6 shrink-0 items-center border-t border-border bg-panel px-2 text-xs text-muted">
        <StatusBar explorer={explorer} />
      </footer>
      <prompt.View />
      <deleteConfirm.View />
      <Toaster />
      <ImportDialog
        open={importOpen()}
        onClose={() => setImportOpen(false)}
        onImported={() => {
          explorer.controller.reloadServers().catch(() => {});
        }}
      />
      <ServerDialog
        open={dialogOpen()}
        editing={editing()}
        groups={groups()}
        onClose={() => setDialogOpen(false)}
        onSaved={() => {
          setDialogOpen(false);
          explorer.controller.reloadServers().catch(() => {});
        }}
      />
    </div>
  );
}
