// Hücre görüntüleyicileri (tembel parça). Görünümler yalnız çizer; çözme/biçimleme createMemo'dadır ve görünüm açıkken çalışır.
// CellPanel bileşeni yeniden oluşturmaz (props getter'dır): tüm okumalar reaktiftir. Hücre değeri hiçbir yere yazılmaz.
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, untrack, type JSX } from "solid-js";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { save } from "@tauri-apps/plugin-dialog";
import type { EditorView } from "@codemirror/view";
import type { Column } from "../../ipc/bindings/Column";
import { toast } from "../../app/toast";
import { Button } from "../../ui/Button";
import { reindentJson } from "./grid-model";
import {
  LIMITS,
  utf8Length,
  decodePrefix,
  fileActionAllowed,
  formatMb,
  previewAllowed,
  sizeVerdict,
  type CellFacts,
  type CellFormat,
} from "./cell-view-model";
import {
  dataUrl,
  decodeByteaHex,
  formatHtml,
  formatXml,
  hexRow,
  hexRowCount,
  pdfVersion,
  previewDoc,
  saveCell,
  utf8Bytes,
} from "./cell-format";
import { createCodeViewer, loadLanguage, replaceCode, type CodeLang, type LoadedLanguage } from "./cell-code";
import { cellIpc, type CellIpc } from "./cell-ipc";
import { tcell } from "./strings-cell";

interface ViewProps {
  value: string;
  column: Column;
  facts: CellFacts;
  ipc: CellIpc;
}

const MONO = '12px "Cascadia Mono", Consolas, monospace';
const note = "font-sans text-[13px] text-muted";

/** Ortak "Kaydet…" (Görsel, PDF, SizeNotice): iletişim kutusu → IPC → toast. */
function createSaver(p: () => ViewProps) {
  const [saving, setSaving] = createSignal(false);
  const run = async () => {
    if (saving()) return;
    setSaving(true);
    const { value, column, facts, ipc } = p();
    try {
      await saveCell(
        {
          pick: (o) => save(o),
          saveBytes: ipc.saveBytes,
          onSaved: () => toast.show(tcell.saved, "success"),
          onError: () => toast.show(tcell.saveFailed, "error"),
        },
        value,
        column.name,
        { pdf: facts.pdf, image: facts.image },
      );
    } finally {
      setSaving(false);
    }
  };
  return { saving, run };
}

function SaveButton(props: ViewProps) {
  const saver = createSaver(() => props);
  const allowed = () => fileActionAllowed(props.facts);
  return (
    <Show when={props.facts.bytea}>
      <Button class="h-6 text-[12px]" disabled={!allowed().enabled || saver.saving()} title={allowed().reason ?? undefined} onClick={() => void saver.run()}>
        {saver.saving() ? tcell.saving : tcell.save}
      </Button>
    </Show>
  );
}

function SizeNotice(props: ViewProps & { size: number; limit: number }) {
  return (
    <div class="flex flex-col items-start gap-2">
      <p class={note} role="status">
        {(props.facts.bytea ? tcell.tooLarge : tcell.tooLargeText)(formatMb(props.size), formatMb(props.limit))}
      </p>
      <SaveButton {...props} />
    </div>
  );
}

/** Boyut sınırı aşılırsa görünüm yerine bildirim; sınır ham uzunluktan, çözmeden önce denetlenir. */
function Guard(props: ViewProps & { format: Exclude<CellFormat, "text">; children: JSX.Element }) {
  const verdict = createMemo(() => sizeVerdict(props.format, props.facts));
  return (
    <Show
      when={verdict().ok}
      fallback={
        <SizeNotice
          {...props}
          size={(verdict() as { size: number }).size}
          limit={(verdict() as { limit: number }).limit}
        />
      }
    >
      {props.children}
    </Show>
  );
}

function HexView(props: ViewProps) {
  // Bytea → çözülen önek; metin → UTF-8 baytları. Tek kez, görünüm açıkken.
  const bytes = createMemo(() =>
    props.facts.bytea ? decodeByteaHex(props.value, LIMITS.hexBytes) : utf8Bytes(props.value, LIMITS.hexBytes),
  );
  // Toplam: bytea için ham uzunluktan; metin için kesilme olasıysa ayırmasız sayımla, değilse kodlanan uzunluktan.
  const total = createMemo(() => {
    if (props.facts.bytea) return props.facts.byteLength;
    return props.value.length * 3 > LIMITS.hexBytes ? utf8Length(props.value) : (bytes()?.length ?? 0);
  });
  const truncated = () => total() > LIMITS.hexBytes;
  let scrollEl!: HTMLDivElement;
  const rows = createVirtualizer({
    get count() {
      return hexRowCount(bytes()?.length ?? 0);
    },
    getScrollElement: () => scrollEl,
    estimateSize: () => 18,
    overscan: 20,
  });
  // Hücre değişince başa dön.
  createEffect(on(() => props.value, () => scrollEl?.scrollTo({ top: 0 }), { defer: true }));
  return (
    <Show when={bytes()} fallback={<p class={note}>{tcell.reason.badHex}</p>}>
      {(b) => (
        <div class="flex h-full min-h-0 flex-col gap-1">
          <p class={note}>
            {tcell.hex.total(total())}
            <Show when={truncated()}>
              {" · "}
              {tcell.hex.partial(formatMb(b().length), formatMb(total()))}
            </Show>
          </p>
          <div ref={scrollEl} class="min-h-0 flex-1 overflow-auto" tabindex="0" role="region" aria-label={tcell.hex.ariaLabel}>
            <div class="relative" style={{ height: `${rows.getTotalSize()}px`, width: "max-content", "min-width": "100%" }}>
              <For each={rows.getVirtualItems()}>
                {(r) => (
                  <div
                    class="absolute left-0 whitespace-pre"
                    style={{ top: "0", height: "18px", "line-height": "18px", font: MONO, transform: `translateY(${r.start}px)` }}
                  >
                    {hexRow(b(), r.index)}
                  </div>
                )}
              </For>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}

const CHECKER = "repeating-conic-gradient(var(--border) 0% 25%, transparent 0% 50%) 0 0 / 16px 16px";

function ImageView(props: ViewProps) {
  const bytes = createMemo(() => {
    if (!props.facts.image) return null;
    return props.facts.bytea ? decodeByteaHex(props.value) : utf8Bytes(props.value);
  });
  const url = createMemo(() => {
    const kind = props.facts.image;
    const b = bytes();
    return kind && b ? dataUrl(kind, b) : null;
  });
  const [fit, setFit] = createSignal(true);
  const [dims, setDims] = createSignal<{ w: number; h: number } | null>(null);
  const [broken, setBroken] = createSignal(false);
  createEffect(
    on(url, () => {
      setDims(null);
      setBroken(false);
    }),
  );
  const kind = () => (props.facts.image ?? "").toUpperCase();
  const alt = () => {
    const k = kind();
    const d = dims();
    return d ? tcell.image.alt(k, d.w, d.h) : k;
  };
  return (
    <Show when={url()} fallback={<p class={note}>{tcell.reason.badHex}</p>}>
      {(u) => (
        <div class="flex h-full min-h-0 flex-col gap-2">
          <div class="flex items-center gap-2">
            <Button class="h-6 text-[12px]" aria-pressed={fit()} onClick={() => setFit(true)}>
              {tcell.image.fit}
            </Button>
            <Button class="h-6 text-[12px]" aria-pressed={!fit()} onClick={() => setFit(false)}>
              {tcell.image.actual}
            </Button>
            <SaveButton {...props} />
          </div>
          <p class={note}>
            {tcell.image.info(kind(), dims() ? `${dims()!.w} × ${dims()!.h}` : null, tcell.hex.total(bytes()?.length ?? 0))}
          </p>
          <Show when={!broken()} fallback={<p class="font-sans text-[13px] text-danger">{tcell.image.broken}</p>}>
            <div
              class={`min-h-0 flex-1 rounded border border-border ${fit() ? "flex items-center justify-center overflow-hidden" : "overflow-auto"}`}
              style={{ background: CHECKER }}
            >
              <img
                src={u()}
                alt={alt()}
                class={fit() ? "max-h-full max-w-full object-contain" : "max-w-none"}
                onLoad={(e) => setDims({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })}
                onError={() => setBroken(true)}
              />
            </div>
          </Show>
        </div>
      )}
    </Show>
  );
}

/** Salt okunur CodeMirror; dil tembel yüklenir, metin/dil değişince replaceCode, kapanışta destroy. */
function CodeView(props: { lang: CodeLang; text: string }) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  const [loaded, setLoaded] = createSignal<LoadedLanguage | null>(null);
  const [failed, setFailed] = createSignal(false);
  let disposed = false;
  const load = (lang: CodeLang) => {
    setLoaded(null);
    setFailed(false);
    loadLanguage(lang)
      .then((m) => {
        if (!disposed && props.lang === lang) setLoaded(m);
      })
      .catch(() => {
        if (!disposed && props.lang === lang) setFailed(true);
      });
  };
  createEffect(on(() => props.lang, load));
  createEffect(() => {
    const m = loaded();
    const text = props.text;
    if (!m) return;
    untrack(() => {
      if (view) replaceCode(view, text, m.extension);
      else view = createCodeViewer(host, text, m.extension);
    });
  });
  onCleanup(() => {
    disposed = true;
    view?.destroy();
    view = undefined;
  });
  return (
    <div class="flex h-full min-h-0 flex-col">
      <Show when={failed()}>
        <div class="flex items-center gap-2">
          <p class="font-sans text-[13px] text-danger">{tcell.loadFailed}</p>
          <Button class="h-6 text-[12px]" onClick={() => load(props.lang)}>
            {tcell.retry}
          </Button>
        </div>
      </Show>
      <div ref={host} class="min-h-0 flex-1 overflow-hidden [&_.cm-editor]:h-full" />
    </div>
  );
}

function FormatToggle(props: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label class="inline-flex items-center gap-1.5 font-sans text-[12px]">
      <input type="checkbox" checked={props.checked} onChange={(e) => props.onChange(e.currentTarget.checked)} />
      {tcell.html.format}
    </label>
  );
}

function JsonView(props: ViewProps) {
  const text = createMemo(() => reindentJson(props.value) ?? props.value);
  return <CodeView lang="json" text={text()} />;
}

function XmlView(props: ViewProps) {
  const [fmt, setFmt] = createSignal(true);
  const text = createMemo(() => (fmt() ? formatXml(props.value) : props.value));
  return (
    <div class="flex h-full min-h-0 flex-col gap-2">
      <div>
        <FormatToggle checked={fmt()} onChange={setFmt} />
      </div>
      <div class="min-h-0 flex-1">
        <CodeView lang="xml" text={text()} />
      </div>
    </div>
  );
}

function HtmlView(props: ViewProps) {
  const [mode, setMode] = createSignal<"source" | "preview">("source");
  const [fmt, setFmt] = createSignal(true);
  const preview = createMemo(() => previewAllowed(props.facts));
  const effective = () => (mode() === "preview" && preview().enabled ? "preview" : "source");
  const text = createMemo(() => (effective() === "source" ? (fmt() ? formatHtml(props.value) : props.value) : ""));
  return (
    <div class="flex h-full min-h-0 flex-col gap-2">
      <div class="flex flex-wrap items-center gap-2">
        <Button class="h-6 text-[12px]" aria-pressed={effective() === "source"} onClick={() => setMode("source")}>
          {tcell.html.source}
        </Button>
        <Button
          class="h-6 text-[12px]"
          aria-pressed={effective() === "preview"}
          aria-disabled={!preview().enabled}
          title={preview().reason ?? undefined}
          onClick={() => {
            if (preview().enabled) setMode("preview");
          }}
        >
          {tcell.html.preview}
        </Button>
        <Show when={effective() === "source"}>
          <FormatToggle checked={fmt()} onChange={setFmt} />
        </Show>
        <Show when={!preview().enabled}>
          <span class={note}>{preview().reason}</span>
        </Show>
      </div>
      <div class="min-h-0 flex-1">
        <Show
          when={effective() === "preview"}
          fallback={<CodeView lang="html" text={text()} />}
        >
          <div class="flex h-full min-h-0 flex-col gap-1">
            <iframe
              sandbox=""
              referrerpolicy="no-referrer"
              title={tcell.html.frameTitle}
              srcdoc={previewDoc(props.value)}
              class="min-h-0 w-full flex-1 bg-white"
            />
            <p class={note}>{tcell.html.previewNote}</p>
          </div>
        </Show>
      </div>
    </div>
  );
}

function PdfView(props: ViewProps) {
  const [opening, setOpening] = createSignal(false);
  const saver = createSaver(() => props);
  const allowed = () => fileActionAllowed(props.facts);
  const version = createMemo(() => {
    const b = decodePrefix(props.value, 16);
    return b ? pdfVersion(b) : null;
  });
  const open = async () => {
    if (opening()) return;
    setOpening(true);
    try {
      await props.ipc.openPdf(props.value);
    } catch {
      toast.show(tcell.openFailed, "error");
    } finally {
      setOpening(false);
    }
  };
  return (
    <div class="flex flex-col items-start gap-2">
      <p class={note}>{tcell.pdf.info(version(), tcell.hex.total(props.facts.byteLength))}</p>
      <div class="flex items-center gap-2">
        <Button class="h-6 text-[12px]" disabled={!allowed().enabled || opening()} title={allowed().reason ?? undefined} onClick={() => void open()}>
          {opening() ? tcell.pdf.opening : tcell.pdf.open}
        </Button>
        <Button class="h-6 text-[12px]" disabled={!allowed().enabled || saver.saving()} title={allowed().reason ?? undefined} onClick={() => void saver.run()}>
          {saver.saving() ? tcell.saving : tcell.save}
        </Button>
      </div>
      <Show when={!allowed().enabled}>
        <p class={note}>{allowed().reason}</p>
      </Show>
      <p class={note}>{tcell.pdf.warn}</p>
    </div>
  );
}

export default function CellViewers(props: {
  format: Exclude<CellFormat, "text">;
  value: string;
  column: Column;
  facts: CellFacts;
  ipc?: CellIpc;
}) {
  // Getter'lı ortak özellikler: biçim/hücre değişince görünümler yeniden oluşturulmadan güncellenir.
  const vp: ViewProps = {
    get value() {
      return props.value;
    },
    get column() {
      return props.column;
    },
    get facts() {
      return props.facts;
    },
    get ipc() {
      return props.ipc ?? cellIpc;
    },
  };
  return (
    <div class="h-full min-h-0">
      <Switch>
        <Match when={props.format === "hex"}>
          <HexView {...vp} />
        </Match>
        <Match when={props.format === "image"}>
          <Guard {...vp} format="image">
            <ImageView {...vp} />
          </Guard>
        </Match>
        <Match when={props.format === "json"}>
          <Guard {...vp} format="json">
            <JsonView {...vp} />
          </Guard>
        </Match>
        <Match when={props.format === "xml"}>
          <Guard {...vp} format="xml">
            <XmlView {...vp} />
          </Guard>
        </Match>
        <Match when={props.format === "html"}>
          <Guard {...vp} format="html">
            <HtmlView {...vp} />
          </Guard>
        </Match>
        <Match when={props.format === "pdf"}>
          <Guard {...vp} format="pdf">
            <PdfView {...vp} />
          </Guard>
        </Match>
      </Switch>
    </div>
  );
}
