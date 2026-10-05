import type { EditorView } from "@codemirror/view";
import { createEffect, onCleanup, onMount } from "solid-js";
import { createReadOnlyEditor, replaceDoc } from "./codemirror-setup";

export default function DdlView(props: { text: string }) {
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  onMount(() => {
    view = createReadOnlyEditor(host, props.text);
  });
  createEffect(() => {
    const text = props.text;
    if (view && view.state.doc.toString() !== text) replaceDoc(view, text);
  });
  onCleanup(() => view?.destroy());
  return <div ref={host} class="h-full min-h-0" />;
}
