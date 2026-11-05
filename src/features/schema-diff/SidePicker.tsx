// Bir karşılaştırma tarafının (Kaynak/Hedef) seçicileri: sunucu, veritabanı, şema. Yalnız çizer; liste yükleme, kuşak ve
// sıfırlama kuralları denetleyicidedir. Liste yokken seçici devre dışıdır; yüklenirken "Yükleniyor…" seçeneği görünür.
import { For, Show, createUniqueId, type JSX } from "solid-js";
import { tsd } from "../../app/strings-diff";
import type { SideTag } from "../../ipc/bindings/SideTag";
import { inputClass } from "../../ui/Field";
import type { DiffController } from "./diff-open";
import { targetBanner } from "./diff-model";

function Select(props: {
  label: string;
  value: string;
  disabled: boolean;
  loading: boolean;
  error: string | null;
  onChange: (v: string) => void;
  children: JSX.Element;
}) {
  const id = createUniqueId();
  const err = createUniqueId();
  return (
    <div class="flex min-w-0 flex-1 flex-col gap-0.5">
      <label for={id} class="text-[12px] text-muted">
        {props.label}
      </label>
      <select
        id={id}
        class={inputClass}
        value={props.value}
        disabled={props.disabled}
        aria-busy={props.loading}
        aria-invalid={props.error !== null}
        aria-describedby={props.error !== null ? err : undefined}
        onChange={(e) => props.onChange(e.currentTarget.value)}
      >
        <Show when={props.loading && props.disabled} fallback={<option value="">{tsd.choose}</option>}>
          <option value="">{tsd.loading}</option>
        </Show>
        {props.children}
      </select>
      <Show when={props.error}>
        {(text) => (
          <p id={err} role="alert" class="text-[12px] text-danger">
            {text()}
          </p>
        )}
      </Show>
    </div>
  );
}

export function SidePicker(props: { diff: DiffController; tabId: string; side: SideTag; title: string }) {
  const s = () => props.diff.model(props.tabId)?.[props.side];
  const loading = () => s()?.loadingLists ?? false;
  return (
    <fieldset class="flex min-w-0 flex-1 flex-col gap-1.5 rounded border border-border p-2">
      <legend class="px-1 text-[12px] font-semibold">{props.title}</legend>
      <Select
        label={tsd.server}
        value={s()?.serverId ?? ""}
        disabled={props.diff.servers().length === 0}
        loading={false}
        error={null}
        onChange={(v) => void props.diff.setServer(props.tabId, props.side, v === "" ? null : v)}
      >
        <For each={props.diff.servers()}>
          {(cfg) => {
            const b = () => targetBanner(cfg);
            return (
              <option value={cfg.id} selected={cfg.id === s()?.serverId}>
                {[cfg.name, b().production ? tsd.production : "", b().locked ? tsd.locked : ""].filter((x) => x).join(" ")}
              </option>
            );
          }}
        </For>
      </Select>
      <Select
        label={tsd.database}
        value={s()?.databaseOid == null ? "" : String(s()?.databaseOid)}
        disabled={!s()?.databases}
        loading={loading()}
        error={s()?.databasesError ?? null}
        onChange={(v) => void props.diff.setDatabase(props.tabId, props.side, v === "" ? null : Number(v))}
      >
        <For each={s()?.databases ?? []}>
          {(d) => (
            <option value={String(d.oid)} selected={d.oid === s()?.databaseOid}>
              {d.name}
            </option>
          )}
        </For>
      </Select>
      <Select
        label={tsd.schema}
        value={s()?.schema ?? ""}
        disabled={!s()?.schemas}
        loading={loading()}
        error={s()?.schemasError ?? null}
        onChange={(v) => props.diff.setSchema(props.tabId, props.side, v === "" ? null : v)}
      >
        <For each={s()?.schemas ?? []}>
          {(name) => (
            <option value={name} selected={name === s()?.schema}>
              {name}
            </option>
          )}
        </For>
      </Select>
    </fieldset>
  );
}
