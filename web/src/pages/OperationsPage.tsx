import {
  ArrowDown,
  ListChecks,
  Play,
  RefreshCw,
  ShieldAlert,
  SlidersHorizontal,
  Terminal,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../components/primitives/Button";
import { Checkbox } from "../components/primitives/Checkbox";
import { ConfirmModal } from "../components/primitives/ConfirmModal";
import { Drawer } from "../components/primitives/Drawer";
import { Select, type SelectOption } from "../components/primitives/Select";
import { StateView } from "../components/primitives/StateView";
import { Switch } from "../components/primitives/Switch";
import { useToast } from "../components/primitives/Toast";
import { ApiClientError } from "../lib/api/client";
import { isTerminalJobStatus, type PlaybookName } from "../lib/api/enums";
import { cancelJobById, fetchJobsList, type JobSummary } from "../lib/api/jobs";
import {
  fetchPlaybookOutline,
  startPlaybookOperation,
  startPrepHosts,
  type PlaybookOutlineResponse,
} from "../lib/api/operations";
import { formatBytes, formatDurationMs, formatTimestampMs } from "../lib/format";
import { useJobEventsSse } from "../lib/sse";

export interface OperationsPageProps {
  selectedJobId: string | null;
  onSelectJob: (jobId: string | null) => void;
}

interface TaskMatrixCell {
  host: string;
  state: "running" | "ok" | "changed" | "failed" | "skipped";
  durationMs?: number;
  percent?: number;
}

interface TaskMatrixRow {
  playName: string;
  taskName: string;
  hosts: Map<string, TaskMatrixCell>;
}

const PLAYBOOK_OPTIONS: readonly SelectOption[] = [
  { value: "deploy", label: "集群部署 (deploy.yaml)" },
  { value: "download", label: "离线下载 (download.yaml)" },
  { value: "cleanup", label: "集群清理 (cleanup.yaml)" },
];

export function OperationsPage({ selectedJobId, onSelectJob }: OperationsPageProps) {
  const { pushToast } = useToast();
  const [jobs, setJobs] = useState<JobSummary[]>([]);
  const [loadingJobs, setLoadingJobs] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [playbook, setPlaybook] = useState<PlaybookName>("deploy");
  const [outline, setOutline] = useState<PlaybookOutlineResponse | null>(null);
  const [selectedTags, setSelectedTags] = useState<Set<string>>(new Set());
  const [forks, setForks] = useState(20);
  const [forceReinstall, setForceReinstall] = useState(false);
  const [bestEffortPrep, setBestEffortPrep] = useState(false);

  const [outlineDrawerOpen, setOutlineDrawerOpen] = useState(false);
  const [confirmAction, setConfirmAction] = useState<"cleanup" | "force_deploy" | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const [hostFilter, setHostFilter] = useState("__all__");
  const [pinnedToBottom, setPinnedToBottom] = useState(true);
  const logContainerRef = useRef<HTMLDivElement | null>(null);

  const stream = useJobEventsSse(selectedJobId);

  const loadJobs = useCallback(async () => {
    setLoadingJobs(true);
    setError(null);
    try {
      const res = await fetchJobsList({ limit: 30 });
      setJobs(res.items);
      if (!selectedJobId && res.items[0]) {
        onSelectJob(res.items[0].id);
      }
    } catch (err) {
      setError(err instanceof ApiClientError ? `${err.message} (${err.code})` : "任务列表加载失败");
    } finally {
      setLoadingJobs(false);
    }
  }, [selectedJobId, onSelectJob]);

  const loadOutline = useCallback(async (pb: PlaybookName) => {
    try {
      const res = await fetchPlaybookOutline(pb);
      setOutline(res);
      setSelectedTags(new Set());
    } catch {
      setOutline(null);
    }
  }, []);

  useEffect(() => {
    void loadJobs();
  }, [loadJobs]);

  useEffect(() => {
    void loadOutline(playbook);
  }, [playbook, loadOutline]);

  useEffect(() => {
    if (stream.jobStatus && isTerminalJobStatus(stream.jobStatus)) {
      void loadJobs();
    }
  }, [stream.jobStatus, loadJobs]);

  useEffect(() => {
    if (pinnedToBottom && logContainerRef.current) {
      const el = logContainerRef.current;
      el.scrollTop = el.scrollHeight;
    }
  }, [stream.frames.length, pinnedToBottom]);

  const handleLogScroll = () => {
    const el = logContainerRef.current;
    if (!el) {
      return;
    }
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setPinnedToBottom(distanceFromBottom < 24);
  };

  const scrollToBottom = () => {
    const el = logContainerRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
    setPinnedToBottom(true);
  };

  const { matrixRows, observedHosts, aggregateCounts } = useMemo(() => {
    const rows: TaskMatrixRow[] = [];
    const rowIndex = new Map<string, TaskMatrixRow>();
    const hostsSet = new Set<string>();
    let currentPlay = "默认阶段";
    let ok = 0;
    let changed = 0;
    let skipped = 0;
    let failed = 0;

    const ensureRow = (playName: string, taskName: string): TaskMatrixRow => {
      const key = `${playName}::${taskName}`;
      const existing = rowIndex.get(key);
      if (existing) {
        return existing;
      }
      const created: TaskMatrixRow = {
        playName,
        taskName,
        hosts: new Map(),
      };
      rowIndex.set(key, created);
      rows.push(created);
      return created;
    };

    for (const frame of stream.frames) {
      const ev = frame.event;
      if (ev.kind === "play_started") {
        currentPlay = ev.play_name;
        for (const h of ev.hosts) {
          hostsSet.add(h);
        }
      } else if (ev.kind === "task_started") {
        currentPlay = ev.play_name || currentPlay;
        hostsSet.add(ev.host);
        const r = ensureRow(currentPlay, ev.task_name);
        r.hosts.set(ev.host, { host: ev.host, state: "running" });
      } else if (ev.kind === "task_progress") {
        hostsSet.add(ev.host);
        const r = ensureRow(currentPlay, ev.task_name);
        r.hosts.set(ev.host, {
          host: ev.host,
          state: "running",
          percent: ev.percent,
        });
      } else if (ev.kind === "task_finished") {
        hostsSet.add(ev.host);
        const r = ensureRow(currentPlay, ev.task_name);
        if (ev.failed) {
          failed += 1;
          r.hosts.set(ev.host, { host: ev.host, state: "failed", durationMs: ev.duration_ms });
        } else if (ev.skipped) {
          skipped += 1;
          r.hosts.set(ev.host, { host: ev.host, state: "skipped", durationMs: ev.duration_ms });
        } else if (ev.changed) {
          changed += 1;
          r.hosts.set(ev.host, { host: ev.host, state: "changed", durationMs: ev.duration_ms });
        } else {
          ok += 1;
          r.hosts.set(ev.host, { host: ev.host, state: "ok", durationMs: ev.duration_ms });
        }
      }
    }

    return {
      matrixRows: rows,
      observedHosts: Array.from(hostsSet),
      aggregateCounts: { ok, changed, skipped, failed },
    };
  }, [stream.frames]);

  const hostFilterOptions = useMemo<SelectOption[]>(() => {
    const base: SelectOption[] = [{ value: "__all__", label: "全部节点" }];
    for (const h of observedHosts) {
      base.push({ value: h, label: h });
    }
    return base;
  }, [observedHosts]);

  const jobSelectOptions = useMemo<SelectOption[]>(
    () =>
      jobs.map((j) => ({
        value: j.id,
        label: `${j.title || j.kind} (${j.status}) · ${j.id.slice(0, 10)}`,
      })),
    [jobs],
  );

  const visibleFrames = useMemo(() => {
    if (hostFilter === "__all__") {
      return stream.frames;
    }
    return stream.frames.filter((f) => {
      const ev = f.event;
      if (ev.kind === "task_started" || ev.kind === "task_progress" || ev.kind === "task_finished") {
        return ev.host === hostFilter;
      }
      if (ev.kind === "play_started") {
        return ev.hosts.includes(hostFilter);
      }
      return true;
    });
  }, [stream.frames, hostFilter]);

  const triggerRun = async (params: {
    pb: PlaybookName;
    tags?: string[];
    extra?: Record<string, unknown>;
  }) => {
    setSubmitting(true);
    try {
      const res = await startPlaybookOperation({
        playbook: params.pb,
        tags: params.tags && params.tags.length > 0 ? params.tags : undefined,
        forks,
        extra_vars: params.extra,
      });
      onSelectJob(res.job_id);
      setPinnedToBottom(true);
      await loadJobs();
      pushToast("info", `任务已启动 (${res.job_id})`);
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "编排启动失败");
    } finally {
      setSubmitting(false);
    }
  };

  const handlePrepHosts = async () => {
    setSubmitting(true);
    try {
      const res = await startPrepHosts(bestEffortPrep);
      onSelectJob(res.job_id);
      setPinnedToBottom(true);
      await loadJobs();
      pushToast("info", "主机名同步已启动");
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "同步主机名失败");
    } finally {
      setSubmitting(false);
    }
  };

  const handleDeployClick = () => {
    if (forceReinstall) {
      setConfirmAction("force_deploy");
      return;
    }
    void triggerRun({
      pb: "deploy",
      tags: Array.from(selectedTags),
    });
  };

  const handleConfirmDanger = () => {
    const mode = confirmAction;
    setConfirmAction(null);
    if (mode === "cleanup") {
      void triggerRun({ pb: "cleanup" });
    } else if (mode === "force_deploy") {
      void triggerRun({
        pb: "deploy",
        tags: Array.from(selectedTags),
        extra: { force_reinstall: true },
      });
    }
  };

  const handleCancelCurrentJob = async () => {
    if (!selectedJobId) {
      return;
    }
    try {
      await cancelJobById(selectedJobId);
      pushToast("info", "已发送取消信号");
      await loadJobs();
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "取消任务失败");
    }
  };

  const activeJobSummary = jobs.find((j) => j.id === selectedJobId) ?? null;
  const effectiveStatus = stream.jobStatus ?? activeJobSummary?.status ?? null;
  const isJobRunning = effectiveStatus === "running" || effectiveStatus === "queued";

  return (
    <div className="space-y-4 max-w-6xl">
      <div className="park-card p-3.5 flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            disabled={submitting}
            onClick={() => void handlePrepHosts()}
          >
            同步主机名
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={submitting}
            onClick={() => void triggerRun({ pb: "deploy", tags: ["check"] })}
          >
            环境预检
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={submitting}
            onClick={() => void triggerRun({ pb: "deploy", tags: ["preflight"] })}
          >
            收敛基线
          </Button>
          <Button
            variant="primary"
            size="sm"
            icon={<Play size={13} />}
            disabled={submitting}
            onClick={handleDeployClick}
          >
            部署集群
          </Button>
          <Button
            variant="danger"
            size="sm"
            icon={<ShieldAlert size={13} />}
            disabled={submitting}
            onClick={() => setConfirmAction("cleanup")}
          >
            清理集群
          </Button>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            icon={<SlidersHorizontal size={13} />}
            onClick={() => setOutlineDrawerOpen(true)}
          >
            编排参数
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<RefreshCw size={13} />}
            onClick={() => void loadJobs()}
          >
            刷新
          </Button>
        </div>
      </div>

      <StateView loading={loadingJobs} error={error} onRetry={() => void loadJobs()}>
        <div className="space-y-4">
          <div className="park-card p-3.5 flex flex-wrap items-end justify-between gap-3">
            <div className="w-full sm:w-96">
              {jobSelectOptions.length > 0 ? (
                <Select
                  label="当前订阅任务"
                  value={selectedJobId ?? ""}
                  options={jobSelectOptions}
                  onChange={(val) => {
                    onSelectJob(val);
                    setPinnedToBottom(true);
                  }}
                />
              ) : (
                <div className="text-xs" style={{ color: "var(--text-muted)" }}>
                  尚无历史任务，可通过上方动作栏发起编排
                </div>
              )}
            </div>

            <div className="flex flex-wrap items-center gap-2 text-xs tabular-nums">
              {effectiveStatus ? (
                <span
                  className="park-badge font-mono-code"
                  data-testid="active-job-status"
                  style={{
                    borderColor:
                      effectiveStatus === "failed"
                        ? "var(--danger)"
                        : effectiveStatus === "succeeded"
                          ? "var(--success)"
                          : "var(--border)",
                  }}
                >
                  状态 · {effectiveStatus}
                </span>
              ) : null}
              <span className="park-badge">成功 {aggregateCounts.ok}</span>
              <span className="park-badge">变更 {aggregateCounts.changed}</span>
              <span className="park-badge">跳过 {aggregateCounts.skipped}</span>
              <span
                className="park-badge"
                style={{
                  borderColor: aggregateCounts.failed > 0 ? "var(--danger)" : "var(--border)",
                }}
              >
                失败 {aggregateCounts.failed}
              </span>
              {isJobRunning ? (
                <Button
                  variant="danger"
                  size="sm"
                  icon={<XCircle size={13} />}
                  onClick={() => void handleCancelCurrentJob()}
                >
                  取消任务
                </Button>
              ) : null}
            </div>
          </div>

          <div className="grid grid-cols-1 lg:grid-cols-12 gap-4">
            <div className="lg:col-span-5 park-panel flex flex-col max-h-[560px]">
              <div
                className="px-3.5 py-2.5 border-b flex items-center justify-between"
                style={{ backgroundColor: "var(--surface)", borderColor: "var(--border)" }}
              >
                <span className="text-xs font-semibold inline-flex items-center gap-1.5">
                  <ListChecks size={14} />
                  <span>Play × Task × Host 矩阵</span>
                </span>
                <span className="text-xs tabular-nums" style={{ color: "var(--text-muted)" }}>
                  {matrixRows.length} 项任务
                </span>
              </div>

              <div className="flex-1 overflow-y-auto p-3 space-y-2">
                {matrixRows.length === 0 ? (
                  <div className="py-12 text-center text-xs" style={{ color: "var(--text-muted)" }}>
                    暂无任务节点矩阵
                  </div>
                ) : (
                  matrixRows.map((row) => (
                    <div
                      key={`${row.playName}::${row.taskName}`}
                      className="p-2.5 border space-y-1.5"
                      style={{
                        backgroundColor: "var(--surface)",
                        borderColor: "var(--border)",
                        borderRadius: "var(--radius-md)",
                      }}
                    >
                      <div className="text-[11px] truncate" style={{ color: "var(--text-muted)" }}>
                        {row.playName}
                      </div>
                      <div className="text-xs font-medium break-words">{row.taskName}</div>
                      <div className="flex flex-wrap gap-1 pt-0.5">
                        {Array.from(row.hosts.values()).map((cell) => (
                          <button
                            key={cell.host}
                            type="button"
                            onClick={() =>
                              setHostFilter((prev) => (prev === cell.host ? "__all__" : cell.host))
                            }
                            className="px-1.5 py-0.5 text-[11px] font-mono-code tabular-nums border cursor-pointer"
                            style={{
                              borderRadius: "4px",
                              backgroundColor: "var(--bg)",
                              borderColor:
                                cell.state === "failed"
                                  ? "var(--danger)"
                                  : cell.state === "changed"
                                    ? "var(--warning)"
                                    : cell.state === "ok"
                                      ? "var(--success)"
                                      : "var(--border)",
                              color: "var(--text)",
                            }}
                          >
                            {cell.host} · {cell.state}
                            {cell.durationMs !== undefined
                              ? ` (${formatDurationMs(cell.durationMs)})`
                              : cell.percent !== undefined
                                ? ` (${cell.percent}%)`
                                : ""}
                          </button>
                        ))}
                      </div>
                    </div>
                  ))
                )}
              </div>
            </div>

            <div className="lg:col-span-7 park-panel flex flex-col max-h-[560px] relative">
              <div
                className="px-3.5 py-2 border-b flex flex-wrap items-center justify-between gap-2"
                style={{ backgroundColor: "var(--surface)", borderColor: "var(--border)" }}
              >
                <span className="text-xs font-semibold inline-flex items-center gap-1.5">
                  <Terminal size={14} />
                  <span>实时日志流</span>
                </span>
                <div className="w-44">
                  <Select
                    ariaLabel="节点过滤"
                    value={hostFilter}
                    options={hostFilterOptions}
                    onChange={setHostFilter}
                  />
                </div>
              </div>

              <div
                ref={logContainerRef}
                onScroll={handleLogScroll}
                className="flex-1 overflow-y-auto p-3.5 font-mono-code text-xs space-y-1.5 leading-relaxed"
                style={{ backgroundColor: "var(--bg)", color: "var(--text)" }}
              >
                {visibleFrames.length === 0 ? (
                  <div className="py-12 text-center" style={{ color: "var(--text-muted)" }}>
                    暂无运行日志
                  </div>
                ) : (
                  visibleFrames.map((frame) => {
                    const ev = frame.event;
                    return (
                      <div key={frame.seq} className="border-b pb-1.5 last:border-b-0" style={{ borderColor: "var(--surface-2)" }}>
                        <div className="flex flex-wrap items-center gap-2 tabular-nums">
                          <span style={{ color: "var(--text-muted)" }}>
                            [{formatTimestampMs(frame.ts_ms).slice(11)}]
                          </span>
                          {ev.kind === "play_started" ? (
                            <span className="font-semibold">
                              PLAY [{ev.play_name}] → {ev.hosts.join(", ")}
                            </span>
                          ) : ev.kind === "task_started" ? (
                            <span>
                              TASK [{ev.task_name}] @ {ev.host} 启动
                            </span>
                          ) : ev.kind === "task_progress" ? (
                            <span>
                              PROGRESS [{ev.task_name}] @ {ev.host} · {ev.percent}% {ev.detail}
                            </span>
                          ) : ev.kind === "task_finished" ? (
                            <span
                              style={{
                                color: ev.failed
                                  ? "var(--danger)"
                                  : ev.changed
                                    ? "var(--warning)"
                                    : ev.skipped
                                      ? "var(--text-muted)"
                                      : "var(--success)",
                              }}
                            >
                              {ev.failed ? "FAILED" : ev.skipped ? "SKIPPED" : ev.changed ? "CHANGED" : "OK"}{" "}
                              [{ev.task_name}] @ {ev.host} ({formatDurationMs(ev.duration_ms)})
                            </span>
                          ) : ev.kind === "artifact_progress" ? (
                            <span>
                              {ev.phase} · {ev.item} · {formatBytes(ev.bytes_done)}
                            </span>
                          ) : (
                            <span>{ev.message}</span>
                          )}
                        </div>
                        {ev.kind === "task_finished" && (ev.stdout || ev.stderr) ? (
                          <pre
                            className="mt-1 mb-0 p-2 overflow-x-auto text-[11px] whitespace-pre-wrap break-all"
                            style={{
                              backgroundColor: "var(--surface)",
                              borderRadius: "var(--radius-md)",
                              color: ev.stderr && ev.failed ? "var(--danger)" : "var(--text)",
                            }}
                          >
                            {[ev.stdout, ev.stderr].filter(Boolean).join("\n")}
                          </pre>
                        ) : null}
                      </div>
                    );
                  })
                )}
              </div>

              {!pinnedToBottom ? (
                <div className="absolute bottom-3 right-4">
                  <Button
                    variant="primary"
                    size="sm"
                    icon={<ArrowDown size={13} />}
                    onClick={scrollToBottom}
                  >
                    回到底部
                  </Button>
                </div>
              ) : null}
            </div>
          </div>
        </div>
      </StateView>

      <Drawer
        open={outlineDrawerOpen}
        onOpenChange={setOutlineDrawerOpen}
        title="编排参数与标签"
        side="right"
        widthClass="max-w-md"
        footer={
          <>
            <Button variant="secondary" size="sm" onClick={() => setOutlineDrawerOpen(false)}>
              关闭
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={submitting}
              onClick={() => {
                setOutlineDrawerOpen(false);
                if (playbook === "cleanup") {
                  setConfirmAction("cleanup");
                } else if (forceReinstall) {
                  setConfirmAction("force_deploy");
                } else {
                  void triggerRun({
                    pb: playbook,
                    tags: Array.from(selectedTags),
                  });
                }
              }}
            >
              执行编排
            </Button>
          </>
        }
      >
        <div className="space-y-4 text-xs">
          <Select
            label="目标剧本"
            value={playbook}
            options={PLAYBOOK_OPTIONS}
            onChange={(val) => setPlaybook(val as PlaybookName)}
          />

          <div>
            <label className="block mb-1 text-xs font-medium">并发批次 (forks)</label>
            <input
              type="number"
              min={1}
              max={64}
              value={forks}
              onChange={(e) => setForks(Math.max(1, Number.parseInt(e.target.value || "20", 10)))}
              className="park-input tabular-nums font-mono-code"
            />
          </div>

          <Switch
            checked={bestEffortPrep}
            onChange={setBestEffortPrep}
            label="主机名同步容忍离线节点"
            description="best_effort 模式跳过不可达节点"
          />

          <Switch
            checked={forceReinstall}
            onChange={setForceReinstall}
            label="强制重装集群组件"
            description="注入 extra_vars.force_reinstall=true，需二次确认"
          />

          {outline ? (
            <div className="space-y-3 pt-2 border-t" style={{ borderColor: "var(--border)" }}>
              <div className="font-semibold">可用 Tags 过滤（未勾选则全量）</div>
              {outline.all_tags.length === 0 ? (
                <div style={{ color: "var(--text-muted)" }}>当前剧本未声明标签</div>
              ) : (
                <div className="flex flex-wrap gap-2">
                  {outline.all_tags.map((tag) => (
                    <Checkbox
                      key={tag}
                      checked={selectedTags.has(tag)}
                      label={tag}
                      onChange={(checked) => {
                        setSelectedTags((prev) => {
                          const next = new Set(prev);
                          if (checked) {
                            next.add(tag);
                          } else {
                            next.delete(tag);
                          }
                          return next;
                        });
                      }}
                    />
                  ))}
                </div>
              )}

              <div className="space-y-2 pt-2">
                <div className="font-semibold">剧本阶段大纲</div>
                {outline.plays.map((play, idx) => (
                  <div key={idx} className="park-card p-2.5 space-y-1">
                    <div className="font-medium">
                      {play.name} <span className="font-mono-code opacity-75">({play.hosts})</span>
                    </div>
                    <div className="tabular-nums" style={{ color: "var(--text-muted)" }}>
                      共 {play.tasks.length} 项任务
                    </div>
                  </div>
                ))}
              </div>
            </div>
          ) : null}
        </div>
      </Drawer>

      <ConfirmModal
        open={confirmAction !== null}
        onOpenChange={(open) => {
          if (!open) {
            setConfirmAction(null);
          }
        }}
        title={confirmAction === "cleanup" ? "清理集群" : "强制重装集群"}
        consequence={
          <span>
            此操作将在受控集群节点执行 <code className="font-mono-code">kubeadm reset</code> 并清理{" "}
            <code className="font-mono-code">/var/lib/etcd</code> 持久化数据，不可逆转。
          </span>
        }
        requiredPhrase="CLEANUP"
        cancelLabel="取消"
        confirmLabel="确认清理"
        danger
        busy={submitting}
        onConfirm={handleConfirmDanger}
      />
    </div>
  );
}
