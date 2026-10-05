import { Dialog as K } from "@kobalte/core/dialog";
import { X } from "lucide-solid";
import { Show, type JSX } from "solid-js";
import { t } from "../app/strings";

export function Dialog(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: JSX.Element;
  footer?: JSX.Element;
  width?: string;
}) {
  return (
    <K open={props.open} onOpenChange={props.onOpenChange} modal>
      <K.Portal>
        <K.Overlay class="fixed inset-0 z-40 bg-black/40" />
        <div class="fixed inset-0 z-50 flex items-center justify-center p-4">
          <K.Content
            class="flex max-h-full flex-col rounded-md border border-border bg-bg shadow-xl"
            style={{ width: props.width ?? "480px" }}
          >
            <div class="flex items-center justify-between border-b border-border px-4 py-2.5">
              <K.Title class="font-semibold">{props.title}</K.Title>
              <K.CloseButton class="rounded p-1 text-muted hover:bg-hover" aria-label={t.server.close}>
                <X size={14} />
              </K.CloseButton>
            </div>
            <div class="min-h-0 overflow-auto p-4">{props.children}</div>
            <Show when={props.footer}>
              <div class="flex justify-end gap-2 border-t border-border px-4 py-2.5">{props.footer}</div>
            </Show>
          </K.Content>
        </div>
      </K.Portal>
    </K>
  );
}
