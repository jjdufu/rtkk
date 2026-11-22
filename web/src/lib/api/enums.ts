export type ConnectorType = "local" | "ssh";

export type BecomeMethod = "sudo" | "su";

export type HostKeyPolicy = "off" | "accept-new" | "strict";

export type CertPolicy = "always" | "ifnotpresent" | "none";

export type RegistryScheme = "auto" | "https" | "http";

export type CniPlugin = "cilium" | "custom" | "none";

export type JobStatus = "queued" | "running" | "succeeded" | "failed" | "cancelled";

export type JobConcurrencyClass = "mutating" | "readonly";

export type ExecutionEventKind =
  | "play_started"
  | "task_started"
  | "task_progress"
  | "task_finished"
  | "artifact_progress"
  | "log_line";

export type ApiErrorCode =
  | "INVALID_ARGUMENT"
  | "UNAUTHORIZED"
  | "NOT_FOUND"
  | "CONFLICT_ACTIVE_JOB"
  | "TEMPLATE_RENDER_ERROR"
  | "AUTH_RATE_LIMITED"
  | "INTERNAL_ERROR"
  | "UPSTREAM_UNREACHABLE";

export type PlaybookName = "deploy" | "cleanup" | "download";

export function isTerminalJobStatus(status: JobStatus | string): boolean {
  return status === "succeeded" || status === "failed" || status === "cancelled";
}
