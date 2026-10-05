import { Moon, RefreshCw, X, Sun, SunMoon } from "lucide-solid";
import { Match, Switch, type JSX } from "solid-js";
import { Button } from "../ui/Button";
import { t } from "./strings";
import type { ThemePreference } from "./theme";

export function Toolbar(props: {
  onRefresh: () => void; themePref: ThemePreference; onCycleTheme: () => void; children?: JSX.Element;
  filter: string;
  onFilter: (q: string) => void;
  onFilterEscape: () => void;
  onFilterDown: () => void;
  filterRef: (el: HTMLInputElement) => void;
}) {
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
      <div class="relative ml-2 w-56">
        <input
          ref={props.filterRef}
          type="search"
          value={props.filter}
          placeholder={t.toolbar.filter}
          aria-label={t.toolbar.filter}
          class="h-7 w-full rounded border border-border bg-transparent pl-2 pr-7 text-[13px] outline-none focus-visible:ring-1 focus-visible:ring-accent [&::-webkit-search-cancel-button]:hidden"
          onInput={(e) => props.onFilter(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              props.onFilterEscape();
            } else if (e.key === "ArrowDown") {
              e.preventDefault();
              props.onFilterDown();
            }
          }}
        />
        {props.filter !== "" && (
          <button
            type="button"
            title={t.toolbar.clearFilter}
            aria-label={t.toolbar.clearFilter}
            class="absolute right-1 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted hover:text-fg"
            onClick={props.onFilterEscape}
          >
            <X size={12} />
          </button>
        )}
      </div>
      <div class="flex-1" />
      <Button variant="ghost" title={t.toolbar.theme[props.themePref]} aria-label={t.toolbar.theme[props.themePref]} onClick={props.onCycleTheme}>
        <ThemeIcon />
      </Button>
    </>
  );
}
