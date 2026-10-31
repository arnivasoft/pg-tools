// "Plan" sekmesi (tembel QueryTab parçası): girintili plan ağacı (WAI-ARIA tree), en pahalı 5 düğüm, özet, seçili düğüm
// ayrıntısı ve "Ham JSON". Hesaplar `plan-model`, gezinti/biçim `plan-view-model`dedir; bu bileşen incedir.
// Ana paket modüllerini (`app/*`, `explorer/*`) içe aktarmaz (pano için doğrudan `navigator.clipboard`).
import { ChevronDown, ChevronRight } from "lucide-solid";
import { For, Show, createMemo, createSignal, onCleanup } from "solid-js";
import { Button } from "../../ui/Button";
import type { PlanEntry } from "./explain-controller";
import { hotNodes, initialExpanded, walkPlan, type PlanBuffers, type PlanNode } from "./plan-model";
import {
  ancestorsOf,
  barRatio,
  detailText,
  findNode,
  fmtBuffers,
  fmtCost,
  fmtMs,
  fmtNum,
  fmtPercent,
  fmtRows,
  misestimateText,
  nodeNotes,
  summaryNotes,
  treeKey,
  visibleRows,
} from "./plan-view-model";
import { tplan } from "./strings-plan";

export function PlanView(props: { entry: PlanEntry | null }) {
  return (
    <Show when={props.entry} keyed fallback={<p class="p-3 text-[13px] text-muted">{tplan.empty}</p>}>
      {(entry) => <PlanBody entry={entry} />}
    </Show>
  );
}

const BUFFER_LABELS: [keyof PlanBuffers, string][] = [
  ["sharedHit", "Shared Hit"],
  ["sharedRead", "Shared Read"],
  ["sharedDirtied", "Shared Dirtied"],
  ["sharedWritten", "Shared Written"],
  ["localHit", "Local Hit"],
  ["localRead", "Local Read"],
  ["localDirtied", "Local Dirtied"],
  ["localWritten", "Local Written"],
  ["tempRead", "Temp Read"],
  ["tempWritten", "Temp Written"],
];

const badgeClass = "rounded px-1 text-[11px] font-medium";
const sr = (s: string) => <span class="sr-only">{s}: </span>;

function PlanBody(props: { entry: PlanEntry }) {
  const e = props.entry;
  const root = e.root;
  const timed = root.inclusiveMs !== undefined;
  const uid = `plan-${e.key}`;
  const [view, setView] = createSignal<"tree" | "raw">("tree");
  const [expanded, setExpanded] = createSignal<ReadonlySet<number>>(initialExpanded(root));
  const [active, setActive] = createSignal(root.id);
  const [selected, setSelected] = createSignal<number>(root.id);
  const rows = createMemo(() => visibleRows(root, expanded()));
  const hot = hotNodes(root, 5);
  const notes = summaryNotes(root);
  const selectedNode = createMemo(() => findNode(root, selected()));
  // Derinlik değişmez: <For> düğüm kimliğiyle (kararlı nesne) çalışsın, aç/kapa tüm satırları yeniden çizmesin.
  const depth = new Map<number, number>();
  walkPlan(root, (n, d) => depth.set(n.id, d));
  const shown = createMemo(() => rows().map((r) => r.node));
  let detailEl: HTMLElement | undefined;

  const itemId = (id: number) => `${uid}-n${id}`;
  const focusItem = (id: number) =>
    requestAnimationFrame(() => {
      const el = document.getElementById(itemId(id));
      el?.focus();
      el?.scrollIntoView({ block: "nearest" });
    });
  const toggle = (id: number) => {
    const next = new Set(expanded());
    if (next.has(id)) {
      next.delete(id);
      // Daraltılan düğümün altında kalan etkin düğüm görünmez olur: etkinlik daraltılan düğüme geçer.
      if ((ancestorsOf(root, active()) ?? []).includes(id)) setActive(id);
    } else next.add(id);
    setExpanded(next);
  };
  const reveal = (id: number) => {
    const path = ancestorsOf(root, id) ?? [];
    if (path.some((p) => !expanded().has(p))) setExpanded(new Set([...expanded(), ...path]));
    setActive(id);
    setSelected(id);
    focusItem(id);
  };

  const onTreeKey = (ev: KeyboardEvent) => {
    if (ev.isComposing || ev.keyCode === 229 || ev.altKey || ev.ctrlKey || ev.metaKey) return; // IME koruması
    const r = treeKey(ev.key, rows(), active(), expanded());
    if (!r) return;
    ev.preventDefault();
    if (r.expand !== undefined || r.collapse !== undefined) toggle((r.expand ?? r.collapse)!);
    setActive(r.active);
    if (r.open !== undefined) {
      setSelected(r.open);
      requestAnimationFrame(() => detailEl?.focus());
      return;
    }
    focusItem(r.active);
  };

  const [copied, setCopied] = createSignal(false);
  let timer: number | undefined;
  onCleanup(() => window.clearTimeout(timer));
  const copy = () => {
    void navigator.clipboard
      .writeText(e.json)
      .then(() => {
        setCopied(true);
        window.clearTimeout(timer);
        timer = window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };

  const tabButton = (v: "tree" | "raw", label: string) => (
    <button
      type="button"
      role="tab"
      id={`${uid}-tab-${v}`}
      aria-controls={`${uid}-panel`}
      aria-selected={view() === v}
      class="border-b-2 border-transparent px-2 py-1 text-[12px] text-muted aria-selected:border-accent aria-selected:text-fg"
      onClick={() => setView(v)}
    >
      {label}
    </button>
  );

  const summaryParts = () => {
    const s = e.summary;
    const out: string[] = [];
    if (s.planningMs !== undefined) out.push(`${tplan.planning}: ${fmtMs(s.planningMs)}`);
    if (s.executionMs !== undefined) out.push(`${tplan.execution}: ${fmtMs(s.executionMs)}`);
    if (timed && s.totalMs !== undefined && s.executionMs === undefined) out.push(`${tplan.total}: ${fmtMs(s.totalMs)}`);
    if (e.elapsedMs !== null) out.push(`${tplan.elapsed}: ${fmtMs(e.elapsedMs)}`);
    return out;
  };

  return (
    <div class="flex h-full min-h-0 flex-col text-[12px]">
      <div class="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-2">
        <div role="tablist" aria-label={tplan.viewTabs} class="flex gap-1">
          {tabButton("tree", tplan.treeTab)}
          {tabButton("raw", tplan.rawTab)}
        </div>
        <span class="text-muted">{summaryParts().join(" · ")}</span>
        <Show when={e.rolledBack}>
          <span class={`${badgeClass} border border-border`}>{tplan.rolledBack}</span>
        </Show>
        <Show when={e.source === "grid"}>
          <span class="text-muted">{tplan.fromGrid}</span>
        </Show>
      </div>
      <div class="min-h-0 flex-1" role="tabpanel" id={`${uid}-panel`} aria-labelledby={`${uid}-tab-${view()}`}>
        <Show
          when={view() === "tree"}
          fallback={
            <div class="flex h-full min-h-0 flex-col">
              <div class="flex shrink-0 justify-end border-b border-border p-1">
                <Button class="h-6 text-[12px]" onClick={copy}>
                  {copied() ? tplan.copied : tplan.copy}
                </Button>
              </div>
              <pre
                tabindex="0"
                aria-label={tplan.rawTab}
                class="min-h-0 flex-1 overflow-auto whitespace-pre p-2 font-mono text-[12px]"
              >
                {e.json}
              </pre>
            </div>
          }
        >
          <div class="flex h-full min-h-0">
            <div
              role="tree"
              aria-label={tplan.treeAria}
              class="min-w-0 flex-1 overflow-auto py-1 font-mono"
              onKeyDown={onTreeKey}
            >
              <For each={shown()}>
                {(n) => {
                  const level = depth.get(n.id) ?? 0;
                  const hasKids = n.children.length > 0;
                  const ratio = barRatio(n, root, e.summary);
                  const mis = misestimateText(n);
                  const buf = fmtBuffers(n.buffers);
                  const never = n.actual?.loops === 0;
                  return (
                    <div
                      id={itemId(n.id)}
                      role="treeitem"
                      aria-level={level + 1}
                      aria-expanded={hasKids ? expanded().has(n.id) : undefined}
                      aria-selected={selected() === n.id}
                      tabindex={active() === n.id ? 0 : -1}
                      class="flex cursor-default items-center gap-2 whitespace-nowrap px-2 py-0.5 outline-none hover:bg-hover aria-selected:bg-selected focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent"
                      onClick={() => {
                        setActive(n.id);
                        setSelected(n.id);
                      }}
                    >
                      <span class="flex shrink-0 items-center" style={{ "padding-left": `${level * 14}px` }}>
                        <span
                          aria-hidden="true"
                          class="inline-flex w-4 justify-center text-muted"
                          onClick={(ev) => {
                            if (!hasKids) return;
                            ev.stopPropagation();
                            setActive(n.id);
                            toggle(n.id);
                          }}
                        >
                          <Show when={hasKids}>{expanded().has(n.id) ? <ChevronDown size={12} /> : <ChevronRight size={12} />}</Show>
                        </span>
                      </span>
                      <span class="min-w-0 flex-1 truncate font-sans" classList={{ "text-muted": never }} title={n.label}>
                        {n.label}
                        <Show when={never}> ({tplan.neverExecuted})</Show>
                      </span>
                      <Show when={ratio !== null}>
                        <span aria-hidden="true" class="h-2 w-20 shrink-0 overflow-hidden rounded bg-hover">
                          <span
                            class="block h-full"
                            classList={{ "bg-danger": ratio! >= 0.5, "bg-accent": ratio! < 0.5 }}
                            style={{ width: `${Math.max(2, ratio! * 100)}%` }}
                          />
                        </span>
                      </Show>
                      <span class="w-32 shrink-0 text-right">
                        {sr(timed ? tplan.col.time : tplan.col.exclusiveCost)}
                        {timed ? tplan.rowTime(fmtMs(n.exclusiveMs), fmtPercent(ratio)) : `${fmtNum(n.exclusiveCost)} (${fmtPercent(ratio)})`}
                      </span>
                      <span class="w-36 shrink-0 text-right text-muted">
                        {sr(tplan.col.cost)}
                        {fmtCost(n)}
                      </span>
                      <span class="w-32 shrink-0 text-right">
                        {sr(tplan.col.rows)}
                        {fmtRows(n)}
                      </span>
                      <span class="w-28 shrink-0">
                        <Show when={mis}>
                          {(t) => (
                            <span
                              class={badgeClass}
                              classList={{
                                "bg-danger text-danger-fg": n.misestimate?.level === "crit",
                                "border border-[color:var(--srv-amber)] text-warn-text": n.misestimate?.level === "warn",
                              }}
                            >
                              {t()}
                            </span>
                          )}
                        </Show>
                      </span>
                      <span class="w-20 shrink-0 text-muted">
                        <Show when={n.actual && n.actual.loops !== 1}>{tplan.loops(fmtNum(n.actual!.loops, 0))}</Show>
                      </span>
                      <span class="w-36 shrink-0 text-muted">
                        <Show when={buf}>
                          {sr(tplan.col.buffers)}
                          {buf}
                        </Show>
                      </span>
                    </div>
                  );
                }}
              </For>
            </div>
            <aside class="flex w-80 shrink-0 flex-col overflow-auto border-l border-border">
              <section class="border-b border-border p-2" aria-labelledby={`${uid}-hot`}>
                <h3 id={`${uid}-hot`} class="mb-1 font-semibold">
                  {tplan.hot} <span class="font-normal text-muted">({timed ? tplan.hotByTime : tplan.hotByCost})</span>
                </h3>
                <ol class="flex flex-col gap-0.5">
                  <For each={hot}>
                    {(n) => (
                      <li>
                        <button
                          type="button"
                          class="flex w-full items-center gap-2 rounded px-1 py-0.5 text-left hover:bg-hover"
                          classList={{ "bg-selected": selected() === n.id }}
                          onClick={() => reveal(n.id)}
                        >
                          <span class="min-w-0 flex-1 truncate">{n.label}</span>
                          <span class="shrink-0 text-muted">{timed ? fmtMs(n.exclusiveMs) : fmtNum(n.exclusiveCost)}</span>
                        </button>
                      </li>
                    )}
                  </For>
                </ol>
              </section>
              <Summary entry={e} notes={notes} />
              <section
                ref={detailEl}
                tabindex="-1"
                // Enter ayrıntıya getirir; Esc ağaçtaki etkin düğüme döndürür.
                onKeyDown={(ev) => {
                  if (ev.key === "Escape" && !ev.isComposing) {
                    ev.preventDefault();
                    focusItem(active());
                  }
                }}
                class="p-2 outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent"
                aria-labelledby={`${uid}-detail`}
              >
                <h3 id={`${uid}-detail`} class="mb-1 font-semibold">
                  {tplan.detail}
                </h3>
                <Show when={selectedNode()} fallback={<p class="text-muted">{tplan.detailEmpty}</p>}>
                  {(n) => <NodeDetail root={root} node={n()} />}
                </Show>
              </section>
            </aside>
          </div>
        </Show>
      </div>
    </div>
  );
}

function Summary(props: { entry: PlanEntry; notes: string[] }) {
  const s = props.entry.summary;
  const has = () => s.triggers.length > 0 || s.jit || (s.settings?.length ?? 0) > 0 || props.notes.length > 0 || s.details.length > 0;
  return (
    <Show when={has()}>
      <section class="border-b border-border p-2">
        <h3 class="mb-1 font-semibold">{tplan.summary}</h3>
        <For each={props.notes}>{(n) => <p class="mb-1 text-warn-text">{n}</p>}</For>
        <Show when={s.triggers.length > 0}>
          <p class="font-medium">{tplan.triggers}</p>
          <ul class="mb-1 pl-2">
            <For each={s.triggers}>
              {(t) => <li>{tplan.trigger(t.relation ? `${t.name} (${t.relation})` : t.name, fmtMs(t.timeMs), fmtNum(t.calls, 0))}</li>}
            </For>
          </ul>
        </Show>
        <Show when={s.jit}>{(j) => <p class="mb-1">{tplan.jit(fmtNum(j().functions, 0), fmtMs(j().totalMs))}</p>}</Show>
        <Show when={(s.settings?.length ?? 0) > 0}>
          <p class="font-medium">{tplan.settings}</p>
          <dl class="mb-1 pl-2">
            <For each={s.settings}>
              {([k, v]) => (
                <div class="flex gap-1">
                  <dt class="text-muted">{k}:</dt>
                  <dd class="break-all">{v}</dd>
                </div>
              )}
            </For>
          </dl>
        </Show>
        <Show when={s.details.length > 0}>
          <p class="font-medium">{tplan.otherKeys}</p>
          <dl class="pl-2">
            <For each={s.details}>
              {([k, v]) => (
                <div class="flex gap-1">
                  <dt class="shrink-0 text-muted">{k}:</dt>
                  <dd class="break-all">{detailText(v)}</dd>
                </div>
              )}
            </For>
          </dl>
        </Show>
      </section>
    </Show>
  );
}

function NodeDetail(props: { root: PlanNode; node: PlanNode }) {
  const pairs = createMemo(() => {
    const n = props.node;
    const c = tplan.col;
    const out: [string, string][] = [["Node Type", n.nodeType]];
    if (n.totalCost !== null) out.push([c.cost, fmtCost(n)], [c.exclusiveCost, fmtNum(n.exclusiveCost)]);
    else out.push([c.cost, tplan.noCosts]);
    out.push([c.rows, fmtRows(n)]);
    if (n.planWidth !== null) out.push([c.width, fmtNum(n.planWidth, 0)]);
    if (n.actual) {
      out.push([c.loops, fmtNum(n.actual.loops, 0)], [c.startup, fmtMs(n.actual.startup)], [c.actualTotal, fmtMs(n.actual.total)]);
      out.push([c.inclusive, fmtMs(n.inclusiveMs)], [c.time, fmtMs(n.exclusiveMs)]);
    }
    const mis = misestimateText(n);
    if (mis) out.push(["", mis]);
    if (n.buffers) {
      for (const [k, label] of BUFFER_LABELS) {
        const v = n.buffers[k];
        if (v !== 0) out.push([`${c.buffers} ${label}`, fmtNum(v, 0)]);
      }
    }
    for (const [k, v] of n.details) out.push([k, detailText(v)]);
    return out;
  });
  return (
    <div>
      <p class="mb-1 break-all font-medium">{props.node.label}</p>
      <For each={nodeNotes(props.root, props.node)}>{(n) => <p class="mb-1 text-warn-text">{n}</p>}</For>
      <dl class="font-mono">
        <For each={pairs()}>
          {([k, v]) => (
            <div class="flex gap-1 py-px">
              <Show when={k}>
                <dt class="shrink-0 text-muted">{k}:</dt>
              </Show>
              <dd class="min-w-0 whitespace-pre-wrap break-all">{v}</dd>
            </div>
          )}
        </For>
      </dl>
    </div>
  );
}
