import { apiRequest } from "./client";
import type { JobConcurrencyClass, JobStatus } from "./enums";

export interface JobSummary {
  id: string;
  kind: string;
  title: string;
  concurrency_class: JobConcurrencyClass;
  status: JobStatus;
  created_at_ms: number;
  started_at_ms: number | null;
  finished_at_ms: number | null;
  error: string | null;
}

export interface HostTaskStats {
  ok: number;
  changed: number;
  failed: number;
  skipped: number;
}

export interface JobDetail extends JobSummary {
  host_stats: Record<string, HostTaskStats>;
}

export interface JobsListResponse {
  items: JobSummary[];
  next_cursor: string | null;
}

export interface CancelJobResponse {
  id: string;
  status: string;
}

export type ExecutionEvent =
  | {
      kind: "play_started";
      play_name: string;
      hosts: string[];
    }
  | {
      kind: "task_started";
      play_name: string;
      task_name: string;
      host: string;
    }
  | {
      kind: "task_progress";
      task_name: string;
      host: string;
      percent: number;
      detail: string;
    }
  | {
      kind: "task_finished";
      task_name: string;
      host: string;
      changed: boolean;
      failed: boolean;
      skipped: boolean;
      stdout: string;
      stderr: string;
      duration_ms: number;
    }
  | {
      kind: "artifact_progress";
      item: string;
      bytes_done: number;
      bytes_total: number | null;
      phase: string;
    }
  | {
      kind: "log_line";
      level: string;
      message: string;
    };

export interface SseExecutionFrame {
  seq: number;
  ts_ms: number;
  event: ExecutionEvent;
}

export interface SseJobStateFrame {
  seq: number;
  ts_ms: number;
  status: JobStatus;
  error?: string | null;
  exit_code?: number;
}

export function fetchJobsList(params?: { limit?: number; cursor?: string }, signal?: AbortSignal): Promise<JobsListResponse> {
  const query = new URLSearchParams();
  if (params?.limit !== undefined) {
    query.set("limit", String(params.limit));
  }
  if (params?.cursor) {
    query.set("cursor", params.cursor);
  }
  const suffix = query.toString() ? `?${query.toString()}` : "";
  return apiRequest<JobsListResponse>(`/api/v1/jobs${suffix}`, { signal });
}

export function fetchJobDetail(jobId: string, signal?: AbortSignal): Promise<JobDetail> {
  return apiRequest<JobDetail>(`/api/v1/jobs/${encodeURIComponent(jobId)}`, { signal });
}

export function cancelJobById(jobId: string): Promise<CancelJobResponse> {
  return apiRequest<CancelJobResponse>(`/api/v1/jobs/${encodeURIComponent(jobId)}/cancel`, {
    method: "POST",
  });
}
