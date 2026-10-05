import { readStorage, writeStorage } from "./storage";

export type ThemePreference = "system" | "light" | "dark";
const KEY = "pgt.theme";

export function resolveTheme(pref: ThemePreference, systemDark: boolean): "light" | "dark" {
  return pref === "system" ? (systemDark ? "dark" : "light") : pref;
}

export function nextPreference(pref: ThemePreference): ThemePreference {
  return pref === "system" ? "light" : pref === "light" ? "dark" : "system";
}

export function loadThemePreference(): ThemePreference {
  const value = readStorage(KEY);
  return value === "light" || value === "dark" ? value : "system";
}

export function saveThemePreference(pref: ThemePreference): void {
  writeStorage(KEY, pref);
}

const media = () => window.matchMedia("(prefers-color-scheme: dark)");

export function applyTheme(pref: ThemePreference): void {
  document.documentElement.dataset.theme = resolveTheme(pref, media().matches);
}

/** Sistem teması değişince (yalnızca "system" tercihinde etkili) yeniden uygular. */
export function watchSystemTheme(getPref: () => ThemePreference): () => void {
  const m = media();
  const handler = () => applyTheme(getPref());
  m.addEventListener("change", handler);
  return () => m.removeEventListener("change", handler);
}
