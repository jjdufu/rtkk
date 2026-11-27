import { apiRequest } from "./client";

export interface HarborSpawnedJob {
  job_id: string;
  status: string;
}

export interface PublicRegistryMirrorItem {
  host: string;
  project: string;
}

export interface HarborProjectInfo {
  project_id: number;
  name: string;
  public: boolean;
  repo_count: number;
  registry_id: number | null;
  creation_time: string;
  mapped_hosts: string[];
  is_default_fallback: boolean;
  exists: boolean;
}

export interface HarborProjectsResponse {
  base_url: string;
  registry_host: string;
  is_harbor: boolean;
  mirrors_enabled: boolean;
  allow_fallback: boolean;
  default_fallback_project: string | null;
  projects: HarborProjectInfo[];
  missing_mirror_projects: string[];
  mirrors: PublicRegistryMirrorItem[];
}

export interface HarborProjectsSyncOutcome {
  base_url: string;
  created: string[];
  existing: string[];
  warnings: string[];
}

export interface HarborRepositoryInfo {
  id: number;
  name: string;
  short_name: string;
  artifact_count: number;
  pull_count: number;
  update_time: string;
}

export interface HarborRepositoriesResponse {
  project: string;
  repositories: HarborRepositoryInfo[];
}

export interface HarborTagInfo {
  name: string;
  push_time: string;
  pull_time: string;
}

export interface HarborArtifactInfo {
  digest: string;
  size: number;
  push_time: string;
  media_type: string;
  tags: HarborTagInfo[];
  architecture: string | null;
  os: string | null;
}

export interface HarborArtifactsResponse {
  project: string;
  repository: string;
  artifacts: HarborArtifactInfo[];
}

export interface TransparentMirrorTarget {
  raw_input: string;
  normalized_source: string;
  source_host: string;
  source_repository: string;
  tag: string;
  digest: string | null;
  target_project: string;
  target_repository: string;
  target_ref: string;
  matched_rule: string;
  used_default_fallback: boolean;
  project_exists: boolean | null;
  already_exists: boolean | null;
  error: string | null;
}

export interface HarborMirrorPlanResponse {
  registry_host: string;
  mirrors_enabled: boolean;
  allow_fallback: boolean;
  default_fallback_project: string | null;
  plans: TransparentMirrorTarget[];
}

export interface HarborMirrorImportPayload {
  images: string[];
  platform?: string;
  save_tar?: boolean;
  insecure?: boolean;
  force?: boolean;
  append_extra_images?: boolean;
}

export function fetchHarborProjects(): Promise<HarborProjectsResponse> {
  return apiRequest<HarborProjectsResponse>("/api/v1/harbor/projects");
}

export function syncHarborProjects(
  projects: string[] = [],
  strict = false,
): Promise<HarborProjectsSyncOutcome> {
  return apiRequest<HarborProjectsSyncOutcome>("/api/v1/harbor/projects/sync", {
    method: "POST",
    body: { projects, strict },
  });
}

export function fetchHarborRepositories(
  project: string,
  query = "",
): Promise<HarborRepositoriesResponse> {
  const qs = query.trim() ? `?q=${encodeURIComponent(query.trim())}` : "";
  return apiRequest<HarborRepositoriesResponse>(
    `/api/v1/harbor/projects/${encodeURIComponent(project)}/repositories${qs}`,
  );
}

export function fetchHarborArtifacts(
  project: string,
  repository: string,
): Promise<HarborArtifactsResponse> {
  const qs = `?repository=${encodeURIComponent(repository)}`;
  return apiRequest<HarborArtifactsResponse>(
    `/api/v1/harbor/projects/${encodeURIComponent(project)}/artifacts${qs}`,
  );
}

export function deleteHarborArtifact(
  project: string,
  repository: string,
  reference: string,
): Promise<{ deleted: boolean }> {
  const qs = `?repository=${encodeURIComponent(repository)}&reference=${encodeURIComponent(reference)}`;
  return apiRequest<{ deleted: boolean }>(
    `/api/v1/harbor/projects/${encodeURIComponent(project)}/artifacts${qs}`,
    { method: "DELETE" },
  );
}

export function planHarborMirror(
  images: string[],
  checkRemote = true,
): Promise<HarborMirrorPlanResponse> {
  return apiRequest<HarborMirrorPlanResponse>("/api/v1/harbor/mirror/plan", {
    method: "POST",
    body: { images, check_remote: checkRemote },
  });
}

export function startHarborMirrorImport(
  payload: HarborMirrorImportPayload,
): Promise<HarborSpawnedJob> {
  return apiRequest<HarborSpawnedJob>("/api/v1/harbor/mirror/import", {
    method: "POST",
    body: payload,
  });
}
