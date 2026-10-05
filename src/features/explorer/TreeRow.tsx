import { ChevronDown, ChevronRight, CircleAlert, LoaderCircle } from "lucide-solid";
import { Show } from "solid-js";
import { NodeIcon } from "./NodeIcon";
import { rowLabel, type VisibleRow } from "./tree-model";

export const ROW_HEIGHT = 24;

export function rowDomId(id: string): string {
  return `tn-${encodeURIComponent(id)}`;
}

export function TreeRow(props: {
  row: VisibleRow;
  top: number;
  selected: boolean;
  onSelect: () => void;
  onToggle: () => void;
}) {
  const r = () => props.row;
  return (
    <div
      id={rowDomId(r().id)}
      data-node-id={encodeURIComponent(r().id)}
      role="treeitem"
      aria-level={r().depth + 1}
      aria-setsize={r().setSize}
      aria-posinset={r().posInSet}
      aria-selected={props.selected}
      aria-expanded={r().node.hasChildren ? r().expanded : undefined}
      class="absolute left-0 right-0 flex cursor-default items-center gap-1 whitespace-nowrap pr-2 select-none"
      classList={{ "bg-selected": props.selected, "hover:bg-hover": !props.selected, "italic text-muted": r().node.flags.fromExtension }}
      style={{ height: `${ROW_HEIGHT}px`, transform: `translateY(${props.top}px)`, "padding-left": `${r().depth * 14 + 4}px` }}
      onClick={props.onSelect}
      onDblClick={props.onToggle}
    >
      <span
        class="flex w-4 shrink-0 justify-center text-muted"
        onDblClick={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          props.onSelect();
          props.onToggle();
        }}
      >
        <Show when={r().node.hasChildren}>
          <Show when={!r().loading} fallback={<LoaderCircle size={12} class="animate-spin" />}>
            <Show when={r().expanded} fallback={<ChevronRight size={12} />}>
              <ChevronDown size={12} />
            </Show>
          </Show>
        </Show>
      </span>
      <NodeIcon node={r().node} />
      <span class="truncate">{rowLabel(r())}</span>
      <Show when={r().node.detail}>
        <span class="truncate text-xs text-muted">{r().node.detail}</span>
      </Show>
      <Show when={r().error}>
        {(error) => (
          <span class="ml-auto shrink-0 text-danger" title={error().detail ? `${error().message}\n${error().detail}` : error().message}>
            <CircleAlert size={13} />
          </span>
        )}
      </Show>
    </div>
  );
}
