import type { AsyncJobSpawnResponse } from "./artifacts";
import { apiRequest } from "./client";
import type { PlaybookName } from "./enums";

export interface PlaybookOutlineTask {
  name: string;
  tags: string[];
}

export interface PlaybookOutlinePlay {
  name: string;
  hosts: string;
  tasks: PlaybookOutlineTask[];
}

export interface PlaybookOutlineResponse {
  name: string;
  all_tags: string[];
  plays: PlaybookOutlinePlay[];
}

export interface RunPlaybookPayload {
  playbook: PlaybookName;
  tags?: string[];
  forks?: number;
  extra_vars?: Record<string, unknown>;
}

export function fetchPlaybookOutline(name: PlaybookName, signal?: AbortSignal): Promise<PlaybookOutlineResponse> {
  return apiRequest<PlaybookOutlineResponse>(`/api/v1/playbooks/${encodeURIComponent(name)}/outline`, { signal });
}

export function startPrepHosts(bestEffort: boolean): Promise<AsyncJobSpawnResponse> {
  return apiRequest<AsyncJobSpawnResponse>("/api/v1/operations/prep-hosts", {
    method: "POST",
    body: { best_effort: bestEffort },
  });
}

export function startPlaybookOperation(payload: RunPlaybookPayload): Promise<AsyncJobSpawnResponse> {
  return apiRequest<AsyncJobSpawnResponse>("/api/v1/operations/run", {
    method: "POST",
    body: payload,
  });
}
