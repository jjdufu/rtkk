# 内网镜像仓库 TLS 加固与 HTTP 风险缓解指南

> [!NOTE]
> 本文档是 [运维准备与安全建议手册](../OPERATION_GUIDE.md) 的配套技术指南，详细阐述内网镜像仓库在不同协议模式（`scheme`）下的行为边界、明文 HTTP 风险与 `rtkk` 内置缓解机制，以及在 Debian 13 (`Trixie`) 环境下利用 `rtkk cert gen` 完成全链路 HTTPS 加固的实操步骤。

---

## 目录

- [1. 内网镜像仓库协议模式（`scheme`）与便利性设计](#1-内网镜像仓库协议模式scheme与便利性设计)
- [2. 明文 HTTP 的潜在风险与 `rtkk` 内置纵深防御](#2-明文-http-的潜在风险与-rtkk-内置纵深防御)
- [3. Debian 13 实战：使用 `rtkk cert gen` 签发仓库 CA 与 TLS 证书](#3-debian-13-实战使用-rtkk-cert-gen-签发仓库-ca-与-tls-证书)
- [4. 配置 Harbor / CNCF Distribution 启用 HTTPS](#4-配置-harbor--cncf-distribution-启用-https)
- [5. Debian 13 节点信任根 CA 与切换 `rtkk` 至严格 HTTPS](#5-debian-13-节点信任根-ca-与切换-rtkk-至严格-https)

---

## 1. 内网镜像仓库协议模式（`scheme`）与便利性设计

在纯物理隔离内网、离线交付初期或实验测试网段中，私有镜像仓库往往直接使用内网 IP（如 `10.0.0.10`）暴露 HTTP 端口，或者仅配置了未经操作系统信任库收录的自签名证书。为兼顾开箱即用体验与生产级严格加密，`rtkk` 在 `vars.registry` 中提供显式的协议与证书控制项：

| 配置字段 | 可选值 / 类型 | 说明 |
|----------|---------------|------|
| `registry.scheme` | `"auto"` \| `"https"` \| `"http"` | 传输层协议策略：<br>• `"auto"`：优先尝试 HTTPS `/v2/` 探测；当 `registry.insecure: true` 且对端未开启 TLS 时自动回退 HTTP<br>• `"https"`：强制全程使用 HTTPS，禁止任何协议回退<br>• `"http"`：显式使用明文 HTTP（适用于纯内网无 TLS 仓库） |
| `registry.insecure` | `bool`（默认 `false`） | 是否允许跳过目标私库的 TLS 证书校验或允许 `"auto"` 模式下回退至 HTTP |
| `registry.ca_cert_file` | PEM 文件路径（可选） | 为目标私库单独指定受信任的内部根 CA 证书路径（如 `"./pki/registry-ca.crt"`），无需关闭 `insecure` 即可校验自建 PKI 证书 |
| `image_sync.insecure` | `bool`（默认 `false`） | 独立控制 `pull-images` / `sync-images` 访问**上游源仓库**时的 TLS 证书校验策略 |

---

## 2. 明文 HTTP 的潜在风险与 `rtkk` 内置纵深防御

### 2.1 明文 HTTP 与跳过证书校验（`insecure: true`）的风险

在非严格隔离的共享办公网、跨机房链路或存在不可信主机的二层广播域中，使用明文 HTTP 或 `insecure: true` 存在两类主要风险：

1. **认证凭据被旁路窃听**：HTTP 明文传输时，网络路径上的嗅探者或 ARP 欺骗者可直接截获 `Authorization: Basic` 请求头或 `Bearer` 访问令牌。
2. **传输报文遭中间人替换（MITM）**：缺少受信服务端证书鉴权时，攻击者可能劫持目标 IP 并返回伪造的镜像数据流。

### 2.2 `rtkk` 内置的多层安全缓解机制

即使运维团队因环境限制暂时采用内网 HTTP 便利模式，`rtkk` 仍在客户端协议栈层面实施了多项纵深保护：

1. **未收到 `401` 挑战前绝不盲目发送凭据**：
   对未知端点、初始协议探测请求或跨域重定向地址，`rtkk` 不会预先附带 `Authorization` 敏感头；仅在目标仓库明确返回 `401 Unauthorized` 挑战且目标域受控时，才按 Basic 或 Bearer 规则发送凭据。
2. **Manifest 与 Blob 全链路 SHA256 摘要强校验**：
   拉取与同步过程中，`rtkk` 同时对 **Manifest 原文字节流**、**Config Blob** 以及每一个 **Layer Blob** 执行流式 SHA256 计算，并与预期的内容寻址摘要（Content-Addressable Digest）严格比对，同时校验字节长度与层归档结构。一旦发生任何单比特篡改或截断，立即中止并清理临时文件，拒绝生成不可信的 tar 归档。
3. **公网上游与内网私库 TLS 策略域隔离**：
   `image_sync.insecure` 与 `registry.insecure` 彼此独立。当内网私库尚未配置 TLS 时，只需设置 `registry.insecure: true`，而保持 **`image_sync.insecure: false`**——这意味着从 `registry.k8s.io`、`quay.io`、`docker.io`、`ghcr.io` 等公网仓库拉取镜像时，依然强制执行完整的公网 CA 证书链与域名校验。
4. **节点侧凭据文件 `0600` 最小权限**：
   分发至各节点的 `/etc/containerd/certs.d/<host>/hosts.toml` 强制使用 `0600` 权限位（仅 `root` 可读写），隔离节点侧非特权用户的本地读取风险。

---

## 3. Debian 13 实战：使用 `rtkk cert gen` 签发仓库 CA 与 TLS 证书

若希望将内网镜像仓库升级为**严格校验的 HTTPS（`insecure: false`）**，无需依赖复杂的外部 OpenSSL 配置文件，直接使用 `rtkk cert gen` 即可签发包含 **IP SAN** 与 **DNS SAN** 的合规 ECDSA P-256 证书。

在 Debian 13 控制机仓库根目录下执行：

```bash
mkdir -p ./pki
chmod 0700 ./pki

# 1) 签发内网镜像仓库专用根 CA
#    默认有效期为 99 年（867240h）；如需 10 年期可追加 --validity 10y
rtkk cert gen \
  --cn "rtkk-registry-ca" \
  --out-key ./pki/registry-ca.key \
  --out-cert ./pki/registry-ca.crt \
  --is-ca

# 2) 使用该根 CA 签发镜像仓库服务端 TLS 证书
#    --san 同时写入仓库 IP（10.0.0.10）与内网域名（按实际环境修改）
#    默认 99 年（自动不超过 CA 到期时间）；如需 1 年或 3 年轮转可追加 --validity 1y 或 --validity 3y
rtkk cert gen \
  --cn "10.0.0.10" \
  --out-key ./pki/registry-server.key \
  --out-cert ./pki/registry-server.crt \
  --ca-key ./pki/registry-ca.key \
  --ca-cert ./pki/registry-ca.crt \
  --san 10.0.0.10 \
  --san registry.internal

# 3) 使用 rtkk cert verify 校验服务端证书、私钥匹配关系与 CA 签名链
rtkk cert verify ./pki/registry-server.crt \
  --key ./pki/registry-server.key \
  --ca ./pki/registry-ca.crt
```

---

## 4. 配置 Harbor / CNCF Distribution 启用 HTTPS

将上一步生成的 `registry-server.crt` 与 `registry-server.key` 拷贝至镜像仓库所在服务器（示例 IP：`10.0.0.10`）的安全目录：

```bash
ssh -i ~/.ssh/rtkk_ed25519 root@10.0.0.10 "install -d -m 0700 /etc/registry/ssl"
scp -i ~/.ssh/rtkk_ed25519 ./pki/registry-server.crt ./pki/registry-server.key root@10.0.0.10:/etc/registry/ssl/
ssh -i ~/.ssh/rtkk_ed25519 root@10.0.0.10 "chmod 0600 /etc/registry/ssl/registry-server.key && chmod 0644 /etc/registry/ssl/registry-server.crt"
```

### 4.1 场景 A：Harbor v2 启用 HTTPS

在 Harbor 宿主机编辑 `harbor.yml`，启用 `https` 配置段并重新生成配置：

```yaml
hostname: 10.0.0.10

https:
  port: 443
  certificate: /etc/registry/ssl/registry-server.crt
  private_key: /etc/registry/ssl/registry-server.key
```

在 Harbor 安装目录应用变更：

```bash
./prepare
docker compose down
docker compose up -d
```

### 4.2 场景 B：CNCF Distribution（`registry:2`）启用 HTTPS

若使用轻量级 CNCF Distribution，在其 `config.yml` 的 `http.tls` 段引用证书与私钥（或通过环境变量 `REGISTRY_HTTP_TLS_CERTIFICATE` / `REGISTRY_HTTP_TLS_KEY` 注入）：

```yaml
version: 0.1
storage:
  filesystem:
    rootdirectory: /var/lib/registry
http:
  addr: :443
  tls:
    certificate: /etc/registry/ssl/registry-server.crt
    key: /etc/registry/ssl/registry-server.key
```

---

## 5. Debian 13 节点信任根 CA 与切换 `rtkk` 至严格 HTTPS

### 5.1 在 Debian 13 控制机与集群节点安装系统级信任 CA

Debian 13 (`Trixie`) 通过 `ca-certificates` 包管理操作系统信任锚。将 `registry-ca.crt`（扩展名必须为 `.crt`，内容为 PEM 格式）放入 `/usr/local/share/ca-certificates/` 并执行 `update-ca-certificates`：

```bash
# 1) 在本机（若控制机也是 Debian 13）安装信任
install -m 0644 ./pki/registry-ca.crt /usr/local/share/ca-certificates/rtkk-registry-ca.crt
update-ca-certificates

# 2) 批量分发至所有 Debian 13 Kubernetes 节点并刷新系统 CA 信任库
for ip in 10.0.0.61 10.0.0.62 10.0.0.63 10.0.0.64 10.0.0.65 10.0.0.66; do
  scp -i ~/.ssh/rtkk_ed25519 ./pki/registry-ca.crt "root@${ip}:/usr/local/share/ca-certificates/rtkk-registry-ca.crt"
  ssh -i ~/.ssh/rtkk_ed25519 "root@${ip}" "chmod 0644 /usr/local/share/ca-certificates/rtkk-registry-ca.crt && update-ca-certificates"
done
```

### 5.2 配置节点 `containerd` `certs.d` 启用严格 TLS 校验

在各 Kubernetes 节点的 `/etc/containerd/certs.d/10.0.0.10/` 目录放置根证书 `ca.crt`（`0644`）与 `hosts.toml`（`0600`），将 `skip_verify` 设为 `false`：

```toml
server = "https://10.0.0.10"

[host."https://10.0.0.10"]
  capabilities = ["pull", "resolve"]
  ca = "/etc/containerd/certs.d/10.0.0.10/ca.crt"
  skip_verify = false
  [host."https://10.0.0.10".header]
    authorization = ["Basic <BASE64_CREDENTIAL>"]
```

### 5.3 更新 `configs/inventory.yaml` 启用严格 HTTPS

完成仓库 HTTPS 改造后，在 `configs/inventory.yaml` 中显式锁定 `scheme: "https"`、指定 `ca_cert_file`，并将 `registry.insecure` 与 `image_sync.insecure` 全部设为 `false`：

```yaml
vars:
  registry:
    url: "10.0.0.10/rtkk"
    host: "10.0.0.10"
    scheme: "https"
    ca_cert_file: "./pki/registry-ca.crt"
    insecure: false
    username: "admin"
    password: "<STRONG_PASSWORD>"
    harbor_project_public: false
  image_sync:
    output_dir: "./downloads/images"
    platform: "linux/amd64"
    insecure: false
```

### 5.4 端到端连通性验证

在 Debian 13 控制机上验证无 `--insecure` 参数下的严格 TLS 握手与镜像推送：

```bash
# 1) 验证 HTTPS 证书链与 IP SAN（不加 -k，预期返回 200 或 401 JSON）
curl -sS -o /dev/null -w "%{http_code}\n" --cacert ./pki/registry-ca.crt https://10.0.0.10/v2/

# 2) 使用 rtkk 在严格 TLS 模式下推送镜像
rtkk push-images -i configs/inventory.yaml --images-file configs/images.yaml -l logs/push-images.log
```

---

## 相关文档

- [运维准备与安全建议手册](../OPERATION_GUIDE.md)
- [镜像仓库认证与 Harbor 权限配置指南](./registry-auth-and-harbor.md)
- [SSH 认证、非 Root 提权（become/sudo）与主机指纹校验指南](./ssh-and-privilege-escalation.md)
