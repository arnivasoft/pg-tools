// Şema karşılaştırma kurulumu (tembel parça; DiffTabs ile birlikte iner — bkz. diff-open.ts). Denetleyici bağımlılıkları
// burada gerçek modüllere bağlanır; `copyText` yalnız bu dosyada içe aktarılır. Kaydetme seçicisi ScriptDialog parçasıyla
// tembel iner (`@tauri-apps/plugin-dialog` bu parçaya girmez).
import { copyText } from "../../app/clipboard";
import { tsd } from "../../app/strings-diff";
import { diffApi } from "./diff-commands";
import { createDiffController, type DiffTabCtl } from "./diff-controller";
import type { DiffDeps } from "./diff-open";

export function startDiff(deps: DiffDeps): { ctl: DiffTabCtl; open: (nodeId: string) => Promise<void> } {
  const ctl = createDiffController({
    api: { ...diffApi, ddlTargetOf: deps.api.ddlTargetOf, ddlLookup: deps.api.ddlLookup },
    servers: () => deps.query.servers(),
    nodeLabel: (nodeId) => deps.tree().nodes.get(nodeId)?.label ?? null,
    connect: deps.connect,
    tabs: deps.tabs,
    openScript: deps.query.openScript,
    // Tıklanınca yüklenir: statik içe aktarma ortak parça yaratıp ana paketin ön yükleme listesini büyütürdü.
    openForAssistant: (input, intent) => import("../query/ai-intent").then((m) => m.openForAssistant(deps.query, input, intent)),
    reviewLabel: tsd.review,
    copy: copyText,
    notify: deps.notify,
    pick: () => import("./ScriptDialog").then((m) => m.picker),
    newId: () => crypto.randomUUID(),
    now: () => new Date(),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  });
  return { ctl, open: ctl.open };
}
