import { describe, expect, it } from "vitest";
import type { JobSummary } from "../../ipc/bindings/JobSummary";
import { liveTexts, type LiveInput } from "./job-live";

const base: LiveInput = { phase: "ready", loadError: null, closing: false, closeAsk: false, summary: null, mode: "backup", singleTransaction: false, startError: null };
const sum = (o: Partial<JobSummary>): JobSummary => ({
  outcome: "success", exitCode: 0, elapsedMs: 65000, warnings: 0, errors: 0, outputPath: null, hint: null, truncatedLines: 0, ...o,
});

describe("liveTexts", () => {
  it("yüklenirken durum, bağlantı hatasında uyarı", () => {
    expect(liveTexts({ ...base, phase: "loading" }).status).toContain("bağlanılıyor");
    const a = liveTexts({ ...base, phase: "loadError", loadError: "yok" });
    expect(a.alert).toContain("yok");
    expect(a.status).toBe("");
  });
  it("başarı status, hata alert olarak duyurulur", () => {
    const ok = liveTexts({ ...base, summary: sum({}) });
    expect(ok.status).toContain("Tamamlandı");
    expect(ok.alert).toBe("");
    const bad = liveTexts({ ...base, summary: sum({ outcome: "failed", exitCode: 1, errors: 2 }) });
    expect(bad.alert).toContain("Başarısız");
    expect(bad.status).toBe("");
  });
  it("çalışırken eski sonuç duyurulmaz; kapatma sorusu ve başlatma hatası uyarıdır", () => {
    const r = liveTexts({ ...base, phase: "running", summary: sum({}), closeAsk: true, closing: true });
    expect(r.status).not.toContain("Tamamlandı");
    expect(r.alert).toContain("İş sürüyor");
    expect(liveTexts({ ...base, startError: "x" }).alert).toContain("x");
  });
});
