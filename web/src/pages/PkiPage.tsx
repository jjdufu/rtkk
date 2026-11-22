import { CheckCircle2, Download, Plus, RefreshCw, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "../components/primitives/Button";
import { DataTable, type DataTableColumn } from "../components/primitives/DataTable";
import { Dialog } from "../components/primitives/Dialog";
import { Select, type SelectOption } from "../components/primitives/Select";
import { StateView } from "../components/primitives/StateView";
import { Switch } from "../components/primitives/Switch";
import { useToast } from "../components/primitives/Toast";
import { ApiClientError } from "../lib/api/client";
import {
  fetchBackupsList,
  fetchCertificates,
  generateCertificate,
  triggerBackupsArchiveDownload,
  verifyCertificate,
  type BackupItem,
  type CertificateItem,
  type VerifyCertificateResponse,
} from "../lib/api/pki";
import { formatBytes, formatTimestampMs } from "../lib/format";

const CERT_VALIDITY_OPTIONS: readonly SelectOption[] = [
  { value: "867240h", label: "99 年 (867240h)" },
  { value: "10y", label: "10 年 (10y)" },
  { value: "1y", label: "1 年 (1y)" },
];

function stripCertNameFromPath(relPath: string): string {
  const file = relPath.split("/").pop() ?? relPath;
  return file.replace(/\.(crt|pem)$/i, "");
}

export function PkiPage() {
  const { pushToast } = useToast();
  const [certs, setCerts] = useState<CertificateItem[]>([]);
  const [backups, setBackups] = useState<BackupItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [genOpen, setGenOpen] = useState(false);
  const [certName, setCertName] = useState("");
  const [certCn, setCertCn] = useState("");
  const [isCa, setIsCa] = useState(false);
  const [caName, setCaName] = useState("__self__");
  const [validity, setValidity] = useState("867240h");
  const [sansText, setSansText] = useState("");
  const [generating, setGenerating] = useState(false);

  const [verifyReport, setVerifyReport] = useState<{
    certPath: string;
    result: VerifyCertificateResponse;
  } | null>(null);

  const loadAll = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [certRes, bakRes] = await Promise.all([fetchCertificates(), fetchBackupsList()]);
      setCerts(certRes.items);
      setBackups(bakRes.items);
    } catch (err) {
      setError(err instanceof ApiClientError ? `${err.message} (${err.code})` : "证书与备份读取失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  const caOptions = useMemo<SelectOption[]>(() => {
    const opts: SelectOption[] = [{ value: "__self__", label: "自签名（无上级 CA）" }];
    for (const c of certs) {
      if (c.is_ca && c.has_matching_key) {
        const stem = stripCertNameFromPath(c.rel_path);
        opts.push({
          value: stem,
          label: `${stem} (CN=${c.cn})`,
        });
      }
    }
    return opts;
  }, [certs]);

  const handleGenerate = async () => {
    const trimmedName = certName.trim();
    const trimmedCn = certCn.trim();
    if (!trimmedName || !trimmedCn) {
      pushToast("error", "证书名称与 CN 不能为空");
      return;
    }
    setGenerating(true);
    try {
      const sans = sansText
        .split(/[\n,]+/)
        .map((s) => s.trim())
        .filter(Boolean);
      const res = await generateCertificate({
        name: trimmedName,
        cn: trimmedCn,
        is_ca: isCa,
        ca_name: isCa || caName === "__self__" ? null : caName,
        sans,
        validity,
      });
      setGenOpen(false);
      setCertName("");
      setCertCn("");
      setSansText("");
      pushToast("success", `已签发 ${res.cert_rel_path}`);
      await loadAll();
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "证书签发失败");
    } finally {
      setGenerating(false);
    }
  };

  const handleVerify = async (item: CertificateItem) => {
    const keyGuess = item.has_matching_key
      ? item.rel_path.replace(/\.(crt|pem)$/i, ".key")
      : null;
    const matchingCa = certs.find(
      (c) => c.is_ca && c.cn === item.issuer && c.rel_path !== item.rel_path,
    );
    try {
      const res = await verifyCertificate({
        cert_rel_path: item.rel_path,
        key_rel_path: keyGuess,
        ca_rel_path: matchingCa ? matchingCa.rel_path : null,
      });
      setVerifyReport({ certPath: item.rel_path, result: res });
    } catch (err) {
      pushToast("error", err instanceof ApiClientError ? err.message : "证书校验失败");
    }
  };

  const certColumns: DataTableColumn<CertificateItem>[] = [
    {
      key: "path",
      header: "证书路径",
      render: (c) => <span className="font-mono-code font-medium text-xs">{c.rel_path}</span>,
    },
    {
      key: "cn",
      header: "通用名 (CN)",
      render: (c) => <span className="font-mono-code text-xs">{c.cn}</span>,
    },
    {
      key: "type",
      header: "角色",
      render: (c) => (
        <span className="park-badge">{c.is_ca ? "根 CA" : "叶证书"}</span>
      ),
    },
    {
      key: "sans",
      header: "SAN 扩展",
      render: (c) => (
        <span className="font-mono-code text-xs" style={{ color: "var(--text-muted)" }}>
          {c.sans.length > 0 ? c.sans.join(", ") : "—"}
        </span>
      ),
    },
    {
      key: "expiry",
      header: "到期 / 剩余天数",
      tabularNums: true,
      render: (c) => (
        <span className="font-mono-code text-xs">
          {c.not_after.slice(0, 10)} · {c.days_remaining} 天
        </span>
      ),
    },
    {
      key: "key",
      header: "私钥状态",
      render: (c) => (
        <span className="park-badge">
          {c.has_matching_key ? "0600 已匹配" : "仅公钥"}
        </span>
      ),
    },
    {
      key: "actions",
      header: "操作",
      align: "right",
      render: (c) => (
        <Button
          variant="secondary"
          size="sm"
          icon={<CheckCircle2 size={13} />}
          onClick={() => void handleVerify(c)}
        >
          校验
        </Button>
      ),
    },
  ];

  const backupColumns: DataTableColumn<BackupItem>[] = [
    {
      key: "path",
      header: "归档相对路径",
      render: (b) => <span className="font-mono-code text-xs">{b.rel_path}</span>,
    },
    {
      key: "size",
      header: "文件体积",
      align: "right",
      tabularNums: true,
      render: (b) => <span className="font-mono-code text-xs">{formatBytes(b.size_bytes)}</span>,
    },
    {
      key: "mtime",
      header: "修改时间",
      align: "right",
      tabularNums: true,
      render: (b) => (
        <span className="font-mono-code text-xs" style={{ color: "var(--text-muted)" }}>
          {formatTimestampMs(b.modified_ms)}
        </span>
      ),
    },
  ];

  return (
    <div className="space-y-5 max-w-6xl">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="park-badge tabular-nums">证书 {certs.length}</span>
          <span className="park-badge tabular-nums">备份 {backups.length}</span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="primary"
            size="sm"
            icon={<Plus size={14} />}
            onClick={() => setGenOpen(true)}
          >
            签发证书
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<Download size={13} />}
            disabled={backups.length === 0}
            onClick={triggerBackupsArchiveDownload}
          >
            导出备份
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<RefreshCw size={13} />}
            onClick={() => void loadAll()}
          >
            刷新
          </Button>
        </div>
      </div>

      <StateView loading={loading} error={error} onRetry={() => void loadAll()}>
        <div className="space-y-6">
          <section className="space-y-2.5">
            <h2 className="text-sm font-semibold m-0 inline-flex items-center gap-1.5">
              <ShieldCheck size={15} />
              <span>X.509 证书清单（公开元数据）</span>
            </h2>
            <DataTable
              rows={certs}
              getRowKey={(c) => c.rel_path}
              columns={certColumns}
              emptyText="暂无证书文件"
              emptyAction={
                <Button variant="primary" size="sm" onClick={() => setGenOpen(true)}>
                  签发证书
                </Button>
              }
            />
          </section>

          <section className="space-y-2.5">
            <h2 className="text-sm font-semibold m-0">集群回收备份（rtkk_bak/）</h2>
            <DataTable
              rows={backups}
              getRowKey={(b) => b.rel_path}
              columns={backupColumns}
              emptyText="暂无备份文件"
            />
          </section>
        </div>
      </StateView>

      <Dialog
        open={genOpen}
        onOpenChange={setGenOpen}
        title="签发证书"
        footer={
          <>
            <Button variant="secondary" disabled={generating} onClick={() => setGenOpen(false)}>
              取消
            </Button>
            <Button variant="primary" disabled={generating} onClick={() => void handleGenerate()}>
              {generating ? "签发中…" : "签发证书"}
            </Button>
          </>
        }
      >
        <div className="space-y-3.5">
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label className="block mb-1 text-xs font-medium">证书文件名</label>
              <input
                type="text"
                name="name"
                value={certName}
                onChange={(e) => setCertName(e.target.value)}
                placeholder="harbor-tls"
                className="park-input font-mono-code"
              />
            </div>
            <div>
              <label className="block mb-1 text-xs font-medium">通用名 (CN)</label>
              <input
                type="text"
                name="cn"
                value={certCn}
                onChange={(e) => setCertCn(e.target.value)}
                placeholder="10.0.0.10"
                className="park-input tabular-nums font-mono-code"
              />
            </div>
          </div>

          <Switch
            checked={isCa}
            onChange={setIsCa}
            label="作为根证书机构 (CA)"
            description="开启 BasicConstraints CA:TRUE，可用于签发下级叶证书"
          />

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {!isCa ? (
              <Select
                label="签发 CA"
                value={caName}
                options={caOptions}
                onChange={setCaName}
              />
            ) : null}
            <Select
              label="有效期"
              value={validity}
              options={CERT_VALIDITY_OPTIONS}
              onChange={setValidity}
            />
          </div>

          <div>
            <label className="block mb-1 text-xs font-medium">SAN 列表（IP 或域名，逗号或换行分隔）</label>
            <textarea
              rows={3}
              name="sans"
              value={sansText}
              onChange={(e) => setSansText(e.target.value)}
              placeholder="10.0.0.10, harbor.internal"
              className="park-textarea"
            />
          </div>
        </div>
      </Dialog>

      <Dialog
        open={verifyReport !== null}
        onOpenChange={(open) => {
          if (!open) {
            setVerifyReport(null);
          }
        }}
        title="证书链核验结果"
        footer={
          <Button variant="primary" size="sm" onClick={() => setVerifyReport(null)}>
            关闭
          </Button>
        }
      >
        {verifyReport ? (
          <div className="space-y-3 text-xs">
            <div className="flex items-center justify-between">
              <span className="font-mono-code">{verifyReport.certPath}</span>
              <span
                className="park-badge"
                style={{
                  borderColor: verifyReport.result.valid ? "var(--success)" : "var(--danger)",
                }}
              >
                {verifyReport.result.valid ? "核验通过" : "核验失败"}
              </span>
            </div>
            <pre className="m-0 p-3 park-panel font-mono-code whitespace-pre-wrap break-words">
              {verifyReport.result.report}
            </pre>
          </div>
        ) : null}
      </Dialog>
    </div>
  );
}
