import { SplitPane } from "./SplitPane";

export default function App() {
  return (
    <div class="flex h-full flex-col">
      <header class="flex h-9 shrink-0 items-center gap-1 border-b border-border bg-panel px-2" />
      <SplitPane left={<div class="h-full" />} right={<div class="h-full" />} />
      <footer class="flex h-6 shrink-0 items-center border-t border-border bg-panel px-2 text-xs text-muted" />
    </div>
  );
}
