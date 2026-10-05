import { Moon, RefreshCw, Sun, SunMoon } from "lucide-solid";
import { Match, Switch, type JSX } from "solid-js";
import { Button } from "../ui/Button";
import { t } from "./strings";
import type { ThemePreference } from "./theme";

export function Toolbar(props: { onRefresh: () => void; themePref: ThemePreference; onCycleTheme: () => void; children?: JSX.Element }) {
  const ThemeIcon = () => (
    <Switch fallback={<SunMoon size={14} />}>
      <Match when={props.themePref === "light"}>
        <Sun size={14} />
      </Match>
      <Match when={props.themePref === "dark"}>
        <Moon size={14} />
      </Match>
    </Switch>
  );
  return (
    <>
      {props.children}
      <Button variant="ghost" title={t.toolbar.refresh} aria-label={t.toolbar.refresh} onClick={props.onRefresh}>
        <RefreshCw size={14} />
      </Button>
      <div class="flex-1" />
      <Button variant="ghost" title={t.toolbar.theme[props.themePref]} aria-label={t.toolbar.theme[props.themePref]} onClick={props.onCycleTheme}>
        <ThemeIcon />
      </Button>
    </>
  );
}
