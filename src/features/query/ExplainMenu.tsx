// ContextBar'daki "Planı göster" düğmesi ve seçenek menüsü (tembel QueryTab parçası; Kobalte DropdownMenu burada
// serbesttir). Sürüm altı / ANALYZE gerektiren seçenekler devre dışıdır ve gerekçe öğe metninde görünür (ekran okuyucu
// da okur). Seçimler sekme başına `explain-controller`da tutulur.
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { ChevronDown, ListTree } from "lucide-solid";
import { For, Show } from "solid-js";
import type { ExplainOptions } from "../../ipc/bindings/ExplainOptions";
import { Button } from "../../ui/Button";
import { MENU_OPTIONS, optionGates, shownOptions, type ExplainKind, type MenuOption } from "./explain-model";
import { tplan } from "./strings-plan";

const itemClass =
  "flex min-h-7 cursor-default items-center gap-2 px-3 py-0.5 text-[13px] outline-none data-[highlighted]:bg-hover data-[disabled]:opacity-50";

export function ExplainMenu(props: {
  options: ExplainOptions;
  version: number | null;
  disabled: boolean;
  onExplain: (kind: ExplainKind) => void;
  onOption: (key: MenuOption, value: boolean) => void;
}) {
  const gates = () => optionGates(props.options, props.version);
  const shown = () => shownOptions(props.options, props.version);
  return (
    <div class="flex items-center">
      <Button
        class="h-7 rounded-r-none text-[12px]"
        disabled={props.disabled}
        title={tplan.buttonHint}
        onClick={() => props.onExplain("menu")}
      >
        <ListTree size={13} /> {tplan.button}
        <Show when={props.options.analyze}>
          <span class="text-muted">· ANALYZE</span>
        </Show>
      </Button>
      <DropdownMenu gutter={2} placement="bottom-end">
        <DropdownMenu.Trigger
          as={Button}
          class="h-7 rounded-l-none border-l-0 px-1.5 text-[12px]"
          aria-label={tplan.menu}
          title={tplan.menu}
        >
          <ChevronDown size={13} />
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content class="z-50 min-w-64 rounded-md border border-border bg-bg py-1 shadow-xl outline-none">
            <DropdownMenu.Item class={itemClass} disabled={props.disabled} onSelect={() => props.onExplain("plain")}>
              {tplan.runPlain}
              <span class="ml-auto pl-6 text-xs text-muted">F7</span>
            </DropdownMenu.Item>
            <DropdownMenu.Item class={itemClass} disabled={props.disabled} onSelect={() => props.onExplain("analyze")}>
              {tplan.runAnalyze}
              <span class="ml-auto pl-6 text-xs text-muted">Shift+F7</span>
            </DropdownMenu.Item>
            <DropdownMenu.Separator class="my-1 border-t border-border" />
            <For each={MENU_OPTIONS}>
              {(key) => {
                const reason = () => gates()[key].reason;
                return (
                  <DropdownMenu.CheckboxItem
                    class={itemClass}
                    checked={shown()[key]}
                    disabled={reason() !== null}
                    closeOnSelect={false}
                    title={reason() ?? undefined}
                    onChange={(v: boolean) => props.onOption(key, v)}
                  >
                    <span class="inline-flex w-4 justify-center" aria-hidden="true">
                      <DropdownMenu.ItemIndicator>✓</DropdownMenu.ItemIndicator>
                    </span>
                    <span class="flex flex-col">
                      <span>{tplan.option[key]}</span>
                      <Show when={reason()}>{(r) => <span class="text-xs text-muted">{r()}</span>}</Show>
                    </span>
                  </DropdownMenu.CheckboxItem>
                );
              }}
            </For>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu>
    </div>
  );
}
