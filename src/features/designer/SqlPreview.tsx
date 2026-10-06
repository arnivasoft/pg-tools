// Designer SQL önizlemesi (tembel parça): arka ucun döndürdüğü ifadeler salt okunur CodeMirror'da (metin yalnızca
// `PlanView.statements[].sql`; istemci SQL üretmez/değiştirmez), ifade listesi (risk, transaction dışı, not), uyarılar,
// genel hatalar ve boş durumlar. Başarısız uygulamada (`outcome.failed`) ilgili ifadenin satırları vurgulanır.
import { Compartment, RangeSetBuilder, type Extension } from "@codemirror/state";
import { Decoration, EditorView } from "@codemirror/view";
import { Copy } from "lucide-solid";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { copyText } from "../../app/clipboard";
import { tdes } from "../../app/strings-designer";
import { toast } from "../../app/toast";
import type { PlanView } from "../../ipc/bindings/PlanView";
import { Button } from "../../ui/Button";
import { createReadOnlyEditor, replaceDoc } from "../inspector/codemirror-setup";
import type { DesignerModel } from "./designer-controller";
import { previewEmptyState, scriptWithRanges, statementRangesToLineRanges } from "./designer-model";
import { RiskBadge } from "./RiskBadge";

const failedLine = Decoration.line({ class: "cm-pgt-failed" });
const failedTheme = EditorView.baseTheme({
  ".cm-pgt-failed": { backgroundColor: "color-mix(in srgb, var(--danger) 16%, transparent)" },
});

/** `from..to` (1 tabanlı, kapsayıcı) satırlarına vurgu; belge dışı satırlar atlanır. */
function failedDecorations(view: EditorView, lines: { from: number; to: number } | null): Extension {
  if (!lines) return [];
  const doc = view.state.doc;
  const b = new RangeSetBuilder<Decoration>();
  for (let l = Math.max(1, lines.from); l <= Math.min(lines.to, doc.lines); l++) {
    const at = doc.line(l).from;
    b.add(at, at, failedLine);
  }
  return EditorView.decorations.of(b.finish());
}

export default function SqlPreview(props: { model: DesignerModel; general: string[] }) {
  const p = tdes.preview;
  // Başarısız uygulamadan sonra denetleyici görünümü düşürür; gönderilen son betik vurguyla gösterilmeye devam eder.
  const [last, setLast] = createSignal<PlanView | null>(null);
  createEffect(() => {
    const v = props.model.view;
    if (v) setLast(v);
  });
  const failed = () => (props.model.outcome?.outcome === "failed" ? props.model.outcome : null);
  const shown = () => props.model.view ?? (failed() ? last() : null);
  const showingLast = () => props.model.view === null && shown() !== null;
  const empty = () => previewEmptyState(shown(), props.model.pending, props.model.mode, props.model.status, props.model.previewError);
  const script = createMemo(() => scriptWithRanges(shown()?.statements ?? []));
  const failedIndex = () => {
    const f = failed();
    const v = shown();
    return f && v && f.failedIndex >= 0 && f.failedIndex < v.statements.length ? f.failedIndex : null;
  };
  const failedLines = createMemo(() => {
    const i = failedIndex();
    if (i === null) return null;
    const s = script();
    return statementRangesToLineRanges(s.text, s.ranges)[i] ?? null;
  });

  let host!: HTMLDivElement;
  let editor: EditorView | undefined;
  const decos = new Compartment();
  onMount(() => {
    editor = createReadOnlyEditor(host, "", [failedTheme, decos.of([])]);
  });
  createEffect(() => {
    const text = empty() === null ? script().text : "";
    const lines = failedLines();
    if (!editor) return;
    if (editor.state.doc.toString() !== text) replaceDoc(editor, text);
    const effects = [decos.reconfigure(failedDecorations(editor, lines))];
    if (lines && lines.from <= editor.state.doc.lines) {
      editor.dispatch({ effects: [...effects, EditorView.scrollIntoView(editor.state.doc.line(lines.from).from, { y: "center" })] });
    } else editor.dispatch({ effects });
  });
  onCleanup(() => editor?.destroy());

  const copy = async () => {
    try {
      await copyText(script().text);
      toast.show(p.copied, "success");
    } catch {
      toast.show(p.copyFailed, "error");
    }
  };

  const emptyText = () => {
    switch (empty()) {
      case "loading": return tdes.loading;
      case "pending": return p.pending;
      case "incomplete": return p.incomplete;
      case "noChanges": return p.noChanges;
      default: return "";
    }
  };

  return (
    <section aria-label={p.title} class="flex min-h-0 flex-1 flex-col">
      <div class="flex h-8 shrink-0 items-center gap-2 border-b border-border px-2">
        <h2 class="text-[12px] font-semibold">{p.title}</h2>
        <Show when={props.model.pending && shown() !== null}>
          <span role="status" class="text-[12px] text-muted">
            {p.refreshing}
          </span>
        </Show>
        <Show when={showingLast()}>
          <span class="truncate text-[12px] text-muted">{p.lastSent}</span>
        </Show>
        <Button
          variant="ghost"
          class="ml-auto !h-6 !px-1.5 text-[12px]"
          title={p.copyTitle}
          disabled={empty() !== null || script().text === ""}
          onClick={() => void copy()}
        >
          <Copy size={13} /> {p.copy}
        </Button>
      </div>
      <Show when={props.general.length > 0}>
        <div role="alert" class="shrink-0 border-b border-border px-3 py-1.5 text-[12px]">
          <p class="font-medium text-danger">{p.general}</p>
          <ul class="list-disc pl-4 text-danger">
            <For each={props.general}>{(g) => <li>{g}</li>}</For>
          </ul>
        </div>
      </Show>
      <Show when={props.model.previewError}>
        {(e) => (
          <div role="alert" class="shrink-0 border-b border-border px-3 py-1.5 text-[12px] text-danger">
            <span class="font-medium">{p.error}:</span> {e()}
          </div>
        )}
      </Show>
      <Show when={(shown()?.warnings.length ?? 0) > 0}>
        <div role="status" class="shrink-0 border-b border-border bg-panel px-3 py-1.5 text-[12px] text-warn-text">
          <p class="font-medium">{p.warnings}</p>
          <ul class="list-disc pl-4">
            <For each={shown()?.warnings ?? []}>{(w) => <li>{w}</li>}</For>
          </ul>
        </div>
      </Show>
      <div class="relative min-h-24 flex-1" classList={{ "opacity-70": props.model.pending && shown() !== null }}>
        <div ref={host} class="absolute inset-0" aria-label={p.title} />
        <Show when={empty()}>
          <div class="absolute inset-0 flex flex-col items-center justify-center gap-1 bg-bg p-4 text-center">
            <p class="text-[13px] text-muted" role="status">
              {emptyText()}
            </p>
            <Show when={empty() === "incomplete"}>
              <p class="text-[12px] text-muted">{p.incompleteNote}</p>
            </Show>
          </div>
        </Show>
      </div>
      <Show when={empty() === null && (shown()?.statements.length ?? 0) > 0}>
        <div class="max-h-40 shrink-0 overflow-auto border-t border-border">
          <h3 class="sr-only">{p.statements}</h3>
          <ol class="flex flex-col">
            <For each={shown()?.statements ?? []}>
              {(s, i) => (
                <li
                  class="flex items-center gap-2 border-b border-border px-2 py-1 text-[12px] last:border-b-0"
                  classList={{ "bg-[color:color-mix(in_srgb,var(--danger)_10%,transparent)]": failedIndex() === i() }}
                >
                  <span class="w-14 shrink-0 text-muted">{p.statement(i())}</span>
                  <RiskBadge risk={s.risk} />
                  <Show when={!s.transactional}>
                    <span
                      class="inline-flex h-[18px] shrink-0 items-center rounded-full border border-[color:var(--srv-amber)] px-1.5 text-[11px] text-warn-text"
                      title={p.nonTxTitle}
                    >
                      {p.nonTx}
                    </span>
                  </Show>
                  <Show when={failedIndex() === i()}>
                    <span class="inline-flex h-[18px] shrink-0 items-center rounded-full border border-danger px-1.5 text-[11px] text-danger">
                      {p.failedBadge}
                    </span>
                  </Show>
                  <Show when={s.note}>
                    <span class="min-w-0 truncate text-muted" title={s.note ?? ""}>
                      {s.note}
                    </span>
                  </Show>
                </li>
              )}
            </For>
          </ol>
        </div>
      </Show>
    </section>
  );
}
