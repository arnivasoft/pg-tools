import { Pencil, Plus } from "lucide-solid";
import { createSignal, onCleanup, onMount } from "solid-js";
import { createExplorer } from "../features/explorer/explorer-store";
import { TreeView } from "../features/explorer/TreeView";
import { Inspector } from "../features/inspector/Inspector";
import { ROOT } from "../features/explorer/tree-model";
import { createPasswordPrompt } from "../features/servers/PasswordPrompt";
import { ServerDialog } from "../features/servers/ServerDialog";
import { groupsOf } from "../features/servers/server-form";
import type { ServerConfig } from "../ipc/bindings/ServerConfig";
import { commands } from "../ipc/commands";
import { toPayload } from "../ipc/errors";
import { Button } from "../ui/Button";
import { SplitPane } from "./SplitPane";
import { StatusBar } from "./StatusBar";
import { t } from "./strings";
import { Toolbar } from "./Toolbar";
import { applyTheme, loadThemePreference, nextPreference, saveThemePreference, watchSystemTheme } from "./theme";

export default function App() {
  const prompt = createPasswordPrompt();
  const explorer = createExplorer({ api: commands, requestPassword: prompt.ask });
  const [themePref, setThemePref] = createSignal(loadThemePreference());
  const [notice, setNotice] = createSignal<string | null>(null);
  const [dialogOpen, setDialogOpen] = createSignal(false);
  const [editing, setEditing] = createSignal<ServerConfig | null>(null);
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
      setNotice(toPayload(e).message);
    }
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
      setNotice(await commands.startupNotice().catch(() => null));
    }
  });
  onCleanup(watchSystemTheme(themePref));

  const cycleTheme = () => {
    const next = nextPreference(themePref());
    setThemePref(next);
    saveThemePreference(next);
    applyTheme(next);
  };

  const refreshSelected = () => {
    const selected = explorer.state().selected;
    void (selected === null ? explorer.controller.reloadServers() : explorer.controller.refresh(selected));
  };

  return (
    <div class="flex h-full flex-col">
      <header class="flex h-9 shrink-0 items-center gap-1 border-b border-border bg-panel px-2">
        <Toolbar onRefresh={refreshSelected} themePref={themePref()} onCycleTheme={cycleTheme}>
          <Button variant="ghost" onClick={() => void openDialog(null)}>
            <Plus size={14} /> {t.toolbar.addServer}
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
      <SplitPane left={<TreeView explorer={explorer} />} right={<Inspector explorer={explorer} />} />
      <footer class="flex h-6 shrink-0 items-center border-t border-border bg-panel px-2 text-xs text-muted">
        <StatusBar explorer={explorer} notice={notice()} />
      </footer>
      <prompt.View />
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
