import {
  Copy,
  Download,
  FolderPlus,
  Layers,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Trash2,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "../components/primitives/Button";
import { Checkbox } from "../components/primitives/Checkbox";
import { DataTable, type DataTableColumn } from "../components/primitives/DataTable";
import { Dialog } from "../components/primitives/Dialog";
import { Drawer } from "../components/primitives/Drawer";
import { Select, type SelectOption } from "../components/primitives/Select";
import { StateView } from "../components/primitives/StateView";
import { Tabs } from "../components/primitives/Tabs";
import { useToast } from "../components/primitives/Toast";
import { ApiClientError } from "../lib/api/client";
import { isTerminalJobStatus } from "../lib/api/enums";
import {
  deleteHarborArtifact,
  fetchHarborArtifacts,
  fetchHarborProjects,
  fetchHarborRepositories,
  planHarborMirror,
  startHarborMirrorImport,
  syncHarborProjects,
  type HarborArtifactInfo,
  type HarborMirrorPlanResponse,
  type HarborProjectInfo,
  type HarborProjectsResponse,
  type HarborRepositoryInfo,
  type PublicRegistryMirrorItem,
  type TransparentMirrorTarget,
} from "../lib/api/harbor";
import { cancelJobById } from "../lib/api/jobs";
import { copyTextToClipboard } from "../lib/clipboard";
import { formatBytes } from "../lib/format";
import { useJobEventsSse } from "../lib/sse";

export interface HarborPageProps {
  onNavigate?: (path: string) => void;
}

const PLATFORM_OPTIONS: readonly SelectOption[] = [
  { value: "linux/amd64", label: "linux/amd64" },
  { value: "linux/arm64", label: "linux/arm64" },
];

function formatShortDigest(digest: string): string {
  const clean = digest.startsWith("sha256:") ? digest.slice(7) : digest;
  return clean.length > 12 ? `sha256:${clean.slice(0, 12)}` : digest || "—";
}

function formatIsoShort(iso: string): string {
  if (!iso) {
    return "—";
  }
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) {
    return iso;
  }
  return d.toLocaleString("zh-CN", { hour12: false });
}

export function HarborPage({ onNavigate }: HarborPageProps) {
  const { pushToast } = useToast();
  const [activeTab, setActiveTab] = useState("import");

  const [projectsData, setProjectsData] = useState<HarborProjectsResponse | null>(null);
  const [loadingProjects, setLoadingProjects] = useState(true);
  const [projectsError, setProjectsError] = useState<string | null>(null);

  // Mirror pre-pull & import state
  const [rawImagesInput, setRawImagesInput] = useState("");
  const [platform, setPlatform] = useState("linux/amd64");
  const [forceImport, setForceImport] = useState(false);
  const [saveTar, setSaveTar] = useState(false);
  const [insecurePull, setInsecurePull] = useState(false);
  const [appendExtraImages, setAppendExtraImages] = useState(false);
  const [planBusy, setPlanBusy] = useState(false);
  const [importBusy, setImportBusy] = useState(false);
  const [planResult, setPlanResult] = useState<HarborMirrorPlanResponse | null>(null);

  // SSE drawer state
  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const stream = useJobEventsSse(activeJobId);

  // Project / Repository / Artifact browser state
  const [selectedProject, setSelectedProject] = useState<string>("");
  const [repoQuery, setRepoQuery] = useState("");
  const [repos, setRepos] = useState<HarborRepositoryInfo[]>([]);
  const [loadingRepos, setLoadingRepos] = useState(false);
  const [selectedRepo, setSelectedRepo] = useState<string>("");
  const [artifacts, setArtifacts] = useState<HarborArtifactInfo[]>([]);
  const [loadingArtifacts, setLoadingArtifacts] = useState(false);

  // Create project modal & sync state
  const [syncBusy, setSyncBusy] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [newProjectName, setNewProjectName] = useState("");
  const [creatingProject, setCreatingProject] = useState(false);

  // Delete confirmation modal
  const [deleteTarget, setDeleteTarget] = useState<{
    project: string;
    repository: string;
    reference: string;
    label: string;
  } | null>(null);
  const [deleting, setDeleting] = useState(false);

  const loadProjects = useCallback(async () => {
    setLoadingProjects(true);
    setProjectsError(null);
    try {
      const res = await fetchHarborProjects();
      setProjectsData(res);
      setSelectedProject((prev) => {
        if (prev && res.projects.some((p) => p.name === prev && p.exists)) {
          return prev;
        }
        const firstExisting = res.projects.find((p) => p.exists);
        return firstExisting ? firstExisting.name : "";
      });
    } catch (err) {
      setProjectsError(
        err instanceof ApiClientError ? `${err.message} (${err.code})` : "私库项目加载失败",
      );
    } finally {
      setLoadingProjects(false);
    }
  }, []);

  const loadRepos = useCallback(
    async (project: string, q: string) => {
      if (!project) {
        setRepos([]);
        setSelectedRepo("");
        setArtifacts([]);
        return;
      }
      setLoadingRepos(true);
      try {
        const res = await fetchHarborRepositories(project, q);
        setRepos(res.repositories);
        setSelectedRepo((prev) => {
          if (prev && res.repositories.some((r) => r.short_name === prev)) {
            return prev;
          }
          return res.repositories[0]?.short_name ?? "";
        });
      } catch (err) {
        pushToast("error", err instanceof ApiClientError ? err.message : "仓库列表读取失败");
        setRepos([]);
      } finally {
        setLoadingRepos(false);
      }
    },
    [pushToast],
  );

  const loadArtifacts = useCallback(
    async (project: string, repoShort: string) => {
      if (!project || !repoShort) {
        setArtifacts([]);
        return;
      }
      setLoadingArtifacts(true);
      try {
        const res = await fetchHarborArtifacts(project, repoShort);
        setArtifacts(res.artifacts);
      } catch (err) {
        pushToast("error", err instanceof ApiClientError ? err.message : "镜像标签读取失败");
        setArtifacts([]);
      } finally {
        setLoadingArtifacts(false);
      }
    },
    [pushToast],
  );

  useEffect(() => {
    void loadProjects();
  }, [loadProjects]);

  useEffect(() => {
    if (selectedProject) {
      void loadRepos(selectedProject, repoQuery);
    } else {
      setRepos([]);
      setSelectedRepo("");
      setArtifacts([]);
    }
  }, [selectedProject, repoQuery, loadRepos]);

  useEffect(() => {
    if (selectedProject && selectedRepo) {
      void loadArtifacts(selectedProject, selectedRepo);
    } else {
      setArtifacts([]);
    }
  }, [selectedProject, selectedRepo, loadArtifacts]);

  useEffect(() => {
    if (stream.jobStatus && isTerminalJobStatus(stream.jobStatus)) {
      void loadProjects();
      if (rawImagesInput.trim()) {
        void planHarborMirror([rawImagesInput], true)
          .then(setPlanResult)
          .catch(() => undefined);
      }
    }
  }, [stream.jobStatus, loadProjects, rawImagesInput]);

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

  const handlePlan = async () => {
    if (!rawImagesInput.trim()) {
      pushToast("error", "请输入镜像地址");
      return;
    }
    setPlanBusy(true);
    try {
      const res = await planHarborMirror([rawImagesInput], true);
      setPlanResult(res);
      const errCount = res.plans.filter((p) => p.error).length;
      if (errCount > 0) {
        pushToast("error", `${errCount} 条镜像解析异常`);
      } else {
        pushToast("success", `已解析 ${res.plans.length} 条映射`);
      }
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "映射解析失败");
    } finally {
      setPlanBusy(false);
    }
  };

  const handleImport = async () => {
    if (!rawImagesInput.trim()) {
      pushToast("error", "请输入镜像地址");
      return;
    }
    setImportBusy(true);
    try {
      const res = await startHarborMirrorImport({
        images: [rawImagesInput],
        platform,
        save_tar: saveTar,
        insecure: insecurePull,
        force: forceImport,
        append_extra_images: appendExtraImages,
      });
      setActiveJobId(res.job_id);
      setDrawerOpen(true);
      pushToast("info", "预拉取导入已启动");
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "预拉取导入启动失败");
    } finally {
      setImportBusy(false);
    }
  };

  const handleSyncMissingProjects = async () => {
    setSyncBusy(true);
    try {
      const res = await syncHarborProjects([], false);
      pushToast(
        "success",
        `已同步项目（新建 ${res.created.length}，已有 ${res.existing.length}）`,
      );
      await loadProjects();
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "项目同步失败");
    } finally {
      setSyncBusy(false);
    }
  };

  const handleCreateProject = async () => {
    const name = newProjectName.trim();
    if (!name) {
      pushToast("error", "请输入项目名");
      return;
    }
    setCreatingProject(true);
    try {
      const res = await syncHarborProjects([name], true);
      if (res.created.length > 0) {
        pushToast("success", `项目 ${name} 已创建`);
      } else {
        pushToast("info", `项目 ${name} 已存在`);
      }
      setNewProjectName("");
      setCreateOpen(false);
      await loadProjects();
      setSelectedProject(name);
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "项目创建失败");
    } finally {
      setCreatingProject(false);
    }
  };

  const handleConfirmDelete = async () => {
    if (!deleteTarget) {
      return;
    }
    setDeleting(true);
    try {
      await deleteHarborArtifact(
        deleteTarget.project,
        deleteTarget.repository,
        deleteTarget.reference,
      );
      pushToast("success", `已删除 ${deleteTarget.label}`);
      setDeleteTarget(null);
      await loadArtifacts(deleteTarget.project, deleteTarget.repository);
      await loadProjects();
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "删除失败");
    } finally {
      setDeleting(false);
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

  const handleCopy = async (text: string) => {
    const ok = await copyTextToClipboard(text);
    pushToast(ok ? "success" : "error", ok ? "已复制" : "复制失败");
  };

  const planColumns: DataTableColumn<TransparentMirrorTarget>[] = [
    {
      key: "source",
      header: "规范化源镜像",
      render: (r) => (
        <div className="space-y-0.5">
          <div className="font-mono-code text-xs break-all">
            {r.normalized_source || r.raw_input}
          </div>
          {r.raw_input !== r.normalized_source && r.normalized_source ? (
            <div className="text-[11px] font-mono-code" style={{ color: "var(--text-muted)" }}>
              原始: {r.raw_input}
            </div>
          ) : null}
        </div>
      ),
    },
    {
      key: "target",
      header: "Harbor 透明目标",
      render: (r) =>
        r.error ? (
          <span className="text-xs" style={{ color: "var(--danger)" }}>
            {r.error}
          </span>
        ) : (
          <span className="font-mono-code text-xs break-all font-medium">{r.target_ref}</span>
        ),
    },
    {
      key: "rule",
      header: "命中规则",
      render: (r) =>
        r.matched_rule ? (
          <span
            className="park-badge font-mono-code"
            style={{
              borderColor: r.used_default_fallback ? "var(--warning)" : "var(--border)",
            }}
          >
            {r.matched_rule}
          </span>
        ) : (
          <span>—</span>
        ),
    },
    {
      key: "status",
      header: "私库状态",
      render: (r) => {
        if (r.error) {
          return (
            <span className="park-badge" style={{ borderColor: "var(--danger)" }}>
              解析失败
            </span>
          );
        }
        if (r.already_exists === true) {
          return (
            <span className="park-badge" style={{ borderColor: "var(--success)" }}>
              已存在
            </span>
          );
        }
        if (r.project_exists === false) {
          return (
            <span className="park-badge" style={{ borderColor: "var(--warning)" }}>
              待建项导入
            </span>
          );
        }
        return (
          <span className="park-badge" style={{ borderColor: "var(--accent)" }}>
            待导入
          </span>
        );
      },
    },
  ];

  const projectColumns: DataTableColumn<HarborProjectInfo>[] = [
    {
      key: "name",
      header: "项目",
      render: (r) => (
        <div className="flex items-center gap-1.5 flex-wrap">
          <button
            type="button"
            disabled={!r.exists}
            onClick={() => setSelectedProject(r.name)}
            className="font-mono-code text-xs font-medium text-left cursor-pointer disabled:cursor-not-allowed disabled:opacity-60 underline-offset-2 hover:underline"
            style={{
              color: selectedProject === r.name ? "var(--accent)" : "var(--text)",
            }}
          >
            {r.name}
          </button>
          {r.is_default_fallback ? (
            <span className="park-badge text-[10px]">_default</span>
          ) : null}
        </div>
      ),
    },
    {
      key: "exists",
      header: "状态",
      render: (r) => (
        <span
          className="park-badge"
          style={{ borderColor: r.exists ? "var(--success)" : "var(--warning)" }}
        >
          {r.exists ? "已就绪" : "缺失"}
        </span>
      ),
    },
    {
      key: "visibility",
      header: "可见性",
      render: (r) => (
        <span className="text-xs" style={{ color: "var(--text-muted)" }}>
          {r.public ? "公开" : "私有"}
        </span>
      ),
    },
    {
      key: "repos",
      header: "仓库数",
      align: "right",
      tabularNums: true,
      render: (r) => <span className="font-mono-code text-xs">{r.repo_count}</span>,
    },
    {
      key: "hosts",
      header: "映射源站",
      render: (r) => (
        <span className="font-mono-code text-[11px] break-all" style={{ color: "var(--text-muted)" }}>
          {r.mapped_hosts.length > 0 ? r.mapped_hosts.join(", ") : r.is_default_fallback ? "其余外部源" : "—"}
        </span>
      ),
    },
  ];

  const repoColumns: DataTableColumn<HarborRepositoryInfo>[] = [
    {
      key: "short_name",
      header: "仓库路径",
      render: (r) => (
        <button
          type="button"
          onClick={() => setSelectedRepo(r.short_name)}
          className="font-mono-code text-xs font-medium text-left cursor-pointer underline-offset-2 hover:underline break-all"
          style={{
            color: selectedRepo === r.short_name ? "var(--accent)" : "var(--text)",
          }}
        >
          {r.short_name}
        </button>
      ),
    },
    {
      key: "artifacts",
      header: "制品",
      align: "right",
      tabularNums: true,
      render: (r) => <span className="font-mono-code text-xs">{r.artifact_count}</span>,
    },
    {
      key: "pulls",
      header: "拉取",
      align: "right",
      tabularNums: true,
      render: (r) => <span className="font-mono-code text-xs">{r.pull_count}</span>,
    },
  ];

  const artifactColumns: DataTableColumn<HarborArtifactInfo>[] = [
    {
      key: "tags",
      header: "标签 / 摘要",
      render: (r) => (
        <div className="space-y-1">
          <div className="flex flex-wrap gap-1">
            {r.tags.length > 0 ? (
              r.tags.map((t) => (
                <span key={t.name} className="park-badge font-mono-code">
                  {t.name}
                </span>
              ))
            ) : (
              <span className="park-badge font-mono-code">untagged</span>
            )}
          </div>
          <div className="font-mono-code text-[11px]" style={{ color: "var(--text-muted)" }}>
            {formatShortDigest(r.digest)}
          </div>
        </div>
      ),
    },
    {
      key: "arch",
      header: "平台",
      render: (r) => (
        <span className="font-mono-code text-xs">
          {r.os && r.architecture ? `${r.os}/${r.architecture}` : "—"}
        </span>
      ),
    },
    {
      key: "size",
      header: "体积",
      align: "right",
      tabularNums: true,
      render: (r) => <span className="font-mono-code text-xs">{formatBytes(r.size)}</span>,
    },
    {
      key: "push_time",
      header: "推送时间",
      tabularNums: true,
      render: (r) => (
        <span className="font-mono-code text-xs" style={{ color: "var(--text-muted)" }}>
          {formatIsoShort(r.push_time)}
        </span>
      ),
    },
    {
      key: "actions",
      header: "操作",
      align: "right",
      render: (r) => {
        const regHost = projectsData?.registry_host ?? "";
        const tag = r.tags[0]?.name;
        const fullRef = tag
          ? `${regHost}/${selectedProject}/${selectedRepo}:${tag}`
          : `${regHost}/${selectedProject}/${selectedRepo}@${r.digest}`;
        const deleteRef = tag ?? r.digest;
        return (
          <div className="inline-flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              icon={<Copy size={12} />}
              onClick={() => void handleCopy(fullRef)}
            >
              复制
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={<Trash2 size={12} />}
              onClick={() =>
                setDeleteTarget({
                  project: selectedProject,
                  repository: selectedRepo,
                  reference: deleteRef,
                  label: `${selectedProject}/${selectedRepo}:${deleteRef}`,
                })
              }
            >
              删除
            </Button>
          </div>
        );
      },
    },
  ];

  const mirrorRuleColumns: DataTableColumn<PublicRegistryMirrorItem>[] = [
    {
      key: "host",
      header: "外部源域名",
      render: (r) => <span className="font-mono-code text-xs font-medium">{r.host}</span>,
    },
    {
      key: "project",
      header: "Harbor 项目",
      render: (r) => <span className="park-badge font-mono-code">{r.project}</span>,
    },
    {
      key: "endpoint",
      header: "透明映射前缀",
      render: (r) => (
        <span className="font-mono-code text-xs" style={{ color: "var(--text-muted)" }}>
          {projectsData?.registry_host || "<registry>"}/v2/{r.project}
        </span>
      ),
    },
  ];

  const existingProjectCount = useMemo(
    () => (projectsData?.projects ?? []).filter((p) => p.exists).length,
    [projectsData],
  );

  const missingCount = projectsData?.missing_mirror_projects.length ?? 0;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            icon={<RefreshCw size={13} />}
            disabled={loadingProjects}
            onClick={() => void loadProjects()}
          >
            刷新
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<Layers size={13} />}
            disabled={syncBusy}
            onClick={() => void handleSyncMissingProjects()}
          >
            补建项目
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<FolderPlus size={13} />}
            onClick={() => setCreateOpen(true)}
          >
            新建项目
          </Button>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {onNavigate ? (
            <Button
              variant="ghost"
              size="sm"
              icon={<Settings size={13} />}
              onClick={() => onNavigate("/inventory")}
            >
              映射配置
            </Button>
          ) : null}
          {activeJobId ? (
            <Button variant="secondary" size="sm" onClick={() => setDrawerOpen(true)}>
              导入进度
            </Button>
          ) : null}
        </div>
      </div>

      {projectsData ? (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
          <div className="park-card p-4">
            <div className="text-xs" style={{ color: "var(--text-muted)" }}>
              私库地址
            </div>
            <div className="mt-1 text-base font-semibold font-mono-code truncate">
              {projectsData.registry_host || "—"}
            </div>
          </div>
          <div className="park-card p-4">
            <div className="text-xs" style={{ color: "var(--text-muted)" }}>
              Harbor 项目
            </div>
            <div className="mt-1 text-base font-semibold tabular-nums font-mono-code">
              {existingProjectCount} 就绪 · {missingCount} 缺失
            </div>
          </div>
          <div className="park-card p-4">
            <div className="text-xs" style={{ color: "var(--text-muted)" }}>
              全局兜底 (_default)
            </div>
            <div className="mt-1 text-base font-semibold font-mono-code">
              {projectsData.default_fallback_project || "未启用"}
            </div>
          </div>
          <div className="park-card p-4">
            <div className="text-xs" style={{ color: "var(--text-muted)" }}>
              外网回退策略
            </div>
            <div className="mt-1 text-base font-semibold">
              {projectsData.allow_fallback ? "允许回退" : "禁止回退"}
            </div>
          </div>
        </div>
      ) : null}

      <Tabs
        value={activeTab}
        onChange={setActiveTab}
        items={[
          {
            value: "import",
            label: "预拉取导入",
            content: (
              <div className="space-y-4">
                <div className="park-card p-5 space-y-4">
                  <div>
                    <label
                      htmlFor="harbor-mirror-images-input"
                      className="block mb-1.5 text-xs font-medium"
                    >
                      上游镜像列表（每行一项，支持短名与 YAML image 行）
                    </label>
                    <textarea
                      id="harbor-mirror-images-input"
                      rows={5}
                      value={rawImagesInput}
                      onChange={(e) => setRawImagesInput(e.target.value)}
                      placeholder={"nginx:1.27\nbitnami/redis:7.4\nregistry.k8s.io/ingress-nginx/controller:v1.12.1"}
                      className="park-textarea font-mono-code"
                    />
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3 items-end">
                    <div>
                      <label className="block mb-1 text-xs font-medium">目标平台</label>
                      <Select
                        ariaLabel="目标平台"
                        value={platform}
                        onChange={setPlatform}
                        options={PLATFORM_OPTIONS}
                      />
                    </div>
                    <Checkbox
                      checked={forceImport}
                      onChange={setForceImport}
                      label="覆盖已有标签"
                    />
                    <Checkbox
                      checked={saveTar}
                      onChange={setSaveTar}
                      label="保留离线归档"
                    />
                    <Checkbox
                      checked={appendExtraImages}
                      onChange={setAppendExtraImages}
                      label="写入 extra_images"
                    />
                  </div>

                  <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
                    <Checkbox
                      checked={insecurePull}
                      onChange={setInsecurePull}
                      label="跳过上游 TLS 校验"
                    />
                    <div className="flex flex-wrap items-center gap-2">
                      <Button
                        variant="secondary"
                        size="sm"
                        icon={<Search size={13} />}
                        disabled={planBusy}
                        onClick={() => void handlePlan()}
                      >
                        解析预览
                      </Button>
                      <Button
                        variant="primary"
                        size="sm"
                        icon={<Download size={13} />}
                        disabled={importBusy}
                        onClick={() => void handleImport()}
                      >
                        预拉取导入
                      </Button>
                    </div>
                  </div>
                </div>

                {planResult ? (
                  <DataTable
                    rows={planResult.plans}
                    getRowKey={(r) => `${r.raw_input}:${r.target_ref}`}
                    columns={planColumns}
                    emptyText="暂无映射结果"
                  />
                ) : null}
              </div>
            ),
          },
          {
            value: "projects",
            label: "项目与镜像",
            badge: projectsData?.projects.length,
            content:
              activeTab === "projects" ? (
                <StateView
                  loading={loadingProjects}
                  error={projectsError}
                  onRetry={() => void loadProjects()}
                >
                  {projectsData && !projectsData.is_harbor ? (
                    <div className="park-card p-6 text-sm" style={{ color: "var(--text-muted)" }}>
                      当前目标私库（{projectsData.registry_host}）未响应 Harbor v2 项目接口，仅支持标准 OCI 推送。
                    </div>
                  ) : projectsData ? (
                    <div className="space-y-4">
                      <DataTable
                        rows={projectsData.projects}
                        getRowKey={(r) => r.name}
                        columns={projectColumns}
                        emptyText="暂无 Harbor 项目"
                      />

                      {selectedProject ? (
                        <div className="grid grid-cols-1 lg:grid-cols-12 gap-4">
                          <div className="lg:col-span-5 space-y-2.5">
                            <div className="flex items-center justify-between gap-2">
                              <span className="text-xs font-semibold font-mono-code">
                                仓库 · {selectedProject}
                              </span>
                              <input
                                type="search"
                                value={repoQuery}
                                onChange={(e) => setRepoQuery(e.target.value)}
                                placeholder="过滤仓库名"
                                aria-label="过滤仓库名"
                                className="park-input w-40"
                              />
                            </div>
                            <StateView loading={loadingRepos} error={null}>
                              <DataTable
                                rows={repos}
                                getRowKey={(r) => r.name}
                                columns={repoColumns}
                                emptyText="该项目下暂无镜像仓库"
                              />
                            </StateView>
                          </div>

                          <div className="lg:col-span-7 space-y-2.5">
                            <div className="flex items-center justify-between gap-2">
                              <span className="text-xs font-semibold font-mono-code truncate">
                                制品与标签 · {selectedRepo ? `${selectedProject}/${selectedRepo}` : "—"}
                              </span>
                              {selectedRepo ? (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  icon={<RefreshCw size={12} />}
                                  onClick={() => void loadArtifacts(selectedProject, selectedRepo)}
                                >
                                  刷新
                                </Button>
                              ) : null}
                            </div>
                            <StateView loading={loadingArtifacts} error={null}>
                              <DataTable
                                rows={artifacts}
                                getRowKey={(r) => r.digest}
                                columns={artifactColumns}
                                emptyText={selectedRepo ? "该仓库暂无标签" : "请选择左侧仓库"}
                              />
                            </StateView>
                          </div>
                        </div>
                      ) : null}
                    </div>
                  ) : null}
                </StateView>
              ) : null,
          },
          {
            value: "rules",
            label: "映射规则",
            badge: projectsData?.mirrors.length,
            content: (
              <div className="space-y-3">
                <DataTable
                  rows={projectsData?.mirrors ?? []}
                  getRowKey={(r) => r.host}
                  columns={mirrorRuleColumns}
                  emptyText="暂无映射规则"
                />
              </div>
            ),
          },
        ]}
      />

      <Dialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        title="新建项目"
        maxWidthClass="max-w-md"
        footer={
          <div className="flex justify-end gap-2">
            <Button variant="secondary" size="sm" onClick={() => setCreateOpen(false)}>
              取消
            </Button>
            <Button
              variant="primary"
              size="sm"
              icon={<Plus size={13} />}
              disabled={creatingProject}
              onClick={() => void handleCreateProject()}
            >
              创建
            </Button>
          </div>
        }
      >
        <div className="space-y-3">
          <div>
            <label htmlFor="new-harbor-project-name" className="block mb-1 text-xs font-medium">
              项目名称
            </label>
            <input
              id="new-harbor-project-name"
              type="text"
              value={newProjectName}
              onChange={(e) => setNewProjectName(e.target.value)}
              placeholder="external"
              className="park-input font-mono-code"
            />
          </div>
        </div>
      </Dialog>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteTarget(null);
          }
        }}
        title="删除制品"
        maxWidthClass="max-w-md"
        footer={
          <div className="flex justify-end gap-2">
            <Button variant="secondary" size="sm" onClick={() => setDeleteTarget(null)}>
              取消
            </Button>
            <Button
              variant="danger"
              size="sm"
              disabled={deleting}
              onClick={() => void handleConfirmDelete()}
            >
              删除
            </Button>
          </div>
        }
      >
        <p className="text-xs break-all" style={{ color: "var(--text-muted)" }}>
          将从 Harbor 移除制品 <span className="font-mono-code">{deleteTarget?.label}</span>，此操作不可恢复。
        </p>
      </Dialog>

      <Drawer
        open={drawerOpen}
        onOpenChange={setDrawerOpen}
        title="镜像导入任务"
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
            <span className="park-badge">
              {stream.jobStatus ?? (stream.connected ? "运行中" : "等待中")}
            </span>
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
                    : frame.event.kind === "artifact_progress"
                      ? `${frame.event.phase} · ${frame.event.item} (${formatBytes(frame.event.bytes_done)})`
                      : frame.event.kind}
                </div>
              ))
            )}
          </div>
        </div>
      </Drawer>
    </div>
  );
}
