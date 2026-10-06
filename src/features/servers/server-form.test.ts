import { describe, expect, it } from "vitest";
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import { emptyForm, formFromConfig, groupsOf, isProductionLabel, suggestLock, toConfig } from "./server-form";

const ID = "11111111-1111-4111-8111-111111111111";

function filled() {
  return { ...emptyForm(), name: " prod ", host: "db.local", username: "app" };
}

describe("server-form", () => {
  it("has sensible defaults", () => {
    const f = emptyForm();
    expect([f.port, f.maintenanceDb, f.sslMode, f.savePassword]).toEqual(["5432", "postgres", "prefer", false]);
  });

  it("converts a valid form to a trimmed config with nulls for empty optionals", () => {
    const r = toConfig({ ...filled(), group: "  ", rootCert: "", password: "" }, ID);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.config).toEqual({
      id: ID, name: "prod", group: null, host: "db.local", port: 5432, maintenanceDb: "postgres", username: "app",
      savePassword: false, ssl: { mode: "prefer", rootCert: null, clientCert: null, clientKey: null },
      color: null, label: null, readOnlyLock: false,
    });
    expect(r.password).toBeNull();
  });

  it("passes a typed password through", () => {
    const r = toConfig({ ...filled(), password: "s3cret" }, ID);
    expect(r.ok && r.password).toBe("s3cret");
  });

  it("reports the tab of the first invalid field", () => {
    expect(toConfig({ ...filled(), name: "" }, ID)).toMatchObject({ ok: false, tab: "general" });
    expect(toConfig({ ...filled(), host: " " }, ID)).toMatchObject({ ok: false, tab: "connection" });
    expect(toConfig({ ...filled(), port: "70000" }, ID)).toMatchObject({ ok: false, tab: "connection" });
    expect(toConfig({ ...filled(), port: "54a" }, ID)).toMatchObject({ ok: false, tab: "connection" });
    expect(toConfig({ ...filled(), username: "" }, ID)).toMatchObject({ ok: false, tab: "connection" });
    expect(toConfig({ ...filled(), clientCert: "c.pem" }, ID)).toMatchObject({ ok: false, tab: "ssl" });
  });

  it("round-trips an existing config without the password", () => {
    const config: ServerConfig = {
      id: ID, name: "p", group: "Üretim", host: "h", port: 6432, maintenanceDb: "postgres", username: "u",
      savePassword: true, ssl: { mode: "verify-full", rootCert: "C:\ca.pem", clientCert: null, clientKey: null },
      color: "red", label: "Üretim", readOnlyLock: true,
    };
    const form = formFromConfig(config);
    expect(form.password).toBe("");
    const r = toConfig(form, ID);
    expect(r.ok && r.config).toEqual(config);
  });

  it("lists distinct groups alphabetically", () => {
    const base = { id: ID, host: "h", port: 1, maintenanceDb: "postgres", username: "u", savePassword: false,
      ssl: { mode: "prefer" as const, rootCert: null, clientCert: null, clientKey: null },
      color: null, label: null, readOnlyLock: false };
    const servers: ServerConfig[] = [
      { ...base, name: "a", group: "Test" }, { ...base, name: "b", group: "Üretim" }, { ...base, name: "c", group: "Test" }, { ...base, name: "d", group: null },
    ];
    expect(groupsOf(servers)).toEqual(["Test", "Üretim"]);
  });
});

describe("security fields", () => {
  it("recognises production labels like the backend", () => {
    for (const l of ["Üretim", "ÜRETİM", "uretim", "URETIM", "Production", " PROD ", "üretim", "PRODUCTİON"]) {
      expect(isProductionLabel(l), l).toBe(true);
    }
    for (const l of ["Test", "Geliştirme", "preprod", "", "   ", "üretimm"]) expect(isProductionLabel(l), l).toBe(false);
    expect(isProductionLabel(null)).toBe(false);
    // Birleşik nokta (U+0307) ve Türkçe ı/İ katlaması (İ/ı → i)
    expect(isProductionLabel("pr\u0307od")).toBe(true);
    expect(isProductionLabel("ÜRET\u0130M")).toBe(true);
  });

  it("auto-locks once when the label becomes production", () => {
    expect(suggestLock("", "Üretim", false, false)).toBe(true);
    expect(suggestLock("Test", "prod", false, false)).toBe(true);
    // Zaten üretim olan etikette (düzenleme) ya da üretim olmayan değerde dokunmaz
    expect(suggestLock("Üretim", "Üretim", false, false)).toBe(false);
    expect(suggestLock("", "Test", false, false)).toBe(false);
    // Etiket üretimden çıkınca kilit açık kalır (kullanıcı kaldırabilir)
    expect(suggestLock("Üretim", "Test", true, false)).toBe(true);
  });

  it("does not force the lock again after the user touched it", () => {
    expect(suggestLock("", "Üretim", false, true)).toBe(false);
    expect(suggestLock("Test", "Üretim", false, true)).toBe(false);
    expect(suggestLock("", "Üretim", true, true)).toBe(true);
  });

  it("round-trips colour, label and lock", () => {
    const base: ServerConfig = {
      id: ID, name: "n", group: null, host: "h", port: 5432, maintenanceDb: "postgres", username: "u", savePassword: false,
      ssl: { mode: "prefer", rootCert: null, clientCert: null, clientKey: null }, color: "red", label: "Üretim", readOnlyLock: true,
    };
    const form = formFromConfig(base);
    expect([form.color, form.label, form.readOnlyLock]).toEqual(["red", "Üretim", true]);
    const r = toConfig(form, ID);
    expect(r.ok && r.config).toEqual(base);
    const none = toConfig({ ...form, color: null, label: "  ", readOnlyLock: false }, ID);
    expect(none.ok && [none.config.color, none.config.label, none.config.readOnlyLock]).toEqual([null, null, false]);
  });
});
