import { apiRequest } from "./client";

export interface ArtifactsSummary {
  binaries_ready: number;
  binaries_total: number;
  archives_ready: number;
  archives_total: number;
  debs_bundle_ready: boolean;
  images_ready: number;
  images_total: number;
  total_bytes: number;
}

export interface BinaryArtifactItem {
  name: string;
  version: string;
  rel_path: string;
  exists: boolean;
  size_bytes: number;
  modified_ms: number | null;
}

export interface ArchiveArtifactItem {
  name: string;
  version: string;
  rel_path: string;
  exists: boolean;
  size_bytes: number;
  modified_ms: number | null;
  optional: boolean;
}

export interface ImageArtifactItem {
  name: string;
  source: string;
  target: string;
  rel_path: string;
  exists: boolean;
  size_bytes: number;
  repo_tags_matched: boolean;
  has_media_manifest: boolean;
}

export interface ArtifactsStatusResponse {
  summary: ArtifactsSummary;
  binaries: BinaryArtifactItem[];
  archives: ArchiveArtifactItem[];
  images: ImageArtifactItem[];
}

export interface AsyncJobSpawnResponse {
  job_id: string;
  status: string;
}

export interface RegistryPingResponse {
  reachable: boolean;
  resolved_scheme: string;
  auth_mode: string;
  is_harbor: boolean;
  project: string;
  project_exists: boolean;
  latency_ms: number;
  detail?: string;
}

export function fetchArtifactsStatus(signal?: AbortSignal): Promise<ArtifactsStatusResponse> {
  return apiRequest<ArtifactsStatusResponse>("/api/v1/artifacts/status", { signal });
}

export function startArtifactsDownload(scopes: string[], force: boolean): Promise<AsyncJobSpawnResponse> {
  return apiRequest<AsyncJobSpawnResponse>("/api/v1/artifacts/download", {
    method: "POST",
    body: { scopes, force },
  });
}

export function pingRegistry(registryOverride: Record<string, unknown> | null = null): Promise<RegistryPingResponse> {
  return apiRequest<RegistryPingResponse>("/api/v1/registry/ping", {
    method: "POST",
    body: { registry_override: registryOverride },
  });
}

export function startRegistryPull(params: {
  image_names: string[];
  force: boolean;
  insecure: boolean;
}): Promise<AsyncJobSpawnResponse> {
  return apiRequest<AsyncJobSpawnResponse>("/api/v1/registry/pull", {
    method: "POST",
    body: params,
  });
}

export function startRegistryPush(imageNames: string[]): Promise<AsyncJobSpawnResponse> {
  return apiRequest<AsyncJobSpawnResponse>("/api/v1/registry/push", {
    method: "POST",
    body: { image_names: imageNames },
  });
}
