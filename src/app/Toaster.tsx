import { X } from "lucide-solid";
import { For } from "solid-js";
import { t } from "./strings";
import { toast } from "./toast";

export function Toaster() {
  return (
    // Persistent polite live region: it exists before any toast is added, so screen
    // readers reliably announce the toasts inserted into it (a live region created
    // together with its content is often missed). Items carry no role of their own
    // to avoid double announcements.
    <div class="fixed bottom-8 right-3 z-[60] flex flex-col gap-2" aria-live="polite" aria-relevant="additions text">
      <For each={toast.list()}>
        {(item) => (
          <div
            class="flex max-w-sm items-start gap-2 rounded border border-border bg-panel px-3 py-2 text-xs shadow-lg"
            classList={{ "text-danger": item.kind === "error" }}
          >
            <span class="min-w-0 flex-1 break-words">{item.text}</span>
            <button
              type="button"
              class="shrink-0 text-muted hover:text-current"
              aria-label={t.toast.dismiss}
              onClick={() => toast.dismiss(item.id)}
            >
              <X size={12} />
            </button>
          </div>
        )}
      </For>
    </div>
  );
}
