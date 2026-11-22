import { apiRequest } from "./client";

export interface DerivedClusterVars {
  first_control_plane: string | null;
  etcd_endpoints_str: string;
  etcd_initial_cluster: string;
}

export interface InventoryResponse {
  inventory: Record<string, unknown>;
  groups: Record<string, string[]>;
  derived: DerivedClusterVars;
  masked_yaml: string;
}

export type InventoryWritePayload =
  | {
      mode: "structured";
      inventory: Record<string, unknown>;
    }
  | {
      mode: "yaml";
      raw_yaml: string;
    };

export interface PutInventoryResponse {
  saved: boolean;
  backup_path: string;
  derived: DerivedClusterVars;
}

export interface ValidateInventoryResponse {
  valid: boolean;
  hosts_count: number;
  groups?: Record<string, string[]>;
  derived: DerivedClusterVars;
  warnings: string[];
}

export interface HostProbeResult {
  host: string;
  ip: string;
  reachable: boolean;
  uid: number | null;
  os_pretty: string;
  arch: string;
  nics: string[];
  latency_ms: number;
  missing_packages: string[];
  error: string | null;
}

export interface ProbeInventoryResponse {
  results: HostProbeResult[];
}

export function fetchInventory(signal?: AbortSignal): Promise<InventoryResponse> {
  return apiRequest<InventoryResponse>("/api/v1/inventory", { signal });
}

export function saveInventory(payload: InventoryWritePayload): Promise<PutInventoryResponse> {
  return apiRequest<PutInventoryResponse>("/api/v1/inventory", {
    method: "PUT",
    body: payload,
  });
}

export function validateInventory(payload: InventoryWritePayload): Promise<ValidateInventoryResponse> {
  return apiRequest<ValidateInventoryResponse>("/api/v1/inventory/validate", {
    method: "POST",
    body: payload,
  });
}

export function probeInventoryHosts(hosts: string[]): Promise<ProbeInventoryResponse> {
  return apiRequest<ProbeInventoryResponse>("/api/v1/inventory/probe", {
    method: "POST",
    body: { hosts },
  });
}
