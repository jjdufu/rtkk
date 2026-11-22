import {
  Activity,
  CheckCircle2,
  Copy,
  Plus,
  RefreshCw,
  Save,
  ShieldAlert,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "../components/primitives/Button";
import { Checkbox } from "../components/primitives/Checkbox";
import { DataTable, type DataTableColumn } from "../components/primitives/DataTable";
import { Dialog } from "../components/primitives/Dialog";
import { Select, type SelectOption } from "../components/primitives/Select";
import { StateView } from "../components/primitives/StateView";
import { Switch } from "../components/primitives/Switch";
import { Tabs } from "../components/primitives/Tabs";
import { useToast } from "../components/primitives/Toast";
import { ApiClientError } from "../lib/api/client";
import type { BecomeMethod, CniPlugin, RegistryScheme } from "../lib/api/enums";
import {
  fetchInventory,
  probeInventoryHosts,
  saveInventory,
  validateInventory,
  type DerivedClusterVars,
  type HostProbeResult,
  type InventoryWritePayload,
} from "../lib/api/inventory";
import { copyTextToClipboard } from "../lib/clipboard";
import { expandIpv4Range } from "../lib/format";

interface HostRow {
  hostname: string;
  ip: string;
  isControlPlane: boolean;
  isWorker: boolean;
  isEtcd: boolean;
}

type BatchRolePreset = "kube_node" | "kube_control_plane" | "control_and_etcd";

const BATCH_ROLE_OPTIONS: readonly SelectOption[] = [
  { value: "kube_node", label: "工作节点 (kube_node)" },
  { value: "control_and_etcd", label: "控制面 + etcd" },
  { value: "kube_control_plane", label: "仅控制面" },
];

const BECOME_METHOD_OPTIONS: readonly SelectOption[] = [
  { value: "sudo", label: "sudo" },
  { value: "su", label: "su (PTY)" },
];

const REGISTRY_SCHEME_OPTIONS: readonly SelectOption[] = [
  { value: "auto", label: "自动协商 (auto)" },
  { value: "https", label: "加密传输 (https)" },
  { value: "http", label: "明文传输 (http)" },
];

const CNI_OPTIONS: readonly SelectOption[] = [
  { value: "cilium", label: "Cilium" },
  { value: "custom", label: "自定义清单 (custom)" },
  { value: "none", label: "不安装 (none)" },
];

const VALIDITY_OPTIONS: readonly SelectOption[] = [
  { value: "867240h", label: "99 年 (867240h)" },
  { value: "876000h", label: "100 年 (876000h)" },
  { value: "87600h", label: "10 年 (87600h)" },
  { value: "26280h", label: "3 年 (26280h)" },
  { value: "8760h", label: "1 年 (8760h)" },
];

const DEFAULT_PUBLIC_REGISTRY_MIRRORS: ReadonlyArray<{ readonly host: string; readonly project: string }> = [
  { host: "docker.io", project: "dockerhub" },
  { host: "registry-1.docker.io", project: "dockerhub" },
  { host: "index.docker.io", project: "dockerhub" },
  { host: "registry.k8s.io", project: "k8s" },
  { host: "quay.io", project: "quay" },
  { host: "gcr.io", project: "gcr" },
  { host: "k8s.gcr.io", project: "gcr" },
  { host: "ghcr.io", project: "ghcr" },
  { host: "mcr.microsoft.com", project: "mcr" },
  { host: "public.ecr.aws", project: "ecr-public" },
  { host: "nvcr.io", project: "nvcr" },
  { host: "registry.gitlab.com", project: "gitlab" },
];

function asRecord(val: unknown): Record<string, unknown> {
  if (typeof val === "object" && val !== null && !Array.isArray(val)) {
    return val as Record<string, unknown>;
  }
  return {};
}

function asString(val: unknown, fallback = ""): string {
  if (typeof val === "string") {
    return val;
  }
  if (typeof val === "number" || typeof val === "boolean") {
    return String(val);
  }
  return fallback;
}

function asNumber(val: unknown, fallback: number): number {
  if (typeof val === "number" && Number.isFinite(val)) {
    return val;
  }
  if (typeof val === "string") {
    const parsed = Number.parseInt(val, 10);
    if (Number.isFinite(parsed)) {
      return parsed;
    }
  }
  return fallback;
}

function asBool(val: unknown, fallback = false): boolean {
  if (typeof val === "boolean") {
    return val;
  }
  if (val === "true") {
    return true;
  }
  if (val === "false") {
    return false;
  }
  return fallback;
}

function asStringArray(val: unknown): string[] {
  if (!Array.isArray(val)) {
    return [];
  }
  return val.filter((item): item is string => typeof item === "string");
}

function extractHostIp(raw: unknown): string {
  if (typeof raw === "string") {
    return raw;
  }
  const rec = asRecord(raw);
  return asString(rec["ip"] ?? rec["ansible_host"], "");
}

function cloneDeepRecord(obj: Record<string, unknown>): Record<string, unknown> {
  return JSON.parse(JSON.stringify(obj)) as Record<string, unknown>;
}

export function InventoryPage() {
  const { pushToast } = useToast();
  const [activeTab, setActiveTab] = useState("hosts");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [inventoryDoc, setInventoryDoc] = useState<Record<string, unknown>>({});
  const [rawYaml, setRawYaml] = useState("");
  const [editMode, setEditMode] = useState<"structured" | "yaml">("structured");
  const [derived, setDerived] = useState<DerivedClusterVars | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);

  const [hostSearch, setHostSearch] = useState("");
  const [selectedHosts, setSelectedHosts] = useState<Set<string>>(new Set());

  const [singleHostName, setSingleHostName] = useState("");
  const [singleHostIp, setSingleHostIp] = useState("");
  const [batchPrefix, setBatchPrefix] = useState("c23-p");
  const [batchRange, setBatchRange] = useState("");
  const [batchRole, setBatchRole] = useState<BatchRolePreset>("kube_node");

  const [saving, setSaving] = useState(false);
  const [validating, setValidating] = useState(false);
  const [probing, setProbing] = useState(false);
  const [probeOpen, setProbeOpen] = useState(false);
  const [probeResults, setProbeResults] = useState<HostProbeResult[]>([]);

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetchInventory();
      setInventoryDoc(res.inventory);
      setRawYaml(res.masked_yaml);
      setDerived(res.derived);
      setEditMode("structured");
      setWarnings([]);
    } catch (err) {
      setError(err instanceof ApiClientError ? `${err.message} (${err.code})` : "集群配置读取失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadData();
  }, [loadData]);

  const varsRec = useMemo(() => asRecord(inventoryDoc["vars"]), [inventoryDoc]);
  const registryRec = useMemo(() => asRecord(varsRec["registry"]), [varsRec]);
  const imageSyncRec = useMemo(() => asRecord(varsRec["image_sync"]), [varsRec]);
  const addonsRec = useMemo(() => asRecord(varsRec["addons"]), [varsRec]);
  const hostsMap = useMemo(() => asRecord(inventoryDoc["hosts"]), [inventoryDoc]);
  const rolesMap = useMemo(() => asRecord(inventoryDoc["roles"]), [inventoryDoc]);

  const updateVarField = useCallback((key: string, value: unknown) => {
    setInventoryDoc((prev) => {
      const next = cloneDeepRecord(prev);
      const nextVars = asRecord(next["vars"]);
      if (value === undefined) {
        delete nextVars[key];
      } else {
        nextVars[key] = value;
      }
      next["vars"] = nextVars;
      return next;
    });
    setEditMode("structured");
  }, []);

  const updateNestedVarField = useCallback((section: string, key: string, value: unknown) => {
    setInventoryDoc((prev) => {
      const next = cloneDeepRecord(prev);
      const nextVars = asRecord(next["vars"]);
      const sub = asRecord(nextVars[section]);
      sub[key] = value;
      nextVars[section] = sub;
      next["vars"] = nextVars;
      return next;
    });
    setEditMode("structured");
  }, []);

  const hostRows = useMemo<HostRow[]>(() => {
    const cpSet = new Set(asStringArray(rolesMap["kube_control_plane"]));
    const workerSet = new Set(asStringArray(rolesMap["kube_node"]));
    const etcdSet = new Set(asStringArray(rolesMap["etcd"]));
    return Object.entries(hostsMap).map(([hostname, rawVal]) => ({
      hostname,
      ip: extractHostIp(rawVal),
      isControlPlane: cpSet.has(hostname),
      isWorker: workerSet.has(hostname),
      isEtcd: etcdSet.has(hostname),
    }));
  }, [hostsMap, rolesMap]);

  const visibleHostRows = useMemo(() => {
    const q = hostSearch.trim().toLowerCase();
    if (!q) {
      return hostRows;
    }
    return hostRows.filter((row) => {
      const roleStr = [
        row.isControlPlane ? "kube_control_plane control" : "",
        row.isWorker ? "kube_node worker" : "",
        row.isEtcd ? "etcd" : "",
      ].join(" ");
      return (
        row.hostname.toLowerCase().includes(q) ||
        row.ip.toLowerCase().includes(q) ||
        roleStr.includes(q)
      );
    });
  }, [hostRows, hostSearch]);

  const updateHostIp = useCallback((hostname: string, nextIp: string) => {
    setInventoryDoc((prev) => {
      const next = cloneDeepRecord(prev);
      const nextHosts = asRecord(next["hosts"]);
      const existing = nextHosts[hostname];
      if (typeof existing === "object" && existing !== null && !Array.isArray(existing)) {
        const hostObj = asRecord(existing);
        hostObj["ip"] = nextIp;
        nextHosts[hostname] = hostObj;
      } else {
        nextHosts[hostname] = nextIp;
      }
      next["hosts"] = nextHosts;
      return next;
    });
    setEditMode("structured");
  }, []);

  const toggleHostRole = useCallback(
    (hostname: string, roleKey: "kube_control_plane" | "kube_node" | "etcd", enabled: boolean) => {
      setInventoryDoc((prev) => {
        const next = cloneDeepRecord(prev);
        const nextRoles = asRecord(next["roles"]);
        const currentList = asStringArray(nextRoles[roleKey]);
        const set = new Set(currentList);
        if (enabled) {
          set.add(hostname);
        } else {
          set.delete(hostname);
        }
        nextRoles[roleKey] = Array.from(set);
        if (!nextRoles["k8s_cluster"]) {
          nextRoles["k8s_cluster"] = ["kube_control_plane", "kube_node"];
        }
        next["roles"] = nextRoles;
        return next;
      });
      setEditMode("structured");
    },
    [],
  );

  const removeHosts = useCallback((names: readonly string[]) => {
    if (names.length === 0) {
      return;
    }
    const removeSet = new Set(names);
    setInventoryDoc((prev) => {
      const next = cloneDeepRecord(prev);
      const nextHosts = asRecord(next["hosts"]);
      for (const h of removeSet) {
        delete nextHosts[h];
      }
      next["hosts"] = nextHosts;

      const nextRoles = asRecord(next["roles"]);
      for (const roleKey of ["kube_control_plane", "kube_node", "etcd"]) {
        nextRoles[roleKey] = asStringArray(nextRoles[roleKey]).filter((h) => !removeSet.has(h));
      }
      next["roles"] = nextRoles;
      return next;
    });
    setSelectedHosts((prev) => {
      const next = new Set(prev);
      for (const h of removeSet) {
        next.delete(h);
      }
      return next;
    });
    setEditMode("structured");
  }, []);

  const handleAddSingleHost = () => {
    const h = singleHostName.trim();
    const ip = singleHostIp.trim();
    if (!h || !ip) {
      pushToast("error", "主机名与 IP 不能为空");
      return;
    }
    setInventoryDoc((prev) => {
      const next = cloneDeepRecord(prev);
      const nextHosts = asRecord(next["hosts"]);
      nextHosts[h] = ip;
      next["hosts"] = nextHosts;

      const nextRoles = asRecord(next["roles"]);
      const workers = new Set(asStringArray(nextRoles["kube_node"]));
      workers.add(h);
      nextRoles["kube_node"] = Array.from(workers);
      if (!nextRoles["k8s_cluster"]) {
        nextRoles["k8s_cluster"] = ["kube_control_plane", "kube_node"];
      }
      next["roles"] = nextRoles;
      return next;
    });
    setSingleHostName("");
    setSingleHostIp("");
    setEditMode("structured");
    pushToast("success", `已添加 ${h}`);
  };

  const handleBatchAdd = () => {
    const expanded = expandIpv4Range(batchPrefix, batchRange);
    if (expanded.error) {
      pushToast("error", expanded.error);
      return;
    }
    setInventoryDoc((prev) => {
      const next = cloneDeepRecord(prev);
      const nextHosts = asRecord(next["hosts"]);
      const nextRoles = asRecord(next["roles"]);
      const cpSet = new Set(asStringArray(nextRoles["kube_control_plane"]));
      const workerSet = new Set(asStringArray(nextRoles["kube_node"]));
      const etcdSet = new Set(asStringArray(nextRoles["etcd"]));

      for (const item of expanded.items) {
        nextHosts[item.hostname] = item.ip;
        if (batchRole === "kube_node") {
          workerSet.add(item.hostname);
        } else if (batchRole === "kube_control_plane") {
          cpSet.add(item.hostname);
        } else {
          cpSet.add(item.hostname);
          etcdSet.add(item.hostname);
        }
      }
      next["hosts"] = nextHosts;
      nextRoles["kube_control_plane"] = Array.from(cpSet);
      nextRoles["kube_node"] = Array.from(workerSet);
      nextRoles["etcd"] = Array.from(etcdSet);
      if (!nextRoles["k8s_cluster"]) {
        nextRoles["k8s_cluster"] = ["kube_control_plane", "kube_node"];
      }
      next["roles"] = nextRoles;
      return next;
    });
    setBatchRange("");
    setEditMode("structured");
    pushToast("success", `已批量生成 ${expanded.items.length} 台节点`);
  };

  const buildWritePayload = (): InventoryWritePayload => {
    if (editMode === "yaml") {
      return { mode: "yaml", raw_yaml: rawYaml };
    }
    return { mode: "structured", inventory: inventoryDoc };
  };

  const handleValidate = async () => {
    setValidating(true);
    try {
      const res = await validateInventory(buildWritePayload());
      setDerived(res.derived);
      setWarnings(res.warnings);
      pushToast(res.valid ? "success" : "error", res.valid ? `校验通过（${res.hosts_count} 节点）` : "校验未通过");
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "校验失败");
    } finally {
      setValidating(false);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      const res = await saveInventory(buildWritePayload());
      setDerived(res.derived);
      const refreshed = await fetchInventory();
      setInventoryDoc(refreshed.inventory);
      setRawYaml(refreshed.masked_yaml);
      setDerived(refreshed.derived);
      setEditMode("structured");
      pushToast("success", res.saved ? "已保存" : "保存完成");
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "保存失败");
    } finally {
      setSaving(false);
    }
  };

  const handleProbe = async () => {
    setProbing(true);
    try {
      const targets = selectedHosts.size > 0 ? Array.from(selectedHosts) : [];
      const res = await probeInventoryHosts(targets);
      setProbeResults(res.results);
      setProbeOpen(true);
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "探测失败");
    } finally {
      setProbing(false);
    }
  };

  const hostColumns: DataTableColumn<HostRow>[] = [
    {
      key: "hostname",
      header: "主机名",
      render: (row) => <span className="font-mono-code font-medium">{row.hostname}</span>,
    },
    {
      key: "ip",
      header: "节点 IP",
      tabularNums: true,
      render: (row) => (
        <input
          type="text"
          value={row.ip}
          aria-label={`${row.hostname} IP`}
          onChange={(e) => updateHostIp(row.hostname, e.target.value)}
          placeholder="10.0.0.61"
          className="park-input tabular-nums font-mono-code max-w-[160px]"
        />
      ),
    },
    {
      key: "cp",
      header: "控制面",
      render: (row) => (
        <Checkbox
          checked={row.isControlPlane}
          ariaLabel={`${row.hostname} 控制面`}
          onChange={(checked) => toggleHostRole(row.hostname, "kube_control_plane", checked)}
        />
      ),
    },
    {
      key: "worker",
      header: "工作节点",
      render: (row) => (
        <Checkbox
          checked={row.isWorker}
          ariaLabel={`${row.hostname} 工作节点`}
          onChange={(checked) => toggleHostRole(row.hostname, "kube_node", checked)}
        />
      ),
    },
    {
      key: "etcd",
      header: "etcd",
      render: (row) => (
        <Checkbox
          checked={row.isEtcd}
          ariaLabel={`${row.hostname} etcd`}
          onChange={(checked) => toggleHostRole(row.hostname, "etcd", checked)}
        />
      ),
    },
    {
      key: "actions",
      header: "操作",
      align: "right",
      render: (row) => (
        <Button
          variant="ghost"
          size="sm"
          icon={<Trash2 size={13} />}
          onClick={() => removeHosts([row.hostname])}
        >
          删除
        </Button>
      ),
    },
  ];

  const probeColumns: DataTableColumn<HostProbeResult>[] = [
    {
      key: "host",
      header: "节点",
      render: (r) => <span className="font-mono-code font-medium">{r.host}</span>,
    },
    {
      key: "ip",
      header: "IP",
      tabularNums: true,
      render: (r) => <span className="font-mono-code text-xs">{r.ip}</span>,
    },
    {
      key: "reachable",
      header: "状态",
      render: (r) => (
        <span
          className="park-badge"
          style={{ borderColor: r.reachable ? "var(--success)" : "var(--danger)" }}
        >
          {r.reachable ? `可达 · UID ${r.uid ?? "—"}` : "不可达"}
        </span>
      ),
    },
    {
      key: "os",
      header: "系统 / 架构",
      render: (r) => (
        <span className="text-xs">
          {r.os_pretty || "—"} {r.arch ? `(${r.arch})` : ""}
        </span>
      ),
    },
    {
      key: "nics",
      header: "网卡",
      render: (r) => <span className="font-mono-code text-xs">{r.nics.join(", ") || "—"}</span>,
    },
    {
      key: "latency",
      header: "延迟",
      align: "right",
      tabularNums: true,
      render: (r) => <span className="font-mono-code text-xs">{r.latency_ms} ms</span>,
    },
  ];

  const publicRegistriesList = asStringArray(varsRec["public_registries"]);
  const extraAptList = asStringArray(varsRec["extra_apt_packages"]);
  const customManifestsList = asStringArray(varsRec["custom_manifests"]);
  const currentCertValidity = asString(varsRec["cert_validity"], "867240h").trim() || "867240h";
  const certValidityOptions = useMemo<readonly SelectOption[]>(() => {
    if (VALIDITY_OPTIONS.some((opt) => opt.value === currentCertValidity)) {
      return VALIDITY_OPTIONS;
    }
    return [{ value: currentCertValidity, label: currentCertValidity }, ...VALIDITY_OPTIONS];
  }, [currentCertValidity]);

  return (
    <div className="space-y-4 max-w-6xl">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="park-badge">
            {editMode === "yaml" ? "YAML 源码模式" : "结构化表单模式"}
          </span>
          {derived?.first_control_plane ? (
            <span className="park-badge font-mono-code tabular-nums">
              首控节点 · {derived.first_control_plane}
            </span>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            icon={<Activity size={13} />}
            disabled={probing || loading}
            onClick={() => void handleProbe()}
          >
            {probing ? "探测中…" : "探测连通性"}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<CheckCircle2 size={13} />}
            disabled={validating || loading}
            onClick={() => void handleValidate()}
          >
            校验
          </Button>
          <Button
            variant="primary"
            size="sm"
            icon={<Save size={13} />}
            disabled={saving || loading}
            onClick={() => void handleSave()}
          >
            {saving ? "保存中…" : "保存"}
          </Button>
        </div>
      </div>

      {warnings.length > 0 ? (
        <div className="park-card p-3 space-y-1">
          {warnings.map((w, idx) => (
            <div key={idx} className="flex items-center gap-2 text-xs" style={{ color: "var(--warning)" }}>
              <ShieldAlert size={14} className="shrink-0" />
              <span>{w}</span>
            </div>
          ))}
        </div>
      ) : null}

      <StateView loading={loading} error={error} onRetry={() => void loadData()}>
        <Tabs
          value={activeTab}
          onChange={setActiveTab}
          items={[
            {
              value: "hosts",
              label: "主机拓扑",
              badge: hostRows.length,
              content: (
                <div className="space-y-4">
                  <div className="park-card p-4 grid grid-cols-1 lg:grid-cols-12 gap-3 items-end">
                    <div className="lg:col-span-2">
                      <label className="block mb-1 text-xs font-medium">主机名前缀</label>
                      <input
                        type="text"
                        value={batchPrefix}
                        onChange={(e) => setBatchPrefix(e.target.value)}
                        placeholder="c23-p"
                        className="park-input font-mono-code"
                      />
                    </div>
                    <div className="lg:col-span-4">
                      <label className="block mb-1 text-xs font-medium">IPv4 地址段</label>
                      <input
                        type="text"
                        value={batchRange}
                        onChange={(e) => setBatchRange(e.target.value)}
                        placeholder="192.168.26.61-66"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                    <div className="lg:col-span-4">
                      <Select
                        label="默认角色组"
                        value={batchRole}
                        options={BATCH_ROLE_OPTIONS}
                        onChange={(val) => setBatchRole(val as BatchRolePreset)}
                      />
                    </div>
                    <div className="lg:col-span-2">
                      <Button
                        variant="secondary"
                        icon={<Plus size={14} />}
                        onClick={handleBatchAdd}
                        className="w-full"
                      >
                        批量添加
                      </Button>
                    </div>
                  </div>

                  <div className="park-card p-3.5 flex flex-wrap items-end justify-between gap-3">
                    <div className="flex flex-wrap items-end gap-2 flex-1 min-w-[240px]">
                      <div className="w-40">
                        <label className="block mb-1 text-xs font-medium">节点主机名</label>
                        <input
                          type="text"
                          value={singleHostName}
                          onChange={(e) => setSingleHostName(e.target.value)}
                          placeholder="c23-p67"
                          className="park-input font-mono-code"
                        />
                      </div>
                      <div className="w-40">
                        <label className="block mb-1 text-xs font-medium">节点 IP</label>
                        <input
                          type="text"
                          value={singleHostIp}
                          onChange={(e) => setSingleHostIp(e.target.value)}
                          placeholder="10.0.0.67"
                          className="park-input tabular-nums font-mono-code"
                        />
                      </div>
                      <Button variant="secondary" onClick={handleAddSingleHost}>
                        添加节点
                      </Button>
                    </div>

                    <div className="flex flex-wrap items-center gap-2">
                      <input
                        type="search"
                        value={hostSearch}
                        onChange={(e) => setHostSearch(e.target.value)}
                        placeholder="搜索主机名 / IP / 角色"
                        aria-label="过滤节点"
                        className="park-input w-52"
                      />
                      {selectedHosts.size > 0 ? (
                        <Button
                          variant="danger"
                          size="sm"
                          icon={<Trash2 size={13} />}
                          onClick={() => removeHosts(Array.from(selectedHosts))}
                        >
                          删除 ({selectedHosts.size})
                        </Button>
                      ) : null}
                    </div>
                  </div>

                  <DataTable
                    rows={visibleHostRows}
                    getRowKey={(row) => row.hostname}
                    columns={hostColumns}
                    selection={{
                      selectedKeys: selectedHosts,
                      onChangeSelectedKeys: setSelectedHosts,
                    }}
                    emptyText="暂无节点"
                  />

                  {derived ? (
                    <div className="park-card p-3.5 grid grid-cols-1 md:grid-cols-2 gap-3 text-xs">
                      <div>
                        <span className="block mb-1" style={{ color: "var(--text-muted)" }}>
                          etcd 端点列表
                        </span>
                        <code className="font-mono-code tabular-nums break-all">
                          {derived.etcd_endpoints_str || "—"}
                        </code>
                      </div>
                      <div>
                        <span className="block mb-1" style={{ color: "var(--text-muted)" }}>
                          etcd 初始集群
                        </span>
                        <code className="font-mono-code tabular-nums break-all">
                          {derived.etcd_initial_cluster || "—"}
                        </code>
                      </div>
                    </div>
                  ) : null}
                </div>
              ),
            },
            {
              value: "privilege",
              label: "连接提权",
              content: (
                <div className="park-card p-5 space-y-4 max-w-3xl">
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                    <div>
                      <label className="block mb-1 text-xs font-medium">SSH 登录账号</label>
                      <input
                        type="text"
                        value={asString(varsRec["ansible_user"], "root")}
                        onChange={(e) => updateVarField("ansible_user", e.target.value)}
                        placeholder="root"
                        className="park-input font-mono-code"
                      />
                    </div>
                    <div>
                      <label className="block mb-1 text-xs font-medium">SSH 登录口令</label>
                      <input
                        type="password"
                        value={asString(varsRec["ansible_ssh_pass"], "")}
                        onChange={(e) => updateVarField("ansible_ssh_pass", e.target.value)}
                        placeholder="********"
                        className="park-input font-mono-code"
                      />
                    </div>
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                    <div>
                      <label className="block mb-1 text-xs font-medium">SSH 私钥路径</label>
                      <input
                        type="text"
                        value={asString(varsRec["ansible_ssh_private_key_file"], "")}
                        onChange={(e) => {
                          const raw = e.target.value;
                          updateVarField(
                            "ansible_ssh_private_key_file",
                            raw.trim().length > 0 ? raw : undefined,
                          );
                        }}
                        placeholder="~/.ssh/rtkk_ed25519"
                        className="park-input font-mono-code"
                      />
                    </div>
                    <div>
                      <label className="block mb-1 text-xs font-medium">SSH 端口</label>
                      <input
                        type="number"
                        min={1}
                        max={65535}
                        value={asString(varsRec["ansible_port"], "22")}
                        onChange={(e) => {
                          const raw = e.target.value.trim();
                          if (!raw) {
                            updateVarField("ansible_port", "");
                            return;
                          }
                          const parsed = Number.parseInt(raw, 10);
                          if (Number.isFinite(parsed) && parsed >= 1 && parsed <= 65535) {
                            updateVarField("ansible_port", parsed);
                          }
                        }}
                        onBlur={() => {
                          if (varsRec["ansible_port"] === "") {
                            updateVarField("ansible_port", undefined);
                          }
                        }}
                        placeholder="22"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                  </div>

                  <div className="pt-2 border-t space-y-4" style={{ borderColor: "var(--border)" }}>
                    <Switch
                      checked={asBool(varsRec["ansible_become"], false)}
                      onChange={(checked) => updateVarField("ansible_become", checked)}
                      label="非 root 提权执行"
                      description="通过 sudo 或 su 切换至目标特权账号"
                    />
                    <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                      <Select
                        label="提权方式"
                        value={asString(varsRec["ansible_become_method"], "sudo")}
                        options={BECOME_METHOD_OPTIONS}
                        onChange={(val) => updateVarField("ansible_become_method", val as BecomeMethod)}
                      />
                      <div>
                        <label className="block mb-1 text-xs font-medium">目标账号</label>
                        <input
                          type="text"
                          value={asString(varsRec["ansible_become_user"], "root")}
                          onChange={(e) => updateVarField("ansible_become_user", e.target.value)}
                          placeholder="root"
                          className="park-input font-mono-code"
                        />
                      </div>
                      <div>
                        <label className="block mb-1 text-xs font-medium">提权口令</label>
                        <input
                          type="password"
                          value={asString(varsRec["ansible_become_pass"], "")}
                          onChange={(e) => updateVarField("ansible_become_pass", e.target.value)}
                          placeholder="********"
                          className="park-input font-mono-code"
                        />
                      </div>
                    </div>
                    <Switch
                      checked={asBool(varsRec["ansible_ssh_host_key_checking"], false)}
                      onChange={(checked) => updateVarField("ansible_ssh_host_key_checking", checked)}
                      label="严格校验 SSH 主机公钥"
                      description="启用 known_hosts 指纹核验"
                    />
                  </div>
                </div>
              ),
            },
            {
              value: "registry",
              label: "镜像私库",
              content: (
                <div className="space-y-4 max-w-4xl">
                  <div className="park-card p-5 space-y-4">
                    <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                      <div>
                        <label className="block mb-1 text-xs font-medium">私库项目地址</label>
                        <input
                          type="text"
                          value={asString(registryRec["url"], "")}
                          onChange={(e) => updateNestedVarField("registry", "url", e.target.value)}
                          placeholder="10.0.0.10/rtkk"
                          className="park-input tabular-nums font-mono-code"
                        />
                      </div>
                      <div>
                        <label className="block mb-1 text-xs font-medium">私库主机端口</label>
                        <input
                          type="text"
                          value={asString(registryRec["host"], "")}
                          onChange={(e) => updateNestedVarField("registry", "host", e.target.value)}
                          placeholder="10.0.0.10"
                          className="park-input tabular-nums font-mono-code"
                        />
                      </div>
                      <Select
                        label="传输协议"
                        value={asString(registryRec["scheme"], "auto")}
                        options={REGISTRY_SCHEME_OPTIONS}
                        onChange={(val) =>
                          updateNestedVarField("registry", "scheme", val as RegistryScheme)
                        }
                      />
                    </div>

                    <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                      <div>
                        <label className="block mb-1 text-xs font-medium">私库账号</label>
                        <input
                          type="text"
                          value={asString(registryRec["username"], "")}
                          onChange={(e) => updateNestedVarField("registry", "username", e.target.value)}
                          placeholder="admin"
                          className="park-input font-mono-code"
                        />
                      </div>
                      <div>
                        <label className="block mb-1 text-xs font-medium">私库口令</label>
                        <input
                          type="password"
                          value={asString(registryRec["password"], "")}
                          onChange={(e) => updateNestedVarField("registry", "password", e.target.value)}
                          placeholder="********"
                          className="park-input font-mono-code"
                        />
                      </div>
                      <div>
                        <label className="block mb-1 text-xs font-medium">Basic 凭据摘要</label>
                        <input
                          type="password"
                          value={asString(registryRec["auth_basic"], "")}
                          onChange={(e) => updateNestedVarField("registry", "auth_basic", e.target.value)}
                          placeholder="********"
                          className="park-input font-mono-code"
                        />
                      </div>
                    </div>

                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 pt-2">
                      <Switch
                        checked={asBool(registryRec["insecure"], true)}
                        onChange={(checked) => updateNestedVarField("registry", "insecure", checked)}
                        label="私库免证书校验"
                        description="允许自签名 TLS 证书连接"
                      />
                      <Switch
                        checked={asBool(registryRec["harbor_project_public"], true)}
                        onChange={(checked) =>
                          updateNestedVarField("registry", "harbor_project_public", checked)
                        }
                        label="Harbor 公开项目"
                        description="自动建项时设为公开拉取权限"
                      />
                    </div>
                  </div>

                  <div className="park-card p-5 space-y-4">
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                      <div data-testid="enable-public-registry-mirrors-box">
                        <Switch
                          checked={asBool(varsRec["enable_public_registry_mirrors"], false)}
                          onChange={(checked) => {
                            setInventoryDoc((prev) => {
                              const next = cloneDeepRecord(prev);
                              const nextVars = asRecord(next["vars"]);
                              nextVars["enable_public_registry_mirrors"] = checked;
                              if (checked && !Array.isArray(nextVars["public_registry_mirrors"])) {
                                nextVars["public_registry_mirrors"] = DEFAULT_PUBLIC_REGISTRY_MIRRORS.map(
                                  (item) => ({ ...item }),
                                );
                              }
                              if (checked && nextVars["auto_create_mirror_projects"] === undefined) {
                                nextVars["auto_create_mirror_projects"] = true;
                              }
                              next["vars"] = nextVars;
                              return next;
                            });
                            setEditMode("structured");
                          }}
                          label="公网源透明映射"
                          description="启用 containerd override_path 按源站映射至私库项目"
                        />
                      </div>
                      <div data-testid="auto-create-mirror-projects-box">
                        <Switch
                          checked={asBool(varsRec["auto_create_mirror_projects"], true)}
                          onChange={(checked) => updateVarField("auto_create_mirror_projects", checked)}
                          label="自动建映射项目"
                          description="部署时在 Harbor 自动创建去重源站项目"
                        />
                      </div>
                    </div>

                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                      <div data-testid="public-registries-insecure-box">
                        <Switch
                          checked={asBool(varsRec["public_registries_insecure"], false)}
                          onChange={(checked) => updateVarField("public_registries_insecure", checked)}
                          label="公网镜像源 DNS 劫持免证书双栈"
                          description="开启 public_registries_insecure，生成 skip_verify 与明文回退 hosts.toml"
                        />
                      </div>
                      <div data-testid="public-registries-auth-box">
                        <Switch
                          checked={asBool(varsRec["public_registries_auth"], false)}
                          onChange={(checked) => updateVarField("public_registries_auth", checked)}
                          label="公网镜像源附加私库认证"
                          description="开启 public_registries_auth，向劫持域名注入 Basic 凭据且收敛 0600"
                        />
                      </div>
                    </div>

                    <div>
                      <label className="block mb-1 text-xs font-medium">
                        公网镜像源域名列表（每行一项）
                      </label>
                      <textarea
                        rows={5}
                        value={publicRegistriesList.join("\n")}
                        onChange={(e) =>
                          updateVarField(
                            "public_registries",
                            e.target.value
                              .split("\n")
                              .map((s) => s.trim())
                              .filter(Boolean),
                          )
                        }
                        placeholder="docker.io"
                        className="park-textarea"
                      />
                    </div>

                    <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                      <div>
                        <label className="block mb-1 text-xs font-medium">镜像架构平台</label>
                        <input
                          type="text"
                          value={asString(imageSyncRec["platform"], "linux/amd64")}
                          onChange={(e) => updateNestedVarField("image_sync", "platform", e.target.value)}
                          placeholder="linux/amd64"
                          className="park-input font-mono-code"
                        />
                      </div>
                      <div>
                        <label className="block mb-1 text-xs font-medium">分层并发数</label>
                        <input
                          type="number"
                          min={1}
                          max={16}
                          value={asNumber(imageSyncRec["layer_concurrency"], 4)}
                          onChange={(e) =>
                            updateNestedVarField(
                              "image_sync",
                              "layer_concurrency",
                              Number.parseInt(e.target.value || "4", 10),
                            )
                          }
                          className="park-input tabular-nums font-mono-code"
                        />
                      </div>
                      <div>
                        <label className="block mb-1 text-xs font-medium">拉取超时（秒）</label>
                        <input
                          type="number"
                          min={30}
                          value={asNumber(imageSyncRec["http_timeout_secs"], 600)}
                          onChange={(e) =>
                            updateNestedVarField(
                              "image_sync",
                              "http_timeout_secs",
                              Number.parseInt(e.target.value || "600", 10),
                            )
                          }
                          className="park-input tabular-nums font-mono-code"
                        />
                      </div>
                    </div>
                  </div>
                </div>
              ),
            },
            {
              value: "network",
              label: "集群网络",
              content: (
                <div className="park-card p-5 space-y-4 max-w-4xl">
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                    <div>
                      <label className="block mb-1 text-xs font-medium">控制面高可用端点</label>
                      <input
                        type="text"
                        value={asString(varsRec["control_plane_endpoint"], "")}
                        onChange={(e) => updateVarField("control_plane_endpoint", e.target.value)}
                        placeholder="10.0.0.68:6443"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                    <div>
                      <label className="block mb-1 text-xs font-medium">VIP 绑定网卡</label>
                      <input
                        type="text"
                        value={asString(varsRec["vip_interface"], "ens33")}
                        onChange={(e) => updateVarField("vip_interface", e.target.value)}
                        placeholder="ens33"
                        className="park-input font-mono-code"
                      />
                    </div>
                    <Select
                      label="CNI 网络插件"
                      value={asString(varsRec["cni_plugin"], "cilium")}
                      options={CNI_OPTIONS}
                      onChange={(val) => updateVarField("cni_plugin", val as CniPlugin)}
                    />
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                    <div>
                      <label className="block mb-1 text-xs font-medium">Pod 网段 CIDR</label>
                      <input
                        type="text"
                        value={asString(varsRec["pod_subnet"], "")}
                        onChange={(e) => updateVarField("pod_subnet", e.target.value)}
                        placeholder="10.233.0.0/18"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                    <div>
                      <label className="block mb-1 text-xs font-medium">Service 网段 CIDR</label>
                      <input
                        type="text"
                        value={asString(varsRec["service_subnet"], "")}
                        onChange={(e) => updateVarField("service_subnet", e.target.value)}
                        placeholder="10.233.64.0/18"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                    <div>
                      <label className="block mb-1 text-xs font-medium">节点子网掩码位</label>
                      <input
                        type="text"
                        value={asString(varsRec["cluster_pool_mask_size"], "24")}
                        onChange={(e) => updateVarField("cluster_pool_mask_size", e.target.value)}
                        placeholder="24"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-4 gap-4">
                    <div>
                      <label className="block mb-1 text-xs font-medium">集群 DNS 地址</label>
                      <input
                        type="text"
                        value={asString(varsRec["cluster_dns_ip"], "")}
                        onChange={(e) => updateVarField("cluster_dns_ip", e.target.value)}
                        placeholder="10.233.64.10"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                    <div>
                      <label className="block mb-1 text-xs font-medium">NodeLocalDNS 地址</label>
                      <input
                        type="text"
                        value={asString(varsRec["nodelocaldns_ip"], "")}
                        onChange={(e) => updateVarField("nodelocaldns_ip", e.target.value)}
                        placeholder="169.254.20.10"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                    <div>
                      <label className="block mb-1 text-xs font-medium">上游 DNS</label>
                      <input
                        type="text"
                        value={asString(varsRec["dns_upstream"], "")}
                        onChange={(e) => updateVarField("dns_upstream", e.target.value)}
                        placeholder="10.0.0.111"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                    <div>
                      <label className="block mb-1 text-xs font-medium">集群域名</label>
                      <input
                        type="text"
                        value={asString(varsRec["cluster_domain"], "cluster.local")}
                        onChange={(e) => updateVarField("cluster_domain", e.target.value)}
                        placeholder="cluster.local"
                        className="park-input font-mono-code"
                      />
                    </div>
                  </div>

                  {asString(varsRec["cni_plugin"], "cilium") === "cilium" ? (
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 pt-2">
                      <Switch
                        checked={asBool(varsRec["cilium_kube_proxy_replacement"], false)}
                        onChange={(checked) =>
                          updateVarField("cilium_kube_proxy_replacement", checked)
                        }
                        label="Cilium KPR 替代 kube-proxy"
                        description="开启 cilium_kube_proxy_replacement 并注入 API Server 端点"
                      />
                      <Switch
                        checked={asBool(varsRec["cilium_l2_announcements"], false)}
                        onChange={(checked) => {
                          setInventoryDoc((prev) => {
                            const next = cloneDeepRecord(prev);
                            const nextVars = asRecord(next["vars"]);
                            nextVars["cilium_l2_announcements"] = checked;
                            if (checked && !asBool(nextVars["cilium_kube_proxy_replacement"], false)) {
                              nextVars["cilium_kube_proxy_replacement"] = true;
                            }
                            next["vars"] = nextVars;
                            return next;
                          });
                          setEditMode("structured");
                        }}
                        label="Cilium L2 负载均衡宣告"
                        description="开启 cilium_l2_announcements 与 externalIPs 支持 VIP 池"
                      />
                    </div>
                  ) : null}

                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 pt-2">
                    <Switch
                      checked={asBool(addonsRec["nodelocaldns"], true)}
                      onChange={(checked) => updateNestedVarField("addons", "nodelocaldns", checked)}
                      label="启用 NodeLocalDNS"
                      description="节点级 DNS 缓存加速"
                    />
                    <Switch
                      checked={asBool(addonsRec["openebs_localpv"], true)}
                      onChange={(checked) =>
                        updateNestedVarField("addons", "openebs_localpv", checked)
                      }
                      label="启用 OpenEBS LocalPV"
                      description="本地持久化存储卷插件"
                    />
                  </div>
                </div>
              ),
            },
            {
              value: "versions",
              label: "组件版本",
              content: (
                <div className="park-card p-5 space-y-4 max-w-4xl">
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                    {[
                      { key: "kube_version", label: "Kubernetes", placeholder: "v1.37.2" },
                      { key: "etcd_version", label: "etcd", placeholder: "v3.7.4" },
                      { key: "containerd_version", label: "containerd", placeholder: "2.4.2" },
                      { key: "runc_version", label: "runc", placeholder: "v1.5.3" },
                      { key: "cni_version", label: "CNI Plugins", placeholder: "v1.10.0" },
                      { key: "helm_version", label: "Helm", placeholder: "v4.3.2" },
                      { key: "coredns_version", label: "CoreDNS", placeholder: "v1.14.6" },
                      { key: "kube_vip_version", label: "kube-vip", placeholder: "v1.3.2" },
                      { key: "pause_version", label: "pause", placeholder: "3.10.2" },
                      { key: "nodelocaldns_version", label: "NodeLocalDNS", placeholder: "1.27.1" },
                      { key: "cilium_version", label: "Cilium", placeholder: "v1.20.5" },
                      { key: "openebs_version", label: "OpenEBS", placeholder: "4.6.0" },
                    ].map((field) => (
                      <div key={field.key}>
                        <label className="block mb-1 text-xs font-medium">{field.label}</label>
                        <input
                          type="text"
                          value={asString(varsRec[field.key], "")}
                          onChange={(e) => updateVarField(field.key, e.target.value)}
                          placeholder={field.placeholder}
                          className="park-input tabular-nums font-mono-code"
                        />
                      </div>
                    ))}
                  </div>
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                    <div>
                      <label className="block mb-1 text-xs font-medium">Cilium Envoy</label>
                      <input
                        type="text"
                        value={asString(varsRec["cilium_envoy_version"], "")}
                        onChange={(e) => updateVarField("cilium_envoy_version", e.target.value)}
                        placeholder="v1.39.6"
                        className="park-input tabular-nums font-mono-code"
                      />
                    </div>
                    <Select
                      label="PKI 证书有效期"
                      value={currentCertValidity}
                      options={certValidityOptions}
                      onChange={(val) => updateVarField("cert_validity", val)}
                    />
                  </div>
                </div>
              ),
            },
            {
              value: "baseline",
              label: "系统基线",
              content: (
                <div className="park-card p-5 space-y-4 max-w-3xl">
                  <Switch
                    checked={asBool(varsRec["auto_install_apt_deps"], true)}
                    onChange={(checked) => updateVarField("auto_install_apt_deps", checked)}
                    label="自动收敛 Debian 13 系统依赖包"
                    description="优先读取离线归档，缺失时回退 apt-get"
                  />
                  <div>
                    <label className="block mb-1 text-xs font-medium">PKI 证书目录</label>
                    <input
                      type="text"
                      value={asString(varsRec["pki_dir"], "./pki")}
                      onChange={(e) => updateVarField("pki_dir", e.target.value)}
                      placeholder="./pki"
                      className="park-input font-mono-code"
                    />
                  </div>
                  <div>
                    <label className="block mb-1 text-xs font-medium">额外 APT 软件包（每行一项）</label>
                    <textarea
                      rows={4}
                      value={extraAptList.join("\n")}
                      onChange={(e) =>
                        updateVarField(
                          "extra_apt_packages",
                          e.target.value
                            .split("\n")
                            .map((s) => s.trim())
                            .filter(Boolean),
                        )
                      }
                      placeholder="chrony"
                      className="park-textarea"
                    />
                  </div>
                  <div>
                    <label className="block mb-1 text-xs font-medium">附加 Manifest 路径（每行一项）</label>
                    <textarea
                      rows={4}
                      value={customManifestsList.join("\n")}
                      onChange={(e) =>
                        updateVarField(
                          "custom_manifests",
                          e.target.value
                            .split("\n")
                            .map((s) => s.trim())
                            .filter(Boolean),
                        )
                      }
                      placeholder="configs/custom-addon.yaml"
                      className="park-textarea"
                    />
                  </div>
                </div>
              ),
            },
            {
              value: "yaml",
              label: "YAML 源码",
              content: (
                <div className="space-y-3">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-xs" style={{ color: "var(--text-muted)" }}>
                      未改动的 ******** 掩码字段在保存时自动保留原磁盘密文
                    </span>
                    <div className="flex items-center gap-2">
                      <Button
                        variant="secondary"
                        size="sm"
                        icon={<RefreshCw size={13} />}
                        onClick={() => void loadData()}
                      >
                        重载
                      </Button>
                      <Button
                        variant="secondary"
                        size="sm"
                        icon={<Copy size={13} />}
                        onClick={() => {
                          void copyTextToClipboard(rawYaml).then((ok) =>
                            pushToast(ok ? "success" : "error", ok ? "已复制" : "复制失败"),
                          );
                        }}
                      >
                        复制
                      </Button>
                    </div>
                  </div>
                  <textarea
                    rows={22}
                    value={rawYaml}
                    aria-label="inventory.yaml 源码"
                    onChange={(e) => {
                      setRawYaml(e.target.value);
                      setEditMode("yaml");
                    }}
                    spellCheck={false}
                    className="park-textarea"
                  />
                </div>
              ),
            },
          ]}
        />
      </StateView>

      <Dialog
        open={probeOpen}
        onOpenChange={setProbeOpen}
        title="节点连通性结果"
        maxWidthClass="max-w-4xl"
      >
        <div className="space-y-3">
          <DataTable
            rows={probeResults}
            getRowKey={(r) => r.host}
            columns={probeColumns}
            emptyText="无探测节点"
          />
          {probeResults.some((r) => r.error || r.missing_packages.length > 0) ? (
            <div className="space-y-1.5 text-xs">
              {probeResults.map((r) =>
                r.error || r.missing_packages.length > 0 ? (
                  <div
                    key={r.host}
                    className="p-2 border font-mono-code"
                    style={{
                      borderColor: "var(--border)",
                      backgroundColor: "var(--surface)",
                      borderRadius: "var(--radius-md)",
                    }}
                  >
                    <strong>{r.host}:</strong>{" "}
                    {r.error
                      ? r.error
                      : `缺失包: ${r.missing_packages.join(", ")}`}
                  </div>
                ) : null,
              )}
            </div>
          ) : null}
        </div>
      </Dialog>
    </div>
  );
}
