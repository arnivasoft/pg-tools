// Kilitler bölümü: üstte engelleme ağacı (`buildLockTree`, iç içe liste), altta düz kilit tablosu. Veri `model.locks`
// alanından gelir. Ağaç düğümü eylemi için sinyal satırı `model.activity` içindeki gerçek satırdır; yoksa `BlockSession`'dan
// türetilen asgari satır kullanılır (`resolveSignalRow`: backendType bilinmediği için karşılaştırılmaz; pid/kullanıcı/
// uygulama onay anında yine doğrulanır).
import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { td } from "../../app/strings-dashboard";
import type { LockRow } from "../../ipc/bindings/LockRow";
import { DataTable, type DataColumn } from "./DataTable";
import type { SectionProps } from "./DashboardView";
import { formatDuration } from "./dash-model";
import { buildLockTree, countWaiters, type LockNode } from "./lock-tree";
import { Banner, SignalButtons, StateBadge } from "./parts";
import { resolveSignalRow } from "./signal-model";
import { oneLine } from "./table-model";

type FlatLock = LockRow & { id: number };

function Badge(props: { children: JSX.Element; title?: string; danger?: boolean }) {
  return (
    <span
      title={props.title}
      class="inline-flex h-[18px] items-center rounded-full border px-1.5 text-[11px]"
      classList={{ "border-danger text-danger": props.danger, "border-border text-muted": !props.danger }}
    >
      {props.children}
    </span>
  );
}

function TreeNode(props: SectionProps & { node: LockNode; depth: number }) {
  const l = td.locks;
  const node = () => props.node;
  const waiters = () => countWaiters(node());
  const row = () => {
    const s = node().session;
    return s ? resolveSignalRow(s, props.model.activity?.rows) : null;
  };
  return (
    <li class="flex flex-col">
      <div class="flex min-h-[28px] flex-wrap items-center gap-x-2 gap-y-0.5 rounded px-2 py-0.5 text-[12px] hover:bg-hover">
        <Show
          when={node().kind === "session"}
          fallback={
            <>
              <span class="font-medium">{l.prepared}</span>
              <span class="text-muted">{l.preparedNote}</span>
            </>
          }
        >
          <Show
            when={node().session}
            fallback={
              <>
                <span class="font-mono">pid {node().pid}</span>
                <span class="text-muted">{l.missing}</span>
              </>
            }
          >
            {(s) => (
              <>
                <span class="font-mono font-medium">pid {s().pid}</span>
                <span>{s().user ?? td.sessions.unknown}</span>
                <span class="text-muted">{s().application}</span>
                <StateBadge state={s().state} />
                <Show when={props.depth > 0 && s().waitSecs != null}>
                  <span class="text-muted">{l.waited(formatDuration(s().waitSecs))}</span>
                </Show>
                <Show when={s().query != null}>
                  <span class="min-w-0 max-w-[40ch] truncate font-mono text-muted" title={s().query ?? ""}>
                    {oneLine(s().query, 120)}
                  </span>
                </Show>
                <Show when={s().query == null}>
                  <span class="italic text-muted">{td.noPrivilege}</span>
                </Show>
              </>
            )}
          </Show>
        </Show>
        <Show when={node().cycle}>
          <Badge danger title={l.cycleTitle}>
            {l.cycle}
          </Badge>
        </Show>
        <Show when={waiters() > 0}>
          <Badge>{l.waiters(waiters())}</Badge>
        </Show>
        <Show when={row()}>
          {(r) => (
            <span class="ml-auto">
              <SignalButtons dash={props.dash} tabId={props.model.id} row={r()} signaling={props.model.signaling} />
            </span>
          )}
        </Show>
      </div>
      <Show when={node().children.length > 0}>
        <ul class="ml-4 flex flex-col border-l border-border pl-2">
          <For each={node().children}>
            {(c) => <TreeNode dash={props.dash} model={props.model} node={c} depth={props.depth + 1} />}
          </For>
        </ul>
      </Show>
    </li>
  );
}

export default function LocksView(props: SectionProps) {
  const l = td.locks;
  const [selected, setSelected] = createSignal<number | null>(null);
  const report = () => props.model.locks;
  const tree = createMemo(() => buildLockTree(report()?.sessions ?? []));
  const flat = createMemo<FlatLock[]>(() => (report()?.locks ?? []).map((r, id) => ({ ...r, id })));

  const columns: DataColumn<FlatLock>[] = [
    { key: "pid", header: l.cols.pid, width: 70, align: "right", cell: (r) => r.pid },
    { key: "locktype", header: l.cols.locktype, width: 130, cell: (r) => r.locktype },
    { key: "mode", header: l.cols.mode, width: 190, cell: (r) => r.mode },
    {
      key: "granted", header: l.cols.granted, width: 80,
      cell: (r) =>
        r.granted ? l.yes : (
          <span class="text-danger" title={l.notGrantedTitle}>
            {l.no}
          </span>
        ),
    },
    { key: "object", header: l.cols.object, width: 280, cell: (r) => r.object ?? "" },
  ];

  return (
    <div class="flex h-full min-h-0 flex-col">
      <Show when={props.model.sectionError.locks}>{(e) => <Banner kind="error">{e()}</Banner>}</Show>
      <Show when={report()?.truncated}>
        <Banner kind="info">{l.truncated}</Banner>
      </Show>
      <Show when={report()} fallback={<p class="p-3 text-muted">{props.model.sectionError.locks ? "" : l.waiting}</p>}>
        <div class="max-h-[50%] shrink-0 overflow-auto border-b border-border p-2">
          <Show when={tree().length > 0} fallback={<p class="p-1 text-[13px] text-muted">{l.empty}</p>}>
            <ul aria-label={l.treeLabel} class="flex flex-col gap-0.5">
              <For each={tree()}>{(n) => <TreeNode dash={props.dash} model={props.model} node={n} depth={0} />}</For>
            </ul>
          </Show>
        </div>
        <Show when={flat().length > 0} fallback={<p class="p-3 text-muted">{l.flatEmpty}</p>}>
          <DataTable
            class="min-h-[120px] flex-1"
            ariaLabel={l.tableLabel}
            columns={columns}
            rows={flat()}
            rowKey={(r) => r.id}
            selectedKey={selected()}
            onSelect={(k) => setSelected(k as number | null)}
          />
        </Show>
      </Show>
    </div>
  );
}
