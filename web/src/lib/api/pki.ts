import { apiRequest } from "./client";

export interface CertificateItem {
  rel_path: string;
  cn: string;
  issuer: string;
  is_ca: boolean;
  sans: string[];
  not_before: string;
  not_after: string;
  days_remaining: number;
  has_matching_key: boolean;
}

export interface CertificatesListResponse {
  items: CertificateItem[];
}

export interface GenerateCertificatePayload {
  name: string;
  cn: string;
  is_ca: boolean;
  ca_name?: string | null;
  sans: string[];
  validity: string;
}

export interface GenerateCertificateResponse {
  cert_rel_path: string;
  key_rel_path: string;
  validity_hours: number;
}

export interface VerifyCertificatePayload {
  cert_rel_path: string;
  key_rel_path?: string | null;
  ca_rel_path?: string | null;
}

export interface VerifyCertificateResponse {
  valid: boolean;
  report: string;
}

export interface BackupItem {
  rel_path: string;
  size_bytes: number;
  modified_ms: number;
}

export interface BackupsListResponse {
  items: BackupItem[];
}

export function fetchCertificates(signal?: AbortSignal): Promise<CertificatesListResponse> {
  return apiRequest<CertificatesListResponse>("/api/v1/pki/certificates", { signal });
}

export function generateCertificate(payload: GenerateCertificatePayload): Promise<GenerateCertificateResponse> {
  return apiRequest<GenerateCertificateResponse>("/api/v1/pki/generate", {
    method: "POST",
    body: payload,
  });
}

export function verifyCertificate(payload: VerifyCertificatePayload): Promise<VerifyCertificateResponse> {
  return apiRequest<VerifyCertificateResponse>("/api/v1/pki/verify", {
    method: "POST",
    body: payload,
  });
}

export function fetchBackupsList(signal?: AbortSignal): Promise<BackupsListResponse> {
  return apiRequest<BackupsListResponse>("/api/v1/backups", { signal });
}

export function triggerBackupsArchiveDownload(): void {
  if (typeof window !== "undefined") {
    window.location.assign("/api/v1/backups/archive");
  }
}
