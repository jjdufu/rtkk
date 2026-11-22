import { Lock, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "../components/primitives/Button";
import { DataTable, type DataTableColumn } from "../components/primitives/DataTable";
import { StateView } from "../components/primitives/StateView";
import { fetchSystemOverview, type SystemOverviewResponse } from "../lib/api/auth";
import { ApiClientError } from "../lib/api/client";
import type { JobSummary } from "../lib/api/jobs";
import { formatDurationMs, formatTimestampMs } from "../lib/format";

export interface OverviewPageProps {
  onNavigate: (path: string) => void;
  onSelectJob: (jobId: string) => void;
}

function ratioPercent(ready: number, total: number): number {
  if (total <= 0) {
    return 0;
  }
  return Math.min(100, Math.round((ready / total) * 100));
}

function statusLabel(status: JobSummary["status"]): string {
  switch (status) {
    case "queued":
      return "排队中";
    case "running":
      return "运行中";
    case "succeeded":
      return "成功";
    case "failed":
      return "失败";
    case "cancelled":
      return "已取消";
  }
}

export function OverviewPage({ onNavigate, onSelectJob }: OverviewPageProps) {
  const [data, setData] = useState<SystemOverviewResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadOverview = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetchSystemOverview();
      setData(res);
    } catch (err) {
      setError(err instanceof ApiClientError ? `${err.message} (${err.code})` : "概览读取失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadOverview();
  }, [loadOverview]);

  const jobColumns: DataTableColumn<JobSummary>[] = [
    {
      key: "id",
      header: "任务 ID",
      tabularNums: true,
      render: (row) => (
        <button
          type="button"
          onClick={() => {
            onSelectJob(row.id);
            onNavigate("/operations");
          }}
          className="font-mono-code text-xs underline cursor-pointer"
          style={{ color: "var(--text)" }}
        >
          {row.id}
        </button>
      ),
    },
    {
      key: "title",
      header: "任务名称",
      render: (row) => <span className="font-medium">{row.title || row.kind}</span>,
    },
    {
      key: "class",
      header: "锁级别",
      render: (row) => (
        <span className="park-badge">{row.concurrency_class === "mutating" ? "变更锁" : "只读"}</span>
      ),
    },
    {
      key: "status",
      header: "状态",
      render: (row) => (
        <span
          className="park-badge"
          style={{
            borderColor:
              row.status === "failed"
                ? "var(--danger)"
                : row.status === "succeeded"
                  ? "var(--success)"
                  : "var(--border)",
          }}
        >
          {statusLabel(row.status)}
        </span>
      ),
    },
    {
      key: "created",
      header: "创建时间",
      tabularNums: true,
      render: (row) => (
        <span className="font-mono-code text-xs" style={{ color: "var(--text-muted)" }}>
          {formatTimestampMs(row.created_at_ms)}
        </span>
      ),
    },
    {
      key: "duration",
      header: "耗时",
      align: "right",
      tabularNums: true,
      render: (row) => {
        const dur =
          row.started_at_ms && row.finished_at_ms ? row.finished_at_ms - row.started_at_ms : null;
        return <span className="font-mono-code text-xs">{formatDurationMs(dur)}</span>;
      },
    },
  ];

  return (
    <div className="space-y-5 max-w-6xl">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          {data?.active_mutating_job_id ? (
            <span className="park-badge tabular-nums font-mono-code">
              <Lock size={12} />
              <span>活跃写锁 · {data.active_mutating_job_id}</span>
            </span>
          ) : (
            <span className="park-badge">写锁空闲</span>
          )}
          {data ? (
            <span className="park-badge">{data.workspace_initialized ? "工作区已初始化" : "待初始化"}</span>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="secondary" size="sm" onClick={() => onNavigate("/inventory")}>
            集群配置
          </Button>
          <Button variant="secondary" size="sm" onClick={() => onNavigate("/artifacts")}>
            离线资源
          </Button>
          <Button variant="primary" size="sm" onClick={() => onNavigate("/operations")}>
            编排执行
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<RefreshCw size={13} />}
            onClick={() => void loadOverview()}
          >
            刷新
          </Button>
        </div>
      </div>

      <StateView loading={loading} error={error} onRetry={() => void loadOverview()}>
        {data ? (
          <div className="space-y-5">
            <div className="grid grid-cols-2 lg:grid-cols-4 gap-3">
              <div className="park-card p-4">
                <div className="text-xs" style={{ color: "var(--text-muted)" }}>
                  集群节点
                </div>
                <div className="mt-1 text-2xl font-semibold tabular-nums">{data.counts.hosts}</div>
              </div>
              <div className="park-card p-4">
                <div className="text-xs" style={{ color: "var(--text-muted)" }}>
                  控制平面
                </div>
                <div className="mt-1 text-2xl font-semibold tabular-nums">
                  {data.counts.control_plane_hosts}
                </div>
              </div>
              <div className="park-card p-4">
                <div className="text-xs" style={{ color: "var(--text-muted)" }}>
                  工作节点
                </div>
                <div className="mt-1 text-2xl font-semibold tabular-nums">
                  {data.counts.worker_hosts}
                </div>
              </div>
              <div className="park-card p-4">
                <div className="text-xs" style={{ color: "var(--text-muted)" }}>
                  etcd 节点
                </div>
                <div className="mt-1 text-2xl font-semibold tabular-nums">
                  {data.counts.etcd_hosts}
                </div>
              </div>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
              {[
                {
                  title: "二进制组件",
                  ready: data.counts.binaries_ready,
                  total: data.counts.binaries_total,
                },
                {
                  title: "归档离线包",
                  ready: data.counts.archives_ready,
                  total: data.counts.archives_total,
                },
                {
                  title: "容器镜像",
                  ready: data.counts.images_ready,
                  total: data.counts.images_total,
                },
              ].map((item) => {
                const pct = ratioPercent(item.ready, item.total);
                return (
                  <div key={item.title} className="park-card p-4 space-y-2.5">
                    <div className="flex items-center justify-between text-xs">
                      <span className="font-medium">{item.title}</span>
                      <span className="tabular-nums font-mono-code" style={{ color: "var(--text-muted)" }}>
                        {item.ready} / {item.total} ({pct}%)
                      </span>
                    </div>
                    <div
                      className="w-full h-2 overflow-hidden"
                      style={{
                        backgroundColor: "var(--surface-2)",
                        borderRadius: "99px",
                      }}
                    >
                      <div
                        className="h-full transition-all"
                        style={{
                          width: `${pct}%`,
                          backgroundColor: "var(--accent)",
                          borderRadius: "99px",
                        }}
                      />
                    </div>
                  </div>
                );
              })}
            </div>

            <div className="grid grid-cols-2 gap-3">
              <div className="park-card p-4 flex items-center justify-between">
                <span className="text-xs font-medium" style={{ color: "var(--text-muted)" }}>
                  X.509 证书文件
                </span>
                <span className="text-lg font-semibold tabular-nums">{data.counts.certificates}</span>
              </div>
              <div className="park-card p-4 flex items-center justify-between">
                <span className="text-xs font-medium" style={{ color: "var(--text-muted)" }}>
                  状态备份文件
                </span>
                <span className="text-lg font-semibold tabular-nums">{data.counts.backups}</span>
              </div>
            </div>

            <div className="space-y-2.5">
              <h2 className="text-sm font-semibold m-0">最近任务</h2>
              <DataTable
                rows={data.recent_jobs}
                getRowKey={(row) => row.id}
                columns={jobColumns}
                emptyText="暂无任务记录"
                emptyAction={
                  <Button variant="primary" size="sm" onClick={() => onNavigate("/inventory")}>
                    集群配置
                  </Button>
                }
              />
            </div>
          </div>
        ) : null}
      </StateView>
    </div>
  );
}
