import { useEffect, useRef, useState } from "react";
import { isTerminalJobStatus, type JobStatus } from "./api/enums";
import type { SseExecutionFrame, SseJobStateFrame } from "./api/jobs";

export interface JobStreamState {
  frames: SseExecutionFrame[];
  jobStatus: JobStatus | null;
  jobError: string | null;
  connected: boolean;
  lastSeq: number;
}

export function useJobEventsSse(jobId: string | null): JobStreamState {
  const [frames, setFrames] = useState<SseExecutionFrame[]>([]);
  const [jobStatus, setJobStatus] = useState<JobStatus | null>(null);
  const [jobError, setJobError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [lastSeq, setLastSeq] = useState(0);
  const seqRef = useRef(0);

  useEffect(() => {
    setFrames([]);
    setJobStatus(null);
    setJobError(null);
    setConnected(false);
    setLastSeq(0);
    seqRef.current = 0;

    if (!jobId) {
      return undefined;
    }

    let disposed = false;
    let source: EventSource | null = null;
    let reconnectTimer: number | null = null;
    let reachedTerminal = false;

    const connect = () => {
      if (disposed || reachedTerminal) {
        return;
      }
      const url = `/api/v1/jobs/${encodeURIComponent(jobId)}/events?after_seq=${seqRef.current}`;
      const es = new EventSource(url, { withCredentials: true });
      source = es;

      es.onopen = () => {
        if (!disposed) {
          setConnected(true);
        }
      };

      es.addEventListener("execution", (evt: MessageEvent) => {
        if (disposed || typeof evt.data !== "string") {
          return;
        }
        try {
          const parsed = JSON.parse(evt.data) as SseExecutionFrame;
          if (typeof parsed.seq === "number" && parsed.seq > seqRef.current) {
            seqRef.current = parsed.seq;
            setLastSeq(parsed.seq);
            setFrames((prev) => [...prev, parsed]);
          }
        } catch {
          /* ignore malformed frame */
        }
      });

      es.addEventListener("job_state", (evt: MessageEvent) => {
        if (disposed || typeof evt.data !== "string") {
          return;
        }
        try {
          const parsed = JSON.parse(evt.data) as SseJobStateFrame;
          if (typeof parsed.seq === "number" && parsed.seq > seqRef.current) {
            seqRef.current = parsed.seq;
            setLastSeq(parsed.seq);
          }
          if (parsed.status) {
            setJobStatus(parsed.status);
            if (parsed.error !== undefined) {
              setJobError(parsed.error ?? null);
            }
            if (isTerminalJobStatus(parsed.status)) {
              reachedTerminal = true;
              setConnected(false);
              es.close();
            }
          }
        } catch {
          /* ignore malformed frame */
        }
      });

      es.onerror = () => {
        if (disposed || reachedTerminal) {
          return;
        }
        setConnected(false);
        es.close();
        reconnectTimer = window.setTimeout(() => {
          connect();
        }, 1200);
      };
    };

    connect();

    return () => {
      disposed = true;
      if (reconnectTimer !== null) {
        window.clearTimeout(reconnectTimer);
      }
      if (source) {
        source.close();
      }
    };
  }, [jobId]);

  return {
    frames,
    jobStatus,
    jobError,
    connected,
    lastSeq,
  };
}
