// Betik önizleme diyaloğu (SchemaDiffView'dan tembel parça): hedef bandı, risk, ifade sayısı, transaction bilgisi,
// uyarılar ve salt okunur CodeMirror. Eylemler denetleyicidedir (Query Tool'da aç / Kopyala / Kaydet… / Kapat). Ayrıca
// denetleyicinin kaydetme seçicisini (`picker`) verir: `@tauri-apps/plugin-dialog` yalnız bu parçaya girer.
import { save } from "@tauri-apps/plugin-dialog";
import { For, Show, onCleanup, onMount } from "solid-js";
import { tsd } from "../../app/strings-diff";
import type { DiffScript } from "../../ipc/bindings/DiffScript";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { RiskBadge } from "../designer/RiskBadge";
import { createReadOnlyEditor } from "../inspector/codemirror-setup";
import type { Picker } from "./diff-controller";
import { targetBanner } from "./diff-model";
import type { DiffController } from "./diff-open";

export const picker: Picker = {
  save: async (o) => {
    const path: unknown = await save(o);
    return typeof path === "string" ? path : null;
  },
};

// Metin bir kez okunur: diyalog kipli, açıkken betik yeniden üretilemez (`buildScript` arkada çalışamaz).
function Editor(props: { sql: string }) {
  let host!: HTMLDivElement;
  onMount(() => {
    const view = createReadOnlyEditor(host, props.sql);
    onCleanup(() => view.destroy());
  });
  return <div ref={host} class="overflow-hidden rounded border border-border [&_.cm-editor]:h-full" style={{ height: "60vh" }} />;
}

function Content(props: { diff: DiffController; data: DiffScript; targetServerId: string | null }) {
  const banner = () => targetBanner(props.diff.servers().find((s) => s.id === props.targetServerId) ?? null);
  return (
    <div class="flex flex-col gap-2 text-[13px]">
      <Show when={banner().production || banner().locked}>
        <div
          role="status"
          class="flex flex-col gap-0.5 rounded border px-2 py-1 text-[12px]"
          classList={{ "border-danger": banner().production, "border-[color:var(--srv-amber)]": !banner().production }}
        >
          <Show when={banner().production}>
            <p class="font-semibold text-danger">{tsd.script.production(banner().name ?? "")}</p>
          </Show>
          <Show when={banner().locked}>
            <p class="text-warn-text">{tsd.script.locked}</p>
          </Show>
        </div>
      </Show>
      <div class="flex flex-wrap items-center gap-3">
        <RiskBadge risk={props.data.risk} />
        <span>{tsd.script.statements(props.data.statements)}</span>
        <span classList={{ "text-muted": props.data.transactional, "text-warn-text": !props.data.transactional }}>
          {props.data.transactional ? tsd.script.tx : tsd.script.noTx}
        </span>
      </div>
      <Show when={props.data.warnings.length > 0}>
        <div class="rounded border border-[color:var(--srv-amber)] px-2 py-1 text-[12px] text-warn-text">
          <p class="font-semibold">{tsd.warnings}</p>
          <ul class="list-disc pl-5">
            <For each={props.data.warnings}>{(w) => <li>{w}</li>}</For>
          </ul>
        </div>
      </Show>
      <Editor sql={props.data.sql} />
    </div>
  );
}

export default function ScriptDialog(props: { diff: DiffController; tabId: string }) {
  const d = props.diff;
  const s = () => d.model(props.tabId)?.script ?? null;
  const data = () => s()?.data ?? null;
  const targetServerId = () => d.model(props.tabId)?.target.serverId ?? null;
  const close = () => d.closeScript(props.tabId);
  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o) close();
      }}
      title={tsd.script.title}
      width="880px"
      footer={
        <>
          <Button onClick={close}>{tsd.close}</Button>
          <Button disabled={data() === null} onClick={() => void d.saveScript(props.tabId)}>
            {tsd.save}
          </Button>
          <Button disabled={data() === null} onClick={() => void d.copyScript(props.tabId)}>
            {tsd.copy}
          </Button>
          <Button variant="primary" disabled={data() === null} onClick={() => void d.openInQueryTool(props.tabId)}>
            {tsd.openInQuery}
          </Button>
        </>
      }
    >
      <Show when={s()?.error}>
        {(e) => (
          <p role="alert" class="mb-2 text-[12px] text-danger">
            {e()}
          </p>
        )}
      </Show>
      <Show when={data()} fallback={<Show when={s()?.busy}><p class="text-muted">{tsd.loading}</p></Show>}>
        {(x) => <Content diff={d} data={x()} targetServerId={targetServerId()} />}
      </Show>
    </Dialog>
  );
}
