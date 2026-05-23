import { invoke } from "@tauri-apps/api/core";
import type { AuditRecord, HostKeyStore, McpGrant, SessionEvent, SessionSummary, TransferTask } from "./types";

export const isBackendAvailable = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function callBackend<T>(command: string, args: Record<string, unknown>, fallback: T): Promise<T> {
  if (!isBackendAvailable()) {
    return fallback;
  }
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    console.warn(`PortMate backend command failed: ${command}`, error);
    return fallback;
  }
}

export async function invokeBackend<T>(command: string, args: Record<string, unknown>): Promise<T> {
  if (!isBackendAvailable()) {
    throw new Error("PortMate desktop backend is not available in browser preview.");
  }
  return invoke<T>(command, args);
}

export const emptySessions: SessionSummary[] = [];
export const emptyLogs: Record<string, SessionEvent[]> = {};
export const emptyTransfers: TransferTask[] = [];
export const emptyAudit: AuditRecord[] = [];
export const emptyGrants: McpGrant[] = [];
export const emptyHostKeys: HostKeyStore = { keys: [] };
