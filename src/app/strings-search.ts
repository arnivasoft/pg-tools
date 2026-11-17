// Nesne arama penceresinin metinleri (Plan 32). Yalnızca tembel arama parçası içe aktarır; ana pakette durmasın diye
// ayrı modüldedir.
import type { SearchKind } from "../ipc/bindings/SearchKind";

const kinds: Record<SearchKind, string> = {
  schema: "Şema",
  table: "Tablo",
  partition: "Bölüm",
  view: "View",
  matview: "Materialized view",
  function: "Fonksiyon",
  procedure: "Procedure",
  triggerFunction: "Trigger fonksiyonu",
  sequence: "Sequence",
  type: "Tip",
  domain: "Domain",
  column: "Kolon",
  index: "İndeks",
  constraint: "Kısıt",
  trigger: "Trigger",
  policy: "Policy",
  extension: "Extension",
};

export const ts = {
  title: (db: string, server: string) => `Nesne ara — ${db} (${server})`,
  placeholder: "Ad ara…",
  hint: "En az bir karakter yazın.",
  searching: "Aranıyor…",
  none: "Sonuç yok.",
  count: (n: number) => `${n} sonuç`,
  truncated: "İlk 500 sonuç gösteriliyor; aramayı daraltın.",
  tooLong: "Arama en fazla 200 karakter olabilir.",
  all: "Tümü",
  kinds,
  notFound: "Nesne ağaçta bulunamadı; listeyi yenileyin.",
  listLabel: "Arama sonuçları",
  filterLabel: "Türler",
};
