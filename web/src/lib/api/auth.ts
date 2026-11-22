import { apiRequest } from "./client";
import type { JobSummary } from "./jobs";

export interface HealthResponse {
  status: string;
  version: string;
  uptime_secs: number;
}

export interface LoginResponse {
  authenticated: boolean;
  expires_in_secs: number;
}

export interface LogoutResponse {
  authenticated: boolean;
}

export interface AuthMeResponse {
  authenticated: boolean;
  version: string;
  work_dir: string;
  bind_addr: string;
}

export interface SystemCounts {
  hosts: number;
  control_plane_hosts: number;
  worker_hosts: number;
  etcd_hosts: number;
  binaries_ready: number;
  binaries_total: number;
  archives_ready: number;
  archives_total: number;
  images_ready: number;
  images_total: number;
  certificates: number;
  backups: number;
}

export interface SystemOverviewResponse {
  workspace_initialized: boolean;
  active_mutating_job_id: string | null;
  recent_jobs: JobSummary[];
  counts: SystemCounts;
}

export function fetchHealth(signal?: AbortSignal): Promise<HealthResponse> {
  return apiRequest<HealthResponse>("/api/v1/health", {
    signal,
    suppressUnauthorizedRedirect: true,
  });
}

export function loginWithToken(token: string): Promise<LoginResponse> {
  return apiRequest<LoginResponse>("/api/v1/auth/login", {
    method: "POST",
    body: { token },
    suppressUnauthorizedRedirect: true,
  });
}

export function logoutSession(): Promise<LogoutResponse> {
  return apiRequest<LogoutResponse>("/api/v1/auth/logout", {
    method: "POST",
    suppressUnauthorizedRedirect: true,
  });
}

export function fetchAuthMe(signal?: AbortSignal): Promise<AuthMeResponse> {
  return apiRequest<AuthMeResponse>("/api/v1/auth/me", {
    signal,
    suppressUnauthorizedRedirect: true,
  });
}

export function fetchSystemOverview(signal?: AbortSignal): Promise<SystemOverviewResponse> {
  return apiRequest<SystemOverviewResponse>("/api/v1/system/overview", { signal });
}
