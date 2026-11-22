import { apiRequest } from "./client";

export interface RenderedTemplateFile {
  name: string;
  source_template: string;
  format: string;
  content: string;
  syntax_valid: boolean;
  validation_error: string | null;
}

export interface RenderTemplatesResponse {
  host: string;
  files: RenderedTemplateFile[];
}

export function renderHostTemplates(host: string, templates: string[] = ["all"]): Promise<RenderTemplatesResponse> {
  return apiRequest<RenderTemplatesResponse>("/api/v1/templates/render", {
    method: "POST",
    body: { host, templates },
  });
}
