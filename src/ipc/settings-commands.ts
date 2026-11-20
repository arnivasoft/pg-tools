// Ayarları okuma: yalnız tembel parçalar (QueryTab, yedekleme) kullanır. Ana paketteki commands.ts'ten taşındı (Plan 37 telafisi).
import type { AppSettings } from "./bindings/AppSettings";
import { call } from "./commands";

export const settingsCommands = { settingsGet: () => call<AppSettings>("settings_get") };
export type SettingsApi = typeof settingsCommands;
