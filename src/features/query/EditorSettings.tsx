import { For } from "solid-js";
import { tedit } from "./strings-editor";
import type { CommaStyle } from "../../ipc/bindings/CommaStyle";
import type { FormatOptions } from "../../ipc/bindings/FormatOptions";
import type { KeywordCase } from "../../ipc/bindings/KeywordCase";

const selectClass =
  "h-7 rounded border border-border bg-bg px-1.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-accent";

/** Satır uzunluğu seçenekleri (arka uç 80–200 kabul eder). */
const MAX_LINES = [80, 100, 120, 160, 200];

/** Ayarlar bölümü "Editör": yazarken denetim + biçimlendirici seçenekleri (`settings.json`). */
export function EditorSettings(props: {
  checkEnabled: boolean;
  format: FormatOptions;
  onChange: (checkEnabled: boolean, format: FormatOptions) => void;
}) {
  const e = tedit;
  const setFormat = (p: Partial<FormatOptions>) => props.onChange(props.checkEnabled, { ...props.format, ...p });
  const field = "flex items-center gap-1.5 text-[12px]";
  return (
    <section class="flex flex-col gap-2" aria-label={e.title}>
      <h3 class="text-[12px] font-semibold text-muted">{e.title}</h3>
      <label class="flex items-center gap-2 text-[13px]">
        <input type="checkbox" checked={props.checkEnabled} onChange={(ev) => props.onChange(ev.currentTarget.checked, props.format)} />
        {e.checkToggle}
      </label>
      <div class="flex flex-wrap items-center gap-3">
        <label class={field}>
          {e.keywordCase}
          <select
            class={selectClass}
            value={props.format.keywordCase}
            onChange={(ev) => setFormat({ keywordCase: ev.currentTarget.value as KeywordCase })}
          >
            <For each={["upper", "lower", "preserve"] as KeywordCase[]}>{(k) => <option value={k}>{e.keywordCases[k]}</option>}</For>
          </select>
        </label>
        <label class={field}>
          {e.indent}
          <select class={selectClass} value={String(props.format.indent)} onChange={(ev) => setFormat({ indent: Number(ev.currentTarget.value) })}>
            <For each={[2, 4]}>{(n) => <option value={String(n)}>{e.indentLabel(n)}</option>}</For>
          </select>
        </label>
        <label class={field}>
          {e.comma}
          <select class={selectClass} value={props.format.comma} onChange={(ev) => setFormat({ comma: ev.currentTarget.value as CommaStyle })}>
            <For each={["trailing", "leading"] as CommaStyle[]}>{(c) => <option value={c}>{e.commas[c]}</option>}</For>
          </select>
        </label>
        <label class={field}>
          {e.maxLine}
          <select class={selectClass} value={String(props.format.maxLine)} onChange={(ev) => setFormat({ maxLine: Number(ev.currentTarget.value) })}>
            <For each={[...new Set([...MAX_LINES, props.format.maxLine])].sort((a, b) => a - b)}>
              {(n) => <option value={String(n)}>{n}</option>}
            </For>
          </select>
        </label>
      </div>
      <p class="text-[11px] text-muted">{e.formatHint}</p>
    </section>
  );
}
