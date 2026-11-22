import {
  Activity,
  Download,
  RefreshCw,
  RotateCcw,
  Upload,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "../components/primitives/Button";
import { DataTable, type DataTableColumn } from "../components/primitives/DataTable";
import { Drawer } from "../components/primitives/Drawer";
import { StateView } from "../components/primitives/StateView";
import { Tabs } from "../components/primitives/Tabs";
import { useToast } from "../components/primitives/Toast";
import {
  fetchArtifactsStatus,
  pingRegistry,
  startArtifactsDownload,
  startRegistryPull,
  startRegistryPush,
  type ArtifactsStatusResponse,
  type ImageArtifactItem,
  type RegistryPingResponse,
} from "../lib/api/artifacts";
import { ApiClientError } from "../lib/api/client";
import { isTerminalJobStatus } from "../lib/api/enums";
import { cancelJobById } from "../lib/api/jobs";
import { formatBytes, formatTimestampMs } from "../lib/format";
import { useJobEventsSse } from "../lib/sse";

interface ComponentRow {
  key: string;
  category: "二进制" | "归档包";
  name: string;
  version: string;
  rel_path: string;
  exists: boolean;
  optional: boolean;
  size_bytes: number;
  modified_ms: number | null;
}

export function ArtifactsPage() {
  const { pushToast } = useToast();
  const [statusData, setStatusData] = useState<ArtifactsStatusResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [activeTab, setActiveTab] = useState("components");
  const [imageQuery, setImageQuery] = useState("");
  const [selectedImages, setSelectedImages] = useState<Set<string>>(new Set());

  const [pingBusy, setPingBusy] = useState(false);
  const [pingResult, setPingResult] = useState<RegistryPingResponse | null>(null);

  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const stream = useJobEventsSse(activeJobId);

  const loadStatus = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetchArtifactsStatus();
      setStatusData(res);
    } catch (err) {
      setError(err instanceof ApiClientError ? `${err.message} (${err.code})` : "离线资源读取失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  useEffect(() => {
    if (stream.jobStatus && isTerminalJobStatus(stream.jobStatus)) {
      void loadStatus();
    }
  }, [stream.jobStatus, loadStatus]);

  const componentRows = useMemo<ComponentRow[]>(() => {
    if (!statusData) {
      return [];
    }
    const bins: ComponentRow[] = statusData.binaries.map((b) => ({
      key: `bin:${b.rel_path}`,
      category: "二进制",
      name: b.name,
      version: b.version,
      rel_path: b.rel_path,
      exists: b.exists,
      optional: false,
      size_bytes: b.size_bytes,
      modified_ms: b.modified_ms,
    }));
    const tars: ComponentRow[] = statusData.archives.map((a) => ({
      key: `tar:${a.rel_path}`,
      category: "归档包",
      name: a.name,
      version: a.version,
      rel_path: a.rel_path,
      exists: a.exists,
      optional: a.optional,
      size_bytes: a.size_bytes,
      modified_ms: a.modified_ms,
    }));
    return [...bins, ...tars];
  }, [statusData]);

  const visibleImages = useMemo<ImageArtifactItem[]>(() => {
    if (!statusData) {
      return [];
    }
    const q = imageQuery.trim().toLowerCase();
    if (!q) {
      return statusData.images;
    }
    return statusData.images.filter(
      (img) =>
        img.name.toLowerCase().includes(q) ||
        img.source.toLowerCase().includes(q) ||
        img.target.toLowerCase().includes(q),
    );
  }, [statusData, imageQuery]);

  const componentBytes = useMemo(
    () => componentRows.reduce((acc, r) => acc + (r.exists ? r.size_bytes : 0), 0),
    [componentRows],
  );

  const imageBytes = useMemo(
    () => (statusData?.images ?? []).reduce((acc, r) => acc + (r.exists ? r.size_bytes : 0), 0),
    [statusData],
  );

  const latestProgressMap = useMemo(() => {
    const map = new Map<string, { bytesDone: number; bytesTotal: number | null; phase: string }>();
    for (const frame of stream.frames) {
      if (frame.event.kind === "artifact_progress") {
        map.set(frame.event.item, {
          bytesDone: frame.event.bytes_done,
          bytesTotal: frame.event.bytes_total,
          phase: frame.event.phase,
        });
      }
    }
    return Array.from(map.entries());
  }, [stream.frames]);

  const handlePing = async () => {
    setPingBusy(true);
    try {
      const res = await pingRegistry(null);
      setPingResult(res);
      pushToast(res.reachable ? "success" : "error", res.reachable ? "私库可达" : "私库不可达");
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "私库探测失败");
    } finally {
      setPingBusy(false);
    }
  };

  const launchJob = async (fn: () => Promise<{ job_id: string }>, title: string) => {
    try {
      const res = await fn();
      setActiveJobId(res.job_id);
      setDrawerOpen(true);
      pushToast("info", `${title}已启动`);
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : `${title}启动失败`);
    }
  };

  const handleCancelActiveJob = async () => {
    if (!activeJobId) {
      return;
    }
    try {
      await cancelJobById(activeJobId);
      pushToast("info", "已请求取消");
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "取消失败");
    }
  };

  const componentColumns: DataTableColumn<ComponentRow>[] = [
    {
      key: "category",
      header: "类别",
      render: (r) => <span className="park-badge">{r.category}</span>,
    },
    {
      key: "name",
      header: "组件名",
      render: (r) => <span className="font-mono-code font-medium">{r.name}</span>,
    },
    {
      key: "version",
      header: "版本",
      tabularNums: true,
      render: (r) => <span className="font-mono-code text-xs">{r.version || "—"}</span>,
    },
    {
      key: "path",
      header: "落盘路径",
      render: (r) => (
        <span className="font-mono-code text-xs" style={{ color: "var(--text-muted)" }}>
          {r.rel_path}
        </span>
      ),
    },
    {
      key: "exists",
      header: "状态",
      render: (r) => (
        <span
          className="park-badge"
          style={{
            borderColor: r.exists
              ? "var(--success)"
              : r.optional
                ? "var(--border)"
                : "var(--warning)",
          }}
        >
          {r.exists ? "已就绪" : r.optional ? "可选项" : "缺失"}
        </span>
      ),
    },
    {
      key: "size",
      header: "体积",
      align: "right",
      tabularNums: true,
      render: (r) => (
        <span className="font-mono-code text-xs">{r.exists ? formatBytes(r.size_bytes) : "—"}</span>
      ),
    },
    {
      key: "mtime",
      header: "修改时间",
      align: "right",
      tabularNums: true,
      render: (r) => (
        <span className="font-mono-code text-xs" style={{ color: "var(--text-muted)" }}>
          {formatTimestampMs(r.modified_ms)}
        </span>
      ),
    },
  ];

  const imageColumns: DataTableColumn<ImageArtifactItem>[] = [
    {
      key: "name",
      header: "镜像标识",
      render: (r) => <span className="font-mono-code font-medium">{r.name}</span>,
    },
    {
      key: "source",
      header: "上游地址",
      render: (r) => (
        <span className="font-mono-code text-xs break-all" style={{ color: "var(--text-muted)" }}>
          {r.source}
        </span>
      ),
    },
    {
      key: "target",
      header: "目标私库地址",
      render: (r) => <span className="font-mono-code text-xs break-all">{r.target}</span>,
    },
    {
      key: "status",
      header: "归档状态",
      render: (r) => (
        <div className="flex flex-wrap items-center gap-1">
          <span
            className="park-badge"
            style={{ borderColor: r.exists ? "var(--success)" : "var(--warning)" }}
          >
            {r.exists ? "已缓存" : "未拉取"}
          </span>
          {r.exists && !r.repo_tags_matched ? (
            <span className="park-badge">跨库重标</span>
          ) : null}
          {r.exists && r.has_media_manifest ? (
            <span className="park-badge">OCI 元数据</span>
          ) : null}
        </div>
      ),
    },
    {
      key: "size",
      header: "体积",
      align: "right",
      tabularNums: true,
      render: (r) => (
        <span className="font-mono-code text-xs">{r.exists ? formatBytes(r.size_bytes) : "—"}</span>
      ),
    },
  ];

  const selectedImageArray = Array.from(selectedImages);

  return (
    <div className="space-y-4 max-w-6xl">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            icon={<Activity size={13} />}
            disabled={pingBusy}
            onClick={() => void handlePing()}
          >
            {pingBusy ? "探测中…" : "探测私库"}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<Download size={13} />}
            onClick={() =>
              void launchJob(
                () => startArtifactsDownload(["binaries", "archives", "charts", "debs"], false),
                "预下载",
              )
            }
          >
            下载缺失项
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<Download size={13} />}
            onClick={() =>
              void launchJob(
                () =>
                  startRegistryPull({
                    image_names: selectedImageArray,
                    force: false,
                    insecure: true,
                  }),
                "拉取镜像",
              )
            }
          >
            拉取镜像
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<RotateCcw size={13} />}
            onClick={() =>
              void launchJob(
                () =>
                  startRegistryPull({
                    image_names: selectedImageArray,
                    force: true,
                    insecure: true,
                  }),
                "强制重拉",
              )
            }
          >
            强制重拉
          </Button>
          <Button
            variant="primary"
            size="sm"
            icon={<Upload size={13} />}
            onClick={() => void launchJob(() => startRegistryPush(selectedImageArray), "推送私库")}
          >
            推送私库
          </Button>
        </div>

        <div className="flex items-center gap-2">
          {activeJobId ? (
            <Button variant="secondary" size="sm" onClick={() => setDrawerOpen(true)}>
              传输进度
            </Button>
          ) : null}
          <Button
            variant="secondary"
            size="sm"
            icon={<RefreshCw size={13} />}
            onClick={() => void loadStatus()}
          >
            刷新
          </Button>
        </div>
      </div>

      {pingResult ? (
        <div
          className="park-card p-3.5 flex flex-wrap items-center justify-between gap-2 text-xs"
          style={{
            borderColor: pingResult.reachable ? "var(--success)" : "var(--danger)",
          }}
        >
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-semibold">
              {pingResult.reachable ? "私库连通正常" : "私库连接异常"}
            </span>
            <span className="park-badge font-mono-code">
              协议 · {pingResult.resolved_scheme || "—"}
            </span>
            <span className="park-badge font-mono-code">
              鉴权 · {pingResult.auth_mode || "—"}
            </span>
            <span className="park-badge font-mono-code">
              {pingResult.is_harbor ? "Harbor v2" : "OCI v2"} · 项目 {pingResult.project || "—"} (
              {pingResult.project_exists ? "已存在" : "待创建"})
            </span>
          </div>
          <span className="tabular-nums font-mono-code" style={{ color: "var(--text-muted)" }}>
            {pingResult.latency_ms} ms {pingResult.detail ? `· ${pingResult.detail}` : ""}
          </span>
        </div>
      ) : null}

      <StateView loading={loading} error={error} onRetry={() => void loadStatus()}>
        {statusData ? (
          <div className="space-y-4">
            <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
              <div className="park-card p-4">
                <div className="text-xs" style={{ color: "var(--text-muted)" }}>
                  二进制与归档
                </div>
                <div className="mt-1 text-xl font-semibold tabular-nums font-mono-code">
                  {statusData.summary.binaries_ready + statusData.summary.archives_ready} /{" "}
                  {statusData.summary.binaries_total + statusData.summary.archives_total} ·{" "}
                  {formatBytes(componentBytes)}
                </div>
              </div>

              <div className="park-card p-4">
                <div className="text-xs" style={{ color: "var(--text-muted)" }}>
                  Debian 13 依赖包
                </div>
                <div className="mt-1 text-xl font-semibold">
                  {statusData.summary.debs_bundle_ready ? "离线包就绪" : "在线 APT 模式"}
                </div>
              </div>

              <div className="park-card p-4">
                <div className="text-xs" style={{ color: "var(--text-muted)" }}>
                  容器镜像归档
                </div>
                <div className="mt-1 text-xl font-semibold tabular-nums font-mono-code">
                  {statusData.summary.images_ready} / {statusData.summary.images_total} ·{" "}
                  {formatBytes(imageBytes)}
                </div>
              </div>
            </div>

            <Tabs
              value={activeTab}
              onChange={setActiveTab}
              items={[
                {
                  value: "components",
                  label: "组件与离线包",
                  badge: componentRows.length,
                  content: (
                    <DataTable
                      rows={componentRows}
                      getRowKey={(r) => r.key}
                      columns={componentColumns}
                      emptyText="暂无组件条目"
                    />
                  ),
                },
                {
                  value: "images",
                  label: "容器镜像清单",
                  badge: statusData.images.length,
                  content: (
                    <div className="space-y-3">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <input
                          type="search"
                          value={imageQuery}
                          onChange={(e) => setImageQuery(e.target.value)}
                          placeholder="搜索镜像名 / 仓库路径"
                          aria-label="搜索镜像"
                          className="park-input w-64"
                        />
                        <span className="text-xs tabular-nums" style={{ color: "var(--text-muted)" }}>
                          {selectedImages.size > 0
                            ? `已勾选 ${selectedImages.size} 项（子集执行）`
                            : "未勾选时默认作用于全量清单"}
                        </span>
                      </div>
                      <DataTable
                        rows={visibleImages}
                        getRowKey={(r) => r.name}
                        columns={imageColumns}
                        selection={{
                          selectedKeys: selectedImages,
                          onChangeSelectedKeys: setSelectedImages,
                        }}
                        emptyText="暂无镜像条目"
                      />
                    </div>
                  ),
                },
              ]}
            />
          </div>
        ) : null}
      </StateView>

      <Drawer
        open={drawerOpen}
        onOpenChange={setDrawerOpen}
        title="传输任务流"
        side="right"
        widthClass="max-w-lg"
        footer={
          stream.jobStatus && !isTerminalJobStatus(stream.jobStatus) ? (
            <Button
              variant="danger"
              size="sm"
              icon={<XCircle size={13} />}
              onClick={() => void handleCancelActiveJob()}
            >
              取消任务
            </Button>
          ) : (
            <Button variant="secondary" size="sm" onClick={() => setDrawerOpen(false)}>
              关闭
            </Button>
          )
        }
      >
        <div className="space-y-4 text-xs">
          <div className="flex items-center justify-between">
            <span className="font-mono-code">{activeJobId ?? "—"}</span>
            <span className="park-badge">{stream.jobStatus ?? (stream.connected ? "运行中" : "等待中")}</span>
          </div>

          {latestProgressMap.length > 0 ? (
            <div className="space-y-2">
              {latestProgressMap.map(([item, prog]) => {
                const pct =
                  prog.bytesTotal && prog.bytesTotal > 0
                    ? Math.min(100, Math.round((prog.bytesDone / prog.bytesTotal) * 100))
                    : null;
                return (
                  <div key={item} className="park-card p-2.5 space-y-1.5">
                    <div className="flex items-center justify-between gap-2 font-mono-code tabular-nums">
                      <span className="truncate">{item}</span>
                      <span>
                        {formatBytes(prog.bytesDone)}
                        {prog.bytesTotal ? ` / ${formatBytes(prog.bytesTotal)}` : ""}
                      </span>
                    </div>
                    {pct !== null ? (
                      <div
                        className="w-full h-1.5 overflow-hidden"
                        style={{ backgroundColor: "var(--surface-2)", borderRadius: "99px" }}
                      >
                        <div
                          className="h-full"
                          style={{ width: `${pct}%`, backgroundColor: "var(--accent)" }}
                        />
                      </div>
                    ) : null}
                  </div>
                );
              })}
            </div>
          ) : null}

          <div
            className="park-panel p-3 max-h-96 overflow-y-auto font-mono-code space-y-1"
            style={{ backgroundColor: "var(--surface)" }}
          >
            {stream.frames.length === 0 ? (
              <div style={{ color: "var(--text-muted)" }}>等待事件帧…</div>
            ) : (
              stream.frames.map((frame) => (
                <div key={frame.seq} className="break-all leading-relaxed">
                  <span className="tabular-nums opacity-60 mr-2">#{frame.seq}</span>
                  {frame.event.kind === "log_line"
                    ? frame.event.message
                    : frame.event.kind === "task_finished"
                      ? `${frame.event.host} · ${frame.event.task_name} (${frame.event.duration_ms}ms)`
                      : frame.event.kind === "artifact_progress"
                        ? `${frame.event.phase} · ${frame.event.item} (${formatBytes(frame.event.bytes_done)})`
                        : `${frame.event.kind}`}
                </div>
              ))
            )}
          </div>
        </div>
      </Drawer>
    </div>
  );
}
