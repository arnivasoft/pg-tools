// Tek <canvas> üzerinde zaman eksenli grafik. Yalnızca veri/boyut/tema/imleç değişince requestAnimationFrame ile bir kez
// çizer (sürekli döngü yok; gizli sekmede boyut 0 olduğundan çizilmez). Renkler CSS değişkenlerinden okunur.
import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { td } from "../../app/strings-dashboard";
import { buildPaths, nearestIndex, niceMax, seriesMax, xDomain, yTicks, type Series, type Viewport } from "./chart-model";
import { formatCount } from "./dash-model";

export interface ChartProps {
  title: string;
  series: Series[];
  /** Zaman penceresi (sn); varsayılan 10 dk. */
  windowSec?: number;
  /** Değer biçimi (eksen ve ipucu). */
  format?: (v: number) => string;
  height?: number;
  /** Sayım değerleri: y ekseni tam sayı adımlı. */
  integer?: boolean;
}

const PAD = { padLeft: 44, padBottom: 18, padTop: 6, padRight: 8 };
const fmtTime = (t: number) => new Date(t * 1000).toLocaleTimeString("tr-TR", { hour12: false });
const defaultFormat = (v: number) => (Number.isInteger(v) ? formatCount(v) : v < 10 ? v.toFixed(1).replace(".", ",") : formatCount(Math.round(v)));

export default function Chart(props: ChartProps) {
  let canvas!: HTMLCanvasElement;
  let box!: HTMLDivElement;
  let raf = 0;
  // Son atanan yedek bellek boyutu: yalnızca değişince atanır (atama tuvali siler ve yeniden ayırır).
  let lastBw = 0;
  let lastBh = 0;
  const [size, setSize] = createSignal({ w: 0, h: 0 });
  const [themeTick, setThemeTick] = createSignal(0);
  const [hoverX, setHoverX] = createSignal<number | null>(null);
  const fmt = () => props.format ?? defaultFormat;
  const win = () => props.windowSec ?? 600;
  const hasData = () => props.series.some((s) => s.points.some((p) => p.v != null));
  /** Zaman ekseni için en çok noktalı seri. */
  const ref = () => props.series.reduce<Series | undefined>((a, s) => (!a || s.points.length > a.points.length ? s : a), undefined);
  const vp = (): Viewport => ({ width: size().w, height: size().h, ...PAD });

  const hoverIdx = () => {
    const x = hoverX();
    const r = ref();
    if (x == null || !r) return null;
    return nearestIndex(r.points, x, vp(), xDomain(props.series, win()));
  };

  const draw = () => {
    raf = 0;
    const { w, h } = size();
    const ctx = canvas.getContext("2d");
    if (!ctx || w <= 0 || h <= 0) return;
    const dpr = window.devicePixelRatio || 1;
    const bw = Math.round(w * dpr);
    const bh = Math.round(h * dpr);
    if (bw !== lastBw || bh !== lastBh) {
      canvas.width = bw;
      canvas.height = bh;
      lastBw = bw;
      lastBh = bh;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    ctx.lineCap = "round";
    const cs = getComputedStyle(document.documentElement);
    const color = (name: string) => cs.getPropertyValue(name).trim() || "#888";
    const v = vp();
    const domain = xDomain(props.series, win());
    const yMax = niceMax(seriesMax(props.series), props.integer);
    ctx.font = "11px " + (cs.getPropertyValue("--font-sans").trim() || "sans-serif");
    ctx.textBaseline = "middle";
    ctx.textAlign = "right";
    // Izgara + y etiketleri
    for (const t of yTicks(yMax, 4, props.integer)) {
      const y = v.padTop + (h - v.padTop - v.padBottom) * (1 - t / yMax);
      ctx.strokeStyle = color("--border");
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(v.padLeft, Math.round(y) + 0.5);
      ctx.lineTo(w - v.padRight, Math.round(y) + 0.5);
      ctx.stroke();
      ctx.fillStyle = color("--muted");
      ctx.fillText(fmt()(t), v.padLeft - 5, y);
    }
    // x etiketleri: başlangıç ve son zaman
    if (hasData()) {
      ctx.fillStyle = color("--muted");
      ctx.textBaseline = "alphabetic";
      ctx.textAlign = "left";
      ctx.fillText(fmtTime(domain[0]), v.padLeft, h - 4);
      ctx.textAlign = "right";
      ctx.fillText(fmtTime(domain[1]), w - v.padRight, h - 4);
    }
    // Seriler (çizim alanına kırpılır)
    ctx.save();
    ctx.beginPath();
    ctx.rect(v.padLeft, v.padTop, w - v.padLeft - v.padRight, h - v.padTop - v.padBottom);
    ctx.clip();
    const cmds = buildPaths(props.series, v, domain, yMax);
    // Aynı seri/tür komutları ardışıktır: her grup tek yol.
    let i = 0;
    while (i < cmds.length) {
      const { s, area } = cmds[i];
      let j = i;
      ctx.beginPath();
      while (j < cmds.length && cmds[j].s === s && cmds[j].area === area) {
        const c = cmds[j++];
        if (c.op === "M") ctx.moveTo(c.x, c.y);
        else if (c.op === "L") ctx.lineTo(c.x, c.y);
        else ctx.closePath();
      }
      const col = color(props.series[s].color);
      if (area) {
        ctx.globalAlpha = 0.55;
        ctx.fillStyle = col;
        ctx.fill();
        ctx.globalAlpha = 1;
      } else {
        ctx.strokeStyle = col;
        ctx.lineWidth = 1.5;
        ctx.lineJoin = "round";
        ctx.stroke();
      }
      i = j;
    }
    ctx.restore();
    // Kılavuz
    const hi = hoverIdx();
    const r = ref();
    if (hi != null && r) {
      const span = domain[1] - domain[0] || 1;
      const x = v.padLeft + ((r.points[hi].t - domain[0]) / span) * (w - v.padLeft - v.padRight);
      ctx.strokeStyle = color("--muted");
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(Math.round(x) + 0.5, v.padTop);
      ctx.lineTo(Math.round(x) + 0.5, h - v.padBottom);
      ctx.stroke();
    }
  };

  const schedule = () => {
    if (!raf) raf = requestAnimationFrame(draw);
  };

  onMount(() => {
    const ro = new ResizeObserver(() => {
      const r = box.getBoundingClientRect();
      setSize({ w: Math.floor(r.width), h: Math.floor(r.height) });
    });
    ro.observe(box);
    // Tema: data-theme özniteliği (uygulama teması) ya da sistem tercihi değişince renkler yeniden okunur.
    const mo = new MutationObserver(() => setThemeTick((n) => n + 1));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "class"] });
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onMq = () => setThemeTick((n) => n + 1);
    mq.addEventListener("change", onMq);
    onCleanup(() => {
      ro.disconnect();
      mo.disconnect();
      mq.removeEventListener("change", onMq);
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    });
  });

  // Veri, boyut, tema ya da imleç değişince bir kez çiz.
  createEffect(() => {
    void props.series;
    void size();
    void themeTick();
    void hoverX();
    schedule();
  });

  const onMove = (ev: PointerEvent) => setHoverX(ev.clientX - box.getBoundingClientRect().left);

  const last = () => {
    const r = ref();
    if (!r) return [];
    return r.points.slice(-5).map((_, k, arr) => {
      const idx = r.points.length - arr.length + k;
      return { t: r.points[idx].t, vals: props.series.map((s) => s.points[idx]?.v ?? null) };
    });
  };
  const lastText = () => {
    const rows = last();
    const row = rows[rows.length - 1];
    if (!row) return td.overview.waitingData;
    return props.series.map((s, i) => `${s.name}: ${row.vals[i] == null ? "—" : fmt()(row.vals[i] as number)}`).join(", ");
  };
  const tip = () => {
    const hi = hoverIdx();
    const r = ref();
    if (hi == null || !r) return null;
    const v = vp();
    const d = xDomain(props.series, win());
    const x = v.padLeft + ((r.points[hi].t - d[0]) / (d[1] - d[0] || 1)) * (v.width - v.padLeft - v.padRight);
    return {
      x,
      right: x > v.width / 2,
      time: fmtTime(r.points[hi].t),
      rows: props.series.map((s) => ({ name: s.name, color: s.color, v: s.points[hi]?.v ?? null })),
    };
  };

  return (
    <figure class="m-0 rounded border border-border bg-panel p-2">
      <figcaption class="mb-1 flex flex-wrap items-center gap-x-3 text-[12px]">
        <span class="font-medium">{props.title}</span>
        <For each={props.series}>
          {(s) => (
            <span class="inline-flex items-center gap-1 text-muted">
              <span aria-hidden="true" class="inline-block h-2 w-2 rounded-sm" style={{ background: `var(${s.color})` }} />
              {s.name}
            </span>
          )}
        </For>
      </figcaption>
      <div
        ref={box}
        class="relative w-full"
        style={{ height: `${props.height ?? 150}px` }}
        onPointerMove={onMove}
        onPointerLeave={() => setHoverX(null)}
      >
        <canvas ref={canvas} role="img" aria-label={`${props.title}. ${lastText()}`} style={{ width: "100%", height: "100%", display: "block" }} />
        <Show when={!hasData()}>
          <div class="pointer-events-none absolute inset-0 flex items-center justify-center text-[12px] text-muted">
            {td.overview.waitingData}
          </div>
        </Show>
        <Show when={tip()}>
          {(t) => (
            <div
              class="pointer-events-none absolute top-1 z-10 rounded border border-border bg-bg px-2 py-1 text-[11px] shadow"
              style={t().right ? { right: `${Math.max(0, size().w - t().x + 8)}px` } : { left: `${t().x + 8}px` }}
            >
              <div class="text-muted">{t().time}</div>
              <For each={t().rows}>
                {(r) => (
                  <div class="flex items-center gap-1">
                    <span aria-hidden="true" class="inline-block h-2 w-2 rounded-sm" style={{ background: `var(${r.color})` }} />
                    {r.name}: {r.v == null ? "—" : fmt()(r.v)}
                  </div>
                )}
              </For>
            </div>
          )}
        </Show>
      </div>
      <table class="sr-only">
        <caption>{`${props.title} · ${td.overview.last}`}</caption>
        <thead>
          <tr>
            <th>{td.overview.time}</th>
            <For each={props.series}>{(s) => <th>{s.name}</th>}</For>
          </tr>
        </thead>
        <tbody>
          <For each={last()}>
            {(row) => (
              <tr>
                <td>{fmtTime(row.t)}</td>
                <For each={row.vals}>{(v) => <td>{v == null ? "—" : fmt()(v)}</td>}</For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </figure>
  );
}
