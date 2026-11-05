// Şema karşılaştırma sekmesinin görünümü (DiffTabs'tan tembel parça). Üst bölüm sabittir (seçiciler, seçenekler,
// Karşılaştır/İptal, ilerleme, şeritler, özet); altında yatay bölünmüş liste (%45) | ayrıntı (%55). Görünüm yalnız
// çizer: kapılar ve durum denetleyicidedir (`diff-model` kapı gerekçeleri). Betik diyaloğu ayrı tembel parçadır.
import { ArrowLeftRight } from "lucide-solid";
import { For, Show, Suspense, createEffect, createMemo, createUniqueId, lazy, type JSX } from "solid-js";
import { tsd } from "../../app/strings-diff";
import type { DiffOptions } from "../../ipc/bindings/DiffOptions";
import { Button } from "../../ui/Button";
import { inputClass } from "../../ui/Field";
import type { DiffModel } from "./diff-controller";
import { DiffList } from "./DiffList";
import {
  CATEGORIES, categoryOf, compareBlocked, counts, foldSearch, itemLabel, kindLabel, problemText, progressView,
  scriptBlocked, selectAllState, selectedKeys, selectionSummary, visibleItems,
} from "./diff-model";
import type { DiffController } from "./diff-open";
import { LineDiffView } from "./LineDiffView";
import { SidePicker } from "./SidePicker";

const ScriptDialog = lazy(() => import("./ScriptDialog"));

/** Devre dışı düğme + görünür gerekçe (`aria-describedby`; `title` fare için). */
function GatedButton(props: {
  label: string;
  reason: string | null;
  primary?: boolean;
  onClick: () => void;
}) {
  const id = createUniqueId();
  return (
    <span class="inline-flex min-w-0 items-center gap-2">
      <Button
        variant={props.primary ? "primary" : "secondary"}
        disabled={props.reason !== null}
        title={props.reason ?? undefined}
        aria-describedby={props.reason !== null ? id : undefined}
        onClick={() => props.onClick()}
      >
        {props.label}
      </Button>
      <Show when={props.reason}>
        {(r) => (
          <span id={id} class="truncate text-[12px] text-muted">
            {r()}
          </span>
        )}
      </Show>
    </span>
  );
}

function Progress(props: { label: string; p: { loaded: number; total: number } | null }) {
  const v = () => progressView(props.p);
  return (
    <div class="flex min-w-0 flex-1 items-center gap-2 text-[12px]">
      <span class="shrink-0 text-muted">{props.label}</span>
      <div
        role="progressbar"
        aria-label={tsd.aria.progress(props.label)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={v().percent ?? undefined}
        aria-valuetext={v().percent === null ? `${tsd.aria.indeterminate} ${v().text}`.trim() : v().text}
        class="relative h-1.5 min-w-16 flex-1 overflow-hidden rounded bg-border"
      >
        <Show when={v().percent !== null} fallback={<div class="absolute inset-y-0 left-0 w-1/3 animate-pulse rounded bg-accent" />}>
          <div class="absolute inset-y-0 left-0 rounded bg-accent" style={{ width: `${v().percent}%` }} />
        </Show>
      </div>
      <span class="w-24 shrink-0 tabular-nums text-muted">{v().text}</span>
    </div>
  );
}

/** Üç durumlu toplu seçim kutusu (`indeterminate` DOM özelliği). */
function TriBox(props: { label: string; state: "none" | "some" | "all"; disabled: boolean; onChange: (on: boolean) => void }) {
  let el!: HTMLInputElement;
  createEffect(() => {
    el.indeterminate = props.state === "some";
  });
  return (
    <label class="inline-flex shrink-0 items-center gap-1 text-[12px]">
      <input
        ref={el}
        type="checkbox"
        checked={props.state === "all"}
        disabled={props.disabled}
        onChange={() => props.onChange(props.state !== "all")}
      />
      {props.label}
    </label>
  );
}

function Strip(props: { role: "status" | "alert"; tone: "muted" | "warn" | "danger"; children: JSX.Element }) {
  return (
    <div
      role={props.role}
      class="rounded border px-2 py-1 text-[12px]"
      classList={{
        "border-border text-muted": props.tone === "muted",
        "border-[color:var(--srv-amber)] text-warn-text": props.tone === "warn",
        "border-danger text-danger": props.tone === "danger",
      }}
    >
      {props.children}
    </div>
  );
}

function Body(props: { diff: DiffController; tabId: string; m: DiffModel }) {
  const d = props.diff;
  const id = props.tabId;
  const m = () => props.m;
  const running = () => m().phase === "running";
  const items = () => m().result?.items ?? [];
  const visible = createMemo(() => visibleItems(items(), m().filters, foldSearch(m().search)));
  const cnt = createMemo(() => counts(items()));
  const compareReason = () => {
    const b = compareBlocked(m().source, m().target, running());
    return b === null ? null : tsd.blocked[b];
  };
  const scriptReason = () => {
    const b = scriptBlocked({
      running: running(), hasResult: m().result !== null, stale: m().stale,
      selected: selectedKeys(m().selected, items()).length,
    });
    return b === null ? null : tsd.blocked[b];
  };
  const sel = () => selectionSummary(m().selected, visible());
  const focusedItem = () => {
    const k = m().focused;
    return k === null ? undefined : items().find((i) => i.key === k);
  };
  const opt = (key: keyof DiffOptions) => (
    <label class="inline-flex items-center gap-1 text-[12px]">
      <input type="checkbox" checked={m().options[key]} onChange={(e) => d.setOption(id, key, e.currentTarget.checked)} />
      {tsd.options[key]}
    </label>
  );

  return (
    <div class="flex h-full min-h-0 flex-col">
      {/* Üst bölüm */}
      <div class="flex shrink-0 flex-col gap-2 border-b border-border p-2">
        <div class="flex items-center gap-2">
          <SidePicker diff={d} tabId={id} side="source" title={tsd.source} />
          <button
            type="button"
            class="rounded p-1.5 text-muted hover:bg-hover disabled:opacity-50"
            aria-label={tsd.swap}
            title={tsd.swap}
            onClick={() => d.swap(id)}
          >
            <ArrowLeftRight size={16} aria-hidden="true" />
          </button>
          <SidePicker diff={d} tabId={id} side="target" title={tsd.target} />
        </div>
        <div class="flex flex-wrap items-center gap-3">
          <fieldset class="flex items-center gap-3">
            <legend class="sr-only">{tsd.optionsLegend}</legend>
            {opt("ownership")}
            {opt("tablespace")}
            {opt("comments")}
          </fieldset>
          <span class="flex-1" />
          <Show
            when={running()}
            fallback={<GatedButton label={tsd.compare} reason={compareReason()} primary onClick={() => void d.compare(id)} />}
          >
            <Button disabled={m().cancelRequested} onClick={() => d.cancel(id)}>
              {m().cancelRequested ? tsd.cancelling : tsd.cancel}
            </Button>
          </Show>
        </div>
        <Show when={running()}>
          <div class="flex items-center gap-4">
            <Progress label={tsd.source} p={m().progress.source} />
            <Progress label={tsd.target} p={m().progress.target} />
          </div>
        </Show>
        <Show when={m().stale}>
          <Strip role="status" tone="warn">
            {tsd.blocked.stale}
          </Strip>
        </Show>
        <Show when={m().banner}>
          {(b) => (
            <Strip role="status" tone="muted">
              {b()}
            </Strip>
          )}
        </Show>
        <Show when={m().error}>
          {(e) => (
            <Strip role="alert" tone="danger">
              {e()}
            </Strip>
          )}
        </Show>
        <Show when={(m().result?.warnings.length ?? 0) > 0}>
          <div class="rounded border border-[color:var(--srv-amber)] px-2 py-1 text-[12px] text-warn-text">
            <p class="font-semibold">{tsd.warnings}</p>
            <ul class="list-disc pl-5">
              <For each={m().result?.warnings ?? []}>{(w) => <li>{w}</li>}</For>
            </ul>
          </div>
        </Show>
        <Show when={m().result}>
          {(r) => (
            <p role="status" class="text-[12px] text-muted">
              {tsd.summary(r().items.length, r().elapsedMs)}
            </p>
          )}
        </Show>
      </div>

      {/* Liste | ayrıntı */}
      <div class="flex min-h-0 flex-1">
        <div class="flex min-h-0 min-w-0 flex-col border-r border-border" style={{ width: "45%" }}>
          <div class="flex shrink-0 flex-col gap-1.5 border-b border-border p-2">
            <div role="group" aria-label={tsd.aria.categories} class="flex flex-wrap gap-1">
              <For each={CATEGORIES}>
                {(c) => (
                  <button
                    type="button"
                    aria-pressed={m().filters[c]}
                    class="h-6 rounded border px-2 text-[12px] hover:bg-hover"
                    classList={{
                      "border-accent bg-selected": m().filters[c],
                      "border-border text-muted": !m().filters[c],
                    }}
                    onClick={() => d.setFilter(id, c, !m().filters[c])}
                  >
                    {`${tsd.category[c]} (${cnt()[c]})`}
                  </button>
                )}
              </For>
            </div>
            <input
              type="search"
              class={inputClass}
              placeholder={tsd.search}
              aria-label={tsd.search}
              value={m().search}
              onInput={(e) => d.setSearch(id, e.currentTarget.value)}
            />
            <div class="flex flex-wrap items-center gap-3">
              <TriBox
                label={tsd.selectAll}
                state={selectAllState(m().selected, items())}
                disabled={m().result === null}
                onChange={(on) => d.selectAll(id, on)}
              />
              <TriBox
                label={tsd.selectVisible}
                state={selectAllState(m().selected, visible())}
                disabled={m().result === null}
                onChange={(on) => d.selectVisible(id, on)}
              />
              <span class="text-[12px] text-muted">{tsd.selection(sel().selected, sel().hidden)}</span>
              <span class="flex-1" />
              <GatedButton label={tsd.buildScript} reason={scriptReason()} onClick={() => void d.buildScript(id)} />
            </div>
          </div>
          <Show
            when={m().result !== null && visible().length > 0}
            fallback={
              <p class="p-3 text-[12px] text-muted">
                {m().result === null ? "" : items().length === 0 ? tsd.empty : tsd.emptyFiltered}
              </p>
            }
          >
            <DiffList diff={d} tabId={id} items={visible()} selected={m().selected} focused={m().focused} />
          </Show>
        </div>
        <section aria-label={tsd.aria.detail} class="flex min-h-0 min-w-0 flex-col" style={{ width: "55%" }}>
          <Show when={focusedItem()}>
            {(it) => (
              <div class="shrink-0 border-b border-border p-2">
                <h3 class="flex flex-wrap items-baseline gap-2 text-[13px]">
                  <span class="text-muted">{kindLabel(it().kind)}</span>
                  <span class="font-mono font-semibold">{itemLabel(it())}</span>
                  <span class="text-[12px] text-muted">{tsd.category[categoryOf(it())]}</span>
                </h3>
                <Show when={problemText(it())}>{(t) => <p class="text-[12px] text-danger">{t()}</p>}</Show>
                <Show when={it().errors.length > 0}>
                  <div role="alert" class="mt-1 text-[12px] text-danger">
                    <p class="font-semibold">{tsd.detail.errors}</p>
                    <ul class="list-disc pl-5">
                      <For each={it().errors}>{(x) => <li>{x}</li>}</For>
                    </ul>
                  </div>
                </Show>
                <Show when={it().notes.length > 0}>
                  <div class="mt-1 text-[12px] text-muted">
                    <p class="font-semibold">{tsd.detail.notes}</p>
                    <ul class="list-disc pl-5">
                      <For each={it().notes}>{(x) => <li>{x}</li>}</For>
                    </ul>
                  </div>
                </Show>
              </div>
            )}
          </Show>
          <Show
            when={m().detail?.error}
            fallback={<LineDiffView picked={m().detail !== null} detail={m().detail?.data ?? null} />}
          >
            {(e) => (
              <p role="alert" class="p-3 text-[12px] text-danger">
                {e()}
              </p>
            )}
          </Show>
        </section>
      </div>

      <Show when={m().script !== null}>
        <Suspense>
          <ScriptDialog diff={d} tabId={id} />
        </Suspense>
      </Show>
    </div>
  );
}

export default function SchemaDiffView(props: { diff: DiffController; tabId: string }) {
  return <Show when={props.diff.model(props.tabId)}>{(m) => <Body diff={props.diff} tabId={props.tabId} m={m()} />}</Show>;
}
