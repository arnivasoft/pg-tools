import { FileInput, Pencil, Plus, SquareTerminal } from "lucide-solid";
import { createEffect, createSignal, lazy, onCleanup, onMount, Show, Suspense } from "solid-js";
import { createExplorer } from "../features/explorer/explorer-store";
import type { ActionDeps } from "../features/explorer/actions";
import { ExplorerMenu } from "../features/explorer/ExplorerMenu";
import { createDeleteConfirm } from "../ui/ConfirmDialog";
import { copyText } from "./clipboard";
import { Inspector } from "../features/inspector/Inspector";
import { ROOT } from "../features/explorer/tree-model";
import { createPasswordPrompt } from "../features/servers/PasswordPrompt";
import { groupsOf } from "../features/servers/server-form";
import { createQueryController } from "../features/query/query-controller";
import { QueryTabs } from "../features/query/QueryTabs";
import { newQueryNode, saveClosesSessions } from "../features/query/workspace-model";
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
import { readStorage, writeStorage } from "./storage";
import { applyTheme, loadThemePreference, nextPreference, saveThemePreference, watchSystemTheme } from "./theme";

const ServerDialog = lazy(() => import("../features/servers/ServerDialog").then((m) => ({ default: m.ServerDialog })));
const ImportDialog = lazy(() => import("../features/servers/ImportDialog").then((m) => ({ default: m.ImportDialog })));

const SHOW_SYSTEM_KEY = "pgt.showSystem";

export default function App() {
  const prompt = createPasswordPrompt();
  const explorer = createExplorer({ api: commands, requestPassword: prompt.ask });
  const query = createQueryController({
    api: commands,
    notify: toast.show,
    tree: explorer.state,
    connect: async (serverId) => {
      const node = [...explorer.state().nodes.values()].find((n) => n.kind === "server" && n.serverId === serverId);
      return node ? explorer.controller.ensureConnected(node.id) : false;
    },
    serversChanged: () => void explorer.controller.reloadServers().catch(() => {}),
  });
  const [themePref, setThemePref] = createSignal(loadThemePreference());
  const [dialogOpen, setDialogOpen] = createSignal(false);
  const [editing, setEditing] = createSignal<ServerConfig | null>(null);
  const [importOpen, setImportOpen] = createSignal(false);
  const [showSystem, setShowSystem] = createSignal(readStorage(SHOW_SYSTEM_KEY) === "1");
  const [serverEverOpened, setServerEverOpened] = createSignal(false);
  const [importEverOpened, setImportEverOpened] = createSignal(false);
  createEffect(() => dialogOpen() && setServerEverOpened(true));
  createEffect(() => importOpen() && setImportEverOpened(true));
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
    openQueryTool: (id) => void query.openFromNode(id),
    beforeServerChange: query.confirmServerChange,
    afterServerChange: (serverId, gone) => {
      query.serverSessionsClosed(serverId, gone);
      if (gone) void query.refreshServers();
    },
  };
  const newQueryTarget = () => newQueryNode(explorer.state(), explorer.state().selected);
  const selectedServerId = () => {
    const node = explorer.selectedNode();
    return node?.kind === "server" ? node.serverId : null;
  };

  onMount(async () => {
    void query.restore();
    await commands.setShowSystem(showSystem()).catch(() => {
      // Arka uç varsayılanı kapalıdır: anahtar ve kayıtlı tercih buna uysun.
      setShowSystem(false);
      writeStorage(SHOW_SYSTEM_KEY, "0");
    });
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
      // Kalıcı diyalog açıkken arkadaki filtre kutusuna odak taşınmaz.
      if (document.querySelector('[role="dialog"]')) return;
      filterEl.focus();
      filterEl.select();
    }
  };
  window.addEventListener("keydown", onWindowKey);
  onCleanup(() => window.removeEventListener("keydown", onWindowKey));

  let togglingSystem = false;
  const toggleSystem = async () => {
    if (togglingSystem) return;
    togglingSystem = true;
    const next = !showSystem();
    try {
      await commands.setShowSystem(next);
      setShowSystem(next);
      writeStorage(SHOW_SYSTEM_KEY, next ? "1" : "0");
    } catch (e) {
      toast.show(toPayload(e).message, "error");
      togglingSystem = false;
      return;
    }
    try {
      await explorer.controller.reloadAll();
    } finally {
      togglingSystem = false;
    }
  };

  const refreshSelected = () => {
    const selected = explorer.state().selected;
    void (selected === null ? explorer.controller.reloadServers() : explorer.controller.refresh(selected));
  };

  return (
    <div class="flex h-full flex-col">
      <header class="flex h-9 shrink-0 items-center gap-1 border-b border-border bg-panel px-2">
        <Toolbar
          showSystem={showSystem()}
          onToggleSystem={() => void toggleSystem()}
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
            disabled={newQueryTarget() === null}
            title={t.toolbar.newQueryHint}
            onClick={() => {
              const id = newQueryTarget();
              if (id !== null) void query.openFromNode(id, { draft: false });
            }}
          >
            <SquareTerminal size={14} /> {t.toolbar.newQuery}
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
        right={<QueryTabs ctl={query} inspector={<Inspector explorer={explorer} />} />}
      />
      <footer class="flex h-6 shrink-0 items-center border-t border-border bg-panel px-2 text-xs text-muted">
        <StatusBar explorer={explorer} />
      </footer>
      <prompt.View />
      <deleteConfirm.View />
      <Toaster />
      <Show when={importEverOpened()}>
        <Suspense>
          <ImportDialog
            open={importOpen()}
            onClose={() => setImportOpen(false)}
            onImported={() => {
              explorer.controller.reloadServers().catch(() => {});
              void query.refreshServers();
            }}
          />
        </Suspense>
      </Show>
      <Show when={serverEverOpened()}>
        <Suspense>
          <ServerDialog
            open={dialogOpen()}
            editing={editing()}
            groups={groups()}
            onClose={() => setDialogOpen(false)}
            beforeSave={async (prev, next, typed) => !prev || !saveClosesSessions(prev, next, typed) || query.confirmServerChange(prev.id, prev.name)}
            onSaved={(saved, typed) => {
              if (saveClosesSessions(editing(), saved, typed)) query.serverSessionsClosed(saved.id, false);
              void query.refreshServers();
              setDialogOpen(false);
              explorer.controller.reloadServers().catch(() => {});
            }}
          />
        </Suspense>
      </Show>
    </div>
  );
}
