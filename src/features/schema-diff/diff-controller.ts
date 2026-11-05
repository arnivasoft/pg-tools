// Şema karşılaştırma sekmelerinin denetleyici yüzü (tembel parça). Task 3 genişletir; şimdilik iskelet.

/** QueryTabs'ın, App'in ve sekme panellerinin gördüğü denetleyici yüzü. */
export interface DiffTabCtl {
  model(tabId: string): null;
  ids(): string[];
  close(tabId: string): void;
}
