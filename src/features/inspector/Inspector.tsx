import { Copy } from "lucide-solid";
import { Match, Show, Suspense, Switch, createResource, createSignal, lazy, onCleanup } from "solid-js";
import { copyText } from "../../app/clipboard";
import { createDebounced } from "../../app/debounce";
import { t } from "../../app/strings";
import type { ErrorPayload } from "../../ipc/bindings/ErrorPayload";
import { commands } from "../../ipc/commands";
import { toPayload } from "../../ipc/errors";
import { Button } from "../../ui/Button";
import { TabStrip, TabStripPanel } from "../../ui/TabStrip";
import type { Explorer } from "../explorer/explorer-store";
import { forNode } from "./for-node";
import { kindLabel } from "./labels";
import { PropertiesView } from "./PropertiesView";

const DdlView = lazy(() => import("./DdlView"));

function ErrorView(props: { error: unknown }) {
  const payload = (): ErrorPayload => toPayload(props.error);
  return (
    <div class="p-3 text-danger">
      <p>{payload().message}</p>
      <Show when={payload().detail}>
        <p class="mt-1 text-xs opacity-80">{payload().detail}</p>
      </Show>
    </div>
  );
}

const Muted = (props: { text: string }) => <p class="p-3 text-muted">{props.text}</p>;

export function Inspector(props: { explorer: Explorer }) {
  const [tab, setTab] = createSignal("properties");
  const [copied, setCopied] = createSignal(false);
  const selected = createDebounced(() => props.explorer.selectedNode(), 150);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(copiedTimer));

  // Backend çağrısı oturum kaybıyla başarısız olursa ağaç da sunucuyu bağlantısız göstermeli.
  const guarded = async <T,>(id: string, call: () => Promise<T>): Promise<T> => {
    try {
      return await call();
    } catch (e) {
      const payload = toPayload(e);
      if (!props.explorer.controller.sessionLost(id, payload)) props.explorer.controller.nodeGone(id, payload);
      throw e;
    }
  };

  const [properties] = createResource(
    () => selected()?.id,
    async (id) => ({ id, value: await guarded(id, () => commands.getProperties(id)) }),
  );
  const [ddl] = createResource(
    () => (tab() === "ddl" ? selected()?.id : undefined),
    async (id) => ({ id, value: await guarded(id, () => commands.getDdl(id)) }),
  );

  // Hatalı bir resource'un `.latest` değeri okunursa Solid hatayı fırlatır; bu yüzden önce `.error` kontrol edilir.
  // Değer yalnızca şu an seçili düğümü üreten istekten geliyorsa geçerlidir (aksi halde "yükleniyor").
  const propsData = () => forNode(properties.error ? undefined : properties.latest, selected()?.id);
  const ddlText = () => forNode(ddl.error ? undefined : ddl.latest, selected()?.id);

  const copy = async () => {
    const text = ddlText();
    if (!text) return;
    try {
      await copyText(text);
    } catch {
      return;
    }
    setCopied(true);
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => setCopied(false), 1500);
  };

  return (
    <Show when={selected()} fallback={<Muted text={t.inspector.nothingSelected} />}>
      {(node) => (
        <div class="flex h-full flex-col">
          <div class="flex h-9 shrink-0 items-center gap-2 border-b border-border px-3">
            <span class="min-w-0 truncate font-semibold" title={node().label}>{node().label}</span>
            <span class="shrink-0 whitespace-nowrap text-muted">· {kindLabel(node().kind)}</span>
            <div class="flex-1" />
            <Show when={tab() === "ddl" && ddlText()}>
              <Button variant="ghost" onClick={() => void copy()}>
                <Copy size={14} /> {copied() ? t.inspector.copied : t.inspector.copy}
              </Button>
            </Show>
          </div>
          <TabStrip
            value={tab()}
            onChange={setTab}
            class="min-h-0 flex-1"
            tabs={[
              { value: "properties", label: t.inspector.properties },
              { value: "ddl", label: t.inspector.ddl },
            ]}
          >
            <TabStripPanel value="properties" class="min-h-0 flex-1 overflow-auto">
              <Switch>
                <Match when={properties.error}>
                  <ErrorView error={properties.error} />
                </Match>
                <Match when={propsData() === undefined}>
                  <Muted text={t.inspector.loading} />
                </Match>
                <Match when={propsData()?.length === 0}>
                  <Muted text={t.inspector.noProperties} />
                </Match>
                <Match when={propsData()}>{(sections) => <PropertiesView sections={sections()} />}</Match>
              </Switch>
            </TabStripPanel>
            <TabStripPanel value="ddl" class="relative min-h-0 flex-1">
              <Switch>
                <Match when={ddl.error}>
                  <ErrorView error={ddl.error} />
                </Match>
                <Match when={ddlText() === undefined}>
                  <Muted text={t.inspector.loading} />
                </Match>
                <Match when={ddlText() === ""}>
                  <Muted text={t.inspector.noDdl} />
                </Match>
                <Match when={ddlText()}>
                  {(text) => (
                    <Suspense fallback={<Muted text={t.inspector.loading} />}>
                      <DdlView text={text()} />
                    </Suspense>
                  )}
                </Match>
              </Switch>
            </TabStripPanel>
          </TabStrip>
        </div>
      )}
    </Show>
  );
}
