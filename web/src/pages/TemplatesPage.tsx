import { CheckCircle2, Copy, RefreshCw, XCircle } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "../components/primitives/Button";
import { Select, type SelectOption } from "../components/primitives/Select";
import { StateView } from "../components/primitives/StateView";
import { useToast } from "../components/primitives/Toast";
import { ApiClientError } from "../lib/api/client";
import { fetchInventory } from "../lib/api/inventory";
import { renderHostTemplates, type RenderedTemplateFile } from "../lib/api/templates";
import { copyTextToClipboard } from "../lib/clipboard";
import { formatBytes } from "../lib/format";

export interface TemplatesPageProps {
  onNavigate: (path: string) => void;
}

export function TemplatesPage({ onNavigate }: TemplatesPageProps) {
  const { pushToast } = useToast();
  const [hosts, setHosts] = useState<SelectOption[]>([]);
  const [selectedHost, setSelectedHost] = useState("");
  const [files, setFiles] = useState<RenderedTemplateFile[]>([]);
  const [activeFileName, setActiveFileName] = useState("");
  const [loadingHosts, setLoadingHosts] = useState(true);
  const [rendering, setRendering] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const runRender = useCallback(async (host: string) => {
    if (!host) {
      return;
    }
    setRendering(true);
    setError(null);
    try {
      const res = await renderHostTemplates(host, ["all"]);
      setFiles(res.files);
      setActiveFileName((prev) => {
        if (prev && res.files.some((f) => f.name === prev)) {
          return prev;
        }
        return res.files[0]?.name ?? "";
      });
    } catch (err) {
      setError(err instanceof ApiClientError ? `${err.message} (${err.code})` : "模板渲染失败");
    } finally {
      setRendering(false);
    }
  }, []);

  const loadHostsAndRender = useCallback(async () => {
    setLoadingHosts(true);
    setError(null);
    try {
      const invRes = await fetchInventory();
      const rawHosts = invRes.inventory["hosts"];
      const entries: SelectOption[] = [];
      if (typeof rawHosts === "object" && rawHosts !== null && !Array.isArray(rawHosts)) {
        for (const [hostname, val] of Object.entries(rawHosts as Record<string, unknown>)) {
          let ipDesc = "";
          if (typeof val === "string") {
            ipDesc = val;
          } else if (typeof val === "object" && val !== null) {
            const rec = val as Record<string, unknown>;
            ipDesc =
              typeof rec["ip"] === "string"
                ? rec["ip"]
                : typeof rec["ansible_host"] === "string"
                  ? rec["ansible_host"]
                  : "";
          }
          entries.push({
            value: hostname,
            label: ipDesc ? `${hostname} (${ipDesc})` : hostname,
          });
        }
      }
      setHosts(entries);
      const initialHost = entries[0]?.value ?? "";
      setSelectedHost(initialHost);
      if (initialHost) {
        await runRender(initialHost);
      } else {
        setFiles([]);
        setActiveFileName("");
      }
    } catch (err) {
      setError(err instanceof ApiClientError ? `${err.message} (${err.code})` : "节点清单读取失败");
    } finally {
      setLoadingHosts(false);
    }
  }, [runRender]);

  useEffect(() => {
    void loadHostsAndRender();
  }, [loadHostsAndRender]);

  const activeFile = useMemo(
    () => files.find((f) => f.name === activeFileName) ?? files[0] ?? null,
    [files, activeFileName],
  );

  const handleHostChange = (nextHost: string) => {
    setSelectedHost(nextHost);
    void runRender(nextHost);
  };

  const handleCopy = async () => {
    if (!activeFile) {
      return;
    }
    const ok = await copyTextToClipboard(activeFile.content);
    pushToast(ok ? "success" : "error", ok ? "已复制" : "复制失败");
  };

  return (
    <div className="space-y-4 max-w-6xl">
      <div className="park-card p-3.5 flex flex-wrap items-end justify-between gap-3">
        <div className="w-full sm:w-72">
          <Select
            label="目标节点"
            value={selectedHost}
            options={hosts}
            disabled={hosts.length === 0 || rendering}
            onChange={handleHostChange}
          />
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            icon={<Copy size={13} />}
            disabled={!activeFile}
            onClick={() => void handleCopy()}
          >
            复制
          </Button>
          <Button
            variant="primary"
            size="sm"
            icon={<RefreshCw size={13} />}
            disabled={!selectedHost || rendering}
            onClick={() => void runRender(selectedHost)}
          >
            渲染
          </Button>
        </div>
      </div>

      <StateView
        loading={loadingHosts || rendering}
        error={error}
        empty={hosts.length === 0}
        emptyTitle="暂无可渲染节点"
        emptyActionLabel="集群配置"
        onEmptyAction={() => onNavigate("/inventory")}
        onRetry={() => void loadHostsAndRender()}
      >
        {files.length > 0 ? (
          <div className="grid grid-cols-1 lg:grid-cols-12 gap-4">
            <div className="lg:col-span-4 space-y-1.5">
              {files.map((file) => {
                const selected = file.name === activeFile?.name;
                const byteCount = new TextEncoder().encode(file.content).length;
                return (
                  <button
                    key={file.name}
                    type="button"
                    onClick={() => setActiveFileName(file.name)}
                    className="w-full p-2.5 border text-left flex items-center justify-between gap-2 cursor-pointer transition-colors"
                    style={{
                      borderRadius: "var(--radius-md)",
                      backgroundColor: selected ? "var(--accent)" : "var(--surface)",
                      color: selected ? "var(--accent-fg)" : "var(--text)",
                      borderColor: selected ? "var(--accent)" : "var(--border)",
                    }}
                  >
                    <div className="min-w-0">
                      <div className="font-mono-code text-xs font-semibold truncate">{file.name}</div>
                      <div
                        className="text-[11px] tabular-nums mt-0.5 truncate"
                        style={{ color: selected ? "var(--accent-fg)" : "var(--text-muted)" }}
                      >
                        {file.format.toUpperCase()} · {formatBytes(byteCount)}
                      </div>
                    </div>
                    <span className="shrink-0 inline-flex items-center gap-1 text-[11px]">
                      {file.syntax_valid ? (
                        <CheckCircle2
                          size={14}
                          style={{ color: selected ? "var(--accent-fg)" : "var(--success)" }}
                        />
                      ) : (
                        <XCircle
                          size={14}
                          style={{ color: selected ? "var(--accent-fg)" : "var(--danger)" }}
                        />
                      )}
                    </span>
                  </button>
                );
              })}
            </div>

            <div className="lg:col-span-8">
              {activeFile ? (
                <div className="park-panel overflow-hidden">
                  <div
                    className="px-3.5 py-2.5 border-b flex flex-wrap items-center justify-between gap-2"
                    style={{ backgroundColor: "var(--surface)", borderColor: "var(--border)" }}
                  >
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="font-mono-code text-xs font-semibold truncate">
                        {activeFile.name}
                      </span>
                      <span className="text-[11px] font-mono-code" style={{ color: "var(--text-muted)" }}>
                        ({activeFile.source_template})
                      </span>
                    </div>
                    <span
                      className="park-badge"
                      style={{
                        borderColor: activeFile.syntax_valid ? "var(--success)" : "var(--danger)",
                      }}
                    >
                      {activeFile.syntax_valid ? "语法合法" : "语法异常"}
                    </span>
                  </div>
                  {activeFile.validation_error ? (
                    <div
                      className="px-3.5 py-2 text-xs border-b font-mono-code"
                      style={{
                        borderColor: "var(--danger)",
                        color: "var(--danger)",
                        backgroundColor: "var(--surface)",
                      }}
                    >
                      {activeFile.validation_error}
                    </div>
                  ) : null}
                  <pre
                    className="m-0 p-3.5 overflow-x-auto max-h-[560px] overflow-y-auto font-mono-code text-xs leading-relaxed"
                    style={{ backgroundColor: "var(--bg)", color: "var(--text)" }}
                  >
                    <code>{activeFile.content}</code>
                  </pre>
                </div>
              ) : null}
            </div>
          </div>
        ) : null}
      </StateView>
    </div>
  );
}
