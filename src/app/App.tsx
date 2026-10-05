import { createSignal, onCleanup, onMount } from "solid-js";
import { createExplorer } from "../features/explorer/explorer-store";
import { TreeView } from "../features/explorer/TreeView";
import { ROOT } from "../features/explorer/tree-model";
import { createPasswordPrompt } from "../features/servers/PasswordPrompt";
import { commands } from "../ipc/commands";
import { SplitPane } from "./SplitPane";
import { StatusBar } from "./StatusBar";
import { Toolbar } from "./Toolbar";
import { applyTheme, loadThemePreference, nextPreference, saveThemePreference, watchSystemTheme } from "./theme";

export default function App() {
  const prompt = createPasswordPrompt();
  const explorer = createExplorer({ api: commands, requestPassword: prompt.ask });
  const [themePref, setThemePref] = createSignal(loadThemePreference());
  const [notice, setNotice] = createSignal<string | null>(null);

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
        <Toolbar onRefresh={refreshSelected} themePref={themePref()} onCycleTheme={cycleTheme} />
      </header>
      <SplitPane left={<TreeView explorer={explorer} />} right={<div class="h-full" />} />
      <footer class="flex h-6 shrink-0 items-center border-t border-border bg-panel px-2 text-xs text-muted">
        <StatusBar explorer={explorer} notice={notice()} />
      </footer>
      <prompt.View />
    </div>
  );
}
