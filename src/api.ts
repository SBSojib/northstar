import { invoke } from "@tauri-apps/api/core";
import type { GroupInput, HostInput, Library } from "./types";

export const api = {
  getLibrary: (query = "") => invoke<Library>("get_library", { query }),
  saveGroup: (input: GroupInput) => invoke<number>("save_group", { input }),
  deleteGroup: (id: number) => invoke<void>("delete_group", { id }),
  saveHost: (input: HostInput) => invoke<number>("save_host", { input }),
  deleteHost: (id: number) => invoke<void>("delete_host", { id }),
  backupLibrary: (path: string, password: string) =>
    invoke<void>("backup_library", { path, password }),
  restoreLibrary: (path: string, password: string) =>
    invoke<string>("restore_library", { path, password }),
  connect: (hostId: number, sessionId: string, cols: number, rows: number) =>
    invoke<string>("connect_ssh", { hostId, sessionId, cols, rows }),
  write: (sessionId: string, data: number[]) =>
    invoke<void>("write_ssh", { sessionId, data }),
  resize: (sessionId: string, cols: number, rows: number) =>
    invoke<void>("resize_ssh", { sessionId, cols, rows }),
  disconnect: (sessionId: string) =>
    invoke<void>("disconnect_ssh", { sessionId }),
};

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
