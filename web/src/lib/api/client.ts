export interface ApiErrorEnvelope {
  code: string;
  message: string;
  request_id: string;
}

export class ApiClientError extends Error {
  public readonly status: number;
  public readonly code: string;
  public readonly requestId: string;

  public constructor(status: number, envelope: ApiErrorEnvelope) {
    super(envelope.message);
    this.name = "ApiClientError";
    this.status = status;
    this.code = envelope.code;
    this.requestId = envelope.request_id;
  }
}

type UnauthorizedHandler = () => void;

let unauthorizedHandler: UnauthorizedHandler | null = null;

export function setUnauthorizedHandler(handler: UnauthorizedHandler | null): void {
  unauthorizedHandler = handler;
}

function isErrorEnvelope(val: unknown): val is ApiErrorEnvelope {
  if (typeof val !== "object" || val === null) {
    return false;
  }
  const rec = val as Record<string, unknown>;
  return typeof rec["code"] === "string" && typeof rec["message"] === "string";
}

export interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "DELETE";
  body?: unknown;
  signal?: AbortSignal;
  suppressUnauthorizedRedirect?: boolean;
}

export async function apiRequest<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = "GET", body, signal, suppressUnauthorizedRedirect = false } = options;
  const headers: Record<string, string> = {
    Accept: "application/json",
  };
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
  }

  const response = await fetch(path, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
    credentials: "same-origin",
    signal,
  });

  const requestIdHeader = response.headers.get("X-Request-Id") ?? "";

  if (!response.ok) {
    let parsedBody: unknown = null;
    try {
      parsedBody = await response.json();
    } catch {
      parsedBody = null;
    }

    const envelope: ApiErrorEnvelope = isErrorEnvelope(parsedBody)
      ? {
          code: parsedBody.code,
          message: parsedBody.message,
          request_id:
            typeof parsedBody.request_id === "string" && parsedBody.request_id
              ? parsedBody.request_id
              : requestIdHeader,
        }
      : {
          code: response.status === 401 ? "UNAUTHORIZED" : "INTERNAL_ERROR",
          message: `HTTP ${response.status}`,
          request_id: requestIdHeader,
        };

    if (response.status === 401 && !suppressUnauthorizedRedirect && unauthorizedHandler) {
      unauthorizedHandler();
    }

    throw new ApiClientError(response.status, envelope);
  }

  return (await response.json()) as T;
}
