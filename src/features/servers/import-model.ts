import { t } from "../../app/strings";
import type { ImportItem } from "../../ipc/bindings/ImportItem";

export const UNGROUPED: string = t.import.ungrouped;

const importable = (i: ImportItem) => i.error === null;

export function defaultSelection(items: ImportItem[]): Set<number> {
  return new Set(items.filter((i) => importable(i) && !i.duplicate).map((i) => i.index));
}
export function canImport(items: ImportItem[], selected: Set<number>): boolean {
  return items.some((i) => importable(i) && selected.has(i.index));
}
export function summarize(items: ImportItem[]) {
  return {
    total: items.length,
    importable: items.filter(importable).length,
    duplicates: items.filter((i) => i.duplicate).length,
    errors: items.filter((i) => !importable(i)).length,
  };
}
export function groupLabel(item: ImportItem): string {
  return item.group ?? UNGROUPED;
}
