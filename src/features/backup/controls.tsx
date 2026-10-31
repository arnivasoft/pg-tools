// Seçenek denetimleri (Plan 27; iki diyalogta ortak). Yalnız çizer: durum `backup-model` kurallarından, değer denetleyiciden gelir.
// Her denetim etiketlidir; devre dışı gerekçesi ve alan hatası `aria-describedby` ile bağlanır.
import { createUniqueId, For, Show, type JSX } from "solid-js";
import { inputClass } from "../../ui/Field";
import type { OptionState } from "./backup-model";

function Notes(props: { id: string; state: OptionState; error: string | undefined }) {
  return (
    <span id={props.id} class="flex flex-col pl-6 text-[11px]">
      <Show when={props.state.reason}>
        <span class={props.state.enabled ? "text-muted" : "text-warn-text"}>{props.state.reason}</span>
      </Show>
      <Show when={props.error}>
        <span class="text-danger">{props.error}</span>
      </Show>
    </span>
  );
}

export function CheckOption(props: {
  label: string;
  checked: boolean;
  state: OptionState;
  error?: string;
  disabled?: boolean;
  /** Seçili değilken gösterilen ek uyarı (ör. tek transaction kapalıyken). */
  note?: string;
  onChange: (v: boolean) => void;
}) {
  const notes = createUniqueId();
  return (
    <div class="flex flex-col">
      <label class="flex items-center gap-2">
        <input
          type="checkbox"
          checked={props.checked}
          disabled={props.disabled || !props.state.enabled}
          aria-describedby={props.state.reason || props.error ? notes : undefined}
          onChange={(e) => props.onChange(e.currentTarget.checked)}
        />
        <span class="font-mono text-[12px]">{props.label}</span>
      </label>
      <Notes id={notes} state={props.state} error={props.error} />
      <Show when={props.note && !props.checked}>
        <span class="pl-6 text-[11px] text-muted">{props.note}</span>
      </Show>
    </div>
  );
}

/** 1–16 gibi sayı alanı; boş = null. Geçersiz ara metin (NaN) yok sayılır. */
export function NumberOption(props: {
  label: string;
  value: number | null;
  state: OptionState;
  error?: string;
  disabled?: boolean;
  onChange: (v: number | null) => void;
}) {
  const id = createUniqueId();
  const notes = createUniqueId();
  return (
    <div class="flex flex-col gap-1">
      <label for={id} class="font-mono text-[12px]">{props.label}</label>
      <input
        id={id}
        type="number"
        min="1"
        max="16"
        step="1"
        class={inputClass}
        style={{ width: "6rem" }}
        value={props.value ?? ""}
        disabled={props.disabled || !props.state.enabled}
        aria-describedby={props.state.reason || props.error ? notes : undefined}
        onInput={(e) => {
          const v = e.currentTarget.value.trim();
          if (v === "") return props.onChange(null);
          const n = Number(v);
          if (Number.isInteger(n)) props.onChange(n);
        }}
      />
      <Notes id={notes} state={props.state} error={props.error} />
    </div>
  );
}

export function SelectOption(props: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  state: OptionState;
  error?: string;
  disabled?: boolean;
  onChange: (v: string) => void;
}) {
  const id = createUniqueId();
  const notes = createUniqueId();
  return (
    <div class="flex flex-col gap-1">
      <label for={id} class="font-mono text-[12px]">{props.label}</label>
      <select
        id={id}
        class={inputClass}
        value={props.value}
        disabled={props.disabled || !props.state.enabled}
        aria-describedby={props.state.reason || props.error ? notes : undefined}
        onChange={(e) => props.onChange(e.currentTarget.value)}
      >
        <For each={props.options}>{(o) => <option value={o.value} selected={o.value === props.value}>{o.label}</option>}</For>
      </select>
      <Notes id={notes} state={props.state} error={props.error} />
    </div>
  );
}

/** Başlıklı seçenek grubu (`fieldset`; çalışırken `disabled`). */
export function Group(props: { legend: string; disabled?: boolean; children: JSX.Element }) {
  return (
    <fieldset class="flex flex-col gap-2" disabled={props.disabled}>
      <legend class="mb-1 text-[12px] text-muted">{props.legend}</legend>
      {props.children}
    </fieldset>
  );
}
