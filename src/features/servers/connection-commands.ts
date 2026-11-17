// Sunucu diyaloğuna (ServerDialog tembel parçası) özgü "Bağlantıyı sına" komutu. Ana paketteki commands.ts'ten taşındı
// (Plan 32 telafisi). Kaydetme (`saveServer`) sorgu denetleyicisi de kullandığı için commands.ts'te kalır.
import type { ServerConfig } from "../../ipc/bindings/ServerConfig";
import type { ServerInfo } from "../../ipc/bindings/ServerInfo";
import { call } from "../../ipc/commands";

export const testConnection = (config: ServerConfig, password: string | null) =>
  call<ServerInfo>("test_connection", { config, password });
