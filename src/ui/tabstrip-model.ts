// TabStrip'in saf mantığı (DOM'suz; vitest ortamı node).
export type TabKey = "ArrowLeft" | "ArrowRight" | "Home" | "End";

/** Klavye tuşuna göre hedef sekme dizini; uçlarda sarar. Sekme yoksa 0. */
export function nextTabIndex(current: number, count: number, key: TabKey): number {
  if (count <= 0) return 0;
  switch (key) {
    case "Home":
      return 0;
    case "End":
      return count - 1;
    case "ArrowRight":
      return (current + 1) % count;
    case "ArrowLeft":
      return (current - 1 + count) % count;
  }
}

export const tabId = (base: string, value: string): string => `${base}-tab-${value}`;
export const panelId = (base: string, value: string): string => `${base}-panel-${value}`;
