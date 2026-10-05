import { createSignal, type JSX } from "solid-js";
import { clampWidth } from "./split";
import { readStorage, writeStorage } from "./storage";

const KEY = "pgt.split.left";

export function SplitPane(props: { left: JSX.Element; right: JSX.Element }) {
  const [width, setWidth] = createSignal(Number(readStorage(KEY)) || 320);
  let container!: HTMLDivElement;

  const onPointerDown = (e: PointerEvent) => {
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) =>
      setWidth(clampWidth(ev.clientX - container.getBoundingClientRect().left, container.clientWidth));
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      writeStorage(KEY, String(width()));
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  };

  return (
    <div ref={container} class="flex min-h-0 flex-1">
      <div class="min-w-0 shrink-0 border-r border-border bg-panel" style={{ width: `${width()}px` }}>
        {props.left}
      </div>
      <div
        role="separator"
        aria-orientation="vertical"
        class="w-1 shrink-0 cursor-col-resize hover:bg-accent/40"
        onPointerDown={onPointerDown}
      />
      <div class="min-w-0 flex-1">{props.right}</div>
    </div>
  );
}
