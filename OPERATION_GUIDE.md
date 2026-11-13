# RTKK 运维准备与安全建议手册

> [!NOTE]
> **手册定位说明**：本手册为面向运维人员的规范性建议与行前提醒清单，旨在协助规避潜在环境风险，**不构成程序的强制约束**；`rtkk` 默认保留开箱即用的内网极简部署体验。运维团队可根据所在网络隔离等级、审计合规制度与运维习惯，按需采纳下述配置。

---

## 目录

- [准备阶段必做提醒清单（Pre-Flight Checklist）](#准备阶段必做提醒清单pre-flight-checklist)
- [1. 准备阶段必做：更换仓库示例密码与生成 SSH 密钥（适配 Debian 13 Trixie）](#1-准备阶段必做-更换仓库示例密码与生成-ssh-密钥适配-debian-13-trixie)
- [2. X.509 证书有效期策略说明（默认 99 年，支持按需选择 1 年期或多年期）](#2-x509-证书有效期策略说明默认-99-年支持按需选择-1-年期或多年期)
- [3. 镜像仓库认证与 Harbor 项目可见性建议](#3-镜像仓库认证与-harbor-项目可见性建议)
- [4. 内网 HTTP 镜像仓库与 TLS 加固建议](#4-内网-http-镜像仓库与-tls-加固建议)
- [5. SSH Root 直连与非 Root `sudo`/`become` 提权建议](#5-ssh-root-直连与非-root-sudobecome-提权建议)

---

## 准备阶段必做提醒清单（Pre-Flight Checklist）

在面向正式环境或长期运行集群执行 `rtkk` 前，建议逐项核对以下清单：

| 序号 | 检查项 | 风险等级 | 默认体验行为 | 运维准备动作 |
|------|--------|----------|--------------|--------------|
| **1** | **替换示例口令与配置 SSH 密钥** | **必做** | 示例 `configs/inventory.yaml` 含占位口令 `change-me` 以便快速验证 | 生成高熵随机密码与 Ed25519 密钥对，替换所有 `change-me` 占位值 |
| **2** | **确认集群 X.509 证书有效期策略** | 建议核对 | 默认签发 `867240h`（99 年）长效免维护证书 | 内网长期免维护集群保持默认即可；若有年度轮转合规要求，调整 `cert_validity` |
| **3** | **确认镜像仓库账号权限与项目可见性** | 建议核对 | 支持 Basic/Bearer 认证，Harbor 默认自动创建公开项目（`harbor_project_public: true`） | 多租户或敏感环境建议设为私有项目（`false`）并改用 Harbor Robot Account |
| **4** | **评估内网镜像仓库传输层 TLS 策略** | 建议核对 | 支持 `scheme: "auto"`（HTTPS 探测后回退 HTTP）与 SHA256 强校验 | 跨网段或共享内网环境建议用 `rtkk cert gen` 签发私库证书并启用严格 HTTPS |
| **5** | **评估节点 SSH 登录账号与指纹校验策略** | 建议核对 | 默认 `root` 直连、跳过主机指纹校验以便实验环境重装 | 生产环境建议启用密钥登录、按需采用非 `root` `become`/`sudo` 及 `accept-new`/`strict` 指纹校验 |

---

## 1. [准备阶段必做] 更换仓库示例密码与生成 SSH 密钥（适配 Debian 13 Trixie）

仓库内 `configs/inventory.yaml` 提供的示例凭据（如 `change-me`）仅用于本地功能验证与配置结构演示。**在任何正式或联网可触达环境执行部署前，请务必替换所有示例密码并配置专用 SSH 密钥。**

以下操作命令均针对 **Debian 13 (`Trixie`)** 环境验证，可直接复制执行。

### 1.1 安装基础运维工具（最小化安装的 Debian 13 控制机）

若部署控制机采用 Debian 13 最小化安装（Minimal Install），可先补齐密钥管理、随机口令生成及 JSON 处理工具：

```bash
apt update && apt install -y openssh-client openssl apache2-utils jq
```

### 1.2 生成高强度随机密码

使用 OpenSSL CSPRNG 生成 24 字节（32 字符 Base64）高熵随机口令，用作节点应急口令或镜像仓库强口令：

```bash
openssl rand -base64 24
```

### 1.3 生成专用 Ed25519 SSH 密钥对并分发至集群节点

在控制机生成专用 Ed25519 密钥对（`-a 100` 提升 KDF 迭代轮数），并将公钥写入全部 Debian 13 目标节点：

```bash
# 1) 生成 Ed25519 部署密钥对
ssh-keygen -t ed25519 -a 100 -f ~/.ssh/rtkk_ed25519 -N "" -C "rtkk-deploy@$(hostname)"
chmod 600 ~/.ssh/rtkk_ed25519

# 2) 单节点分发示例
ssh-copy-id -i ~/.ssh/rtkk_ed25519.pub root@10.0.0.61

# 3) 多节点批量分发示例（按实际节点 IP 替换 10.0.0.x）
for ip in 10.0.0.61 10.0.0.62 10.0.0.63 10.0.0.64 10.0.0.65 10.0.0.66; do
  ssh-copy-id -i ~/.ssh/rtkk_ed25519.pub "root@${ip}"
done

# 4) 验证免密登录
ssh -i ~/.ssh/rtkk_ed25519 root@10.0.0.61 "cat /etc/debian_version && uname -r"
```

### 1.4 批量轮换目标节点 `root` 口令与更新 Inventory SSH 配置

完成公钥分发并确认免密登录可用后，可批量轮换各目标节点的 `root` 系统密码，并在 `configs/inventory.yaml` 中切换为密钥认证：

```bash
# 通过已分发的密钥通道批量更新各节点 root 密码（请妥善保存生成的密码至密码库）
NEW_ROOT_PASS="$(openssl rand -base64 24)"
printf 'Generated root password: %s\n' "${NEW_ROOT_PASS}"

for ip in 10.0.0.61 10.0.0.62 10.0.0.63 10.0.0.64 10.0.0.65 10.0.0.66; do
  ssh -i ~/.ssh/rtkk_ed25519 "root@${ip}" "printf 'root:%s\n' '${NEW_ROOT_PASS}' | chpasswd"
done
```

随后编辑 `configs/inventory.yaml`，配置显式私钥路径并移除明文 `ansible_ssh_pass`：

```yaml
vars:
  ansible_user: "root"
  ansible_ssh_private_key_file: "~/.ssh/rtkk_ed25519"
  # 使用密钥登录时无需保留明文密码：
  # ansible_ssh_pass: "<STRONG_PASSWORD>"
```

### 1.5 轮换 Harbor / 镜像仓库密码与 `auth_basic` 自动派生说明

将私有镜像仓库（Harbor 或 CNCF Distribution）管理员口令修改为强密码（或创建专用 Robot 账号），并同步更新 `configs/inventory.yaml` 中的 `registry.username` 与 `registry.password`：

```yaml
vars:
  registry:
    url: "10.0.0.10/rtkk"
    host: "10.0.0.10"
    username: "admin"
    password: "<STRONG_PASSWORD>"
```

- **自动派生机制**：`rtkk` 加载 Inventory 时，若未显式填写 `registry.auth_basic`（或值为空），会自动根据 `registry.username` 与 `registry.password` 计算标准 `base64(username:password)` 并注入 `registry.auth_basic` 供 `containerd` `hosts.toml` 模板渲染使用，避免手工维护两处凭据导致不一致。
- **手工核算验证命令（Debian 13）**：如需在终端核对或手工生成单行无换行 Base64 凭据（`-w0` 禁止 76 列折行），可执行：

```bash
printf '%s:%s' 'admin' '<STRONG_PASSWORD>' | base64 -w0 && echo
```

---

## 2. X.509 证书有效期策略说明（默认 99 年，支持按需选择 1 年期或多年期）

### 2.1 默认 99 年（`867240h`）设计初衷与底层工程保障

在内网离线与边缘生产场景中，Kubernetes 控制面及 etcd 证书一年期满若未及时轮转，极易引发集群不可用故障。为此，`rtkk` 默认将集群 CA 与业务组件证书有效期统一设定为 **99 年（`867240h`，即 `99 * 365 * 24h`）**，提供长期免维护的运行体验。

针对超长有效期证书在标准协议与云原生组件中的常见兼容性边界，`rtkk` 在底层做了完整工程处理：

1. **RFC 5280 `GeneralizedTime` 规范编码**：X.509 规范规定 2050 年及以后的时间戳必须采用四位年份的 ASN.1 `GeneralizedTime`（`YYYYMMDDHHMMSSZ`），不能使用两位年份的 `UTCTime`。`rtkk` 签发跨 2050 年的长效证书时自动采用合规 `GeneralizedTime` 编码，杜绝年份截断与解析拒绝。
2. **Go `time.Duration` 溢出上限保护**：Kubernetes、etcd、containerd、Cilium 等核心组件均由 Go 编写，其标准库 `time.Duration` 为有符号 64 位纳秒整数（最大上限约 `292.47` 年，即约 `2562047h`）。`867240h`（99 年）处于安全区间内，且 `rtkk` 对有效期参数实施安全边界校验，防止配置超大值引发 Go 组件整数溢出。
3. **叶证书到期时间自动截齐（Leaf `not_after <= ca.not_after`）**：使用已有根 CA 签发业务叶证书时，若目标有效期的计算截止时刻晚于 CA 证书自身的 `not_after`（例如 CA 已运行若干天后再扩容新节点），`rtkk` 会自动将叶证书的 `not_after` 截齐至不超过 `ca.not_after`，保证证书链生命周期逻辑严密。
4. **ECDSA P-256 与 RFC 5480 `KeyUsage` 严格合规**：全链路采用 ECDSA P-256 椭圆曲线密钥；CA 证书严格限定 `KeyCertSign | CrlSign` 与 `BasicConstraints(CA=true)`，服务端/客户端叶证书精确设置 `DigitalSignature` 及对应的 `ServerAuth` / `ClientAuth` 扩展密钥用法（EKU），符合现代 Go `crypto/x509` 与 OpenSSL 3（Debian 13 默认搭载）的严格校检要求。
5. **`kubeadm` `v1beta4` 有效期同步对齐**：`configs/templates/` 中的 `kubeadm` 配置采用 `kubeadm.k8s.io/v1beta4` API，将 `caCertificateValidityPeriod` 与控制面组件 `certificateValidityPeriod` 与集群证书有效期同步对齐，确保 `kubeadm init` 签发的控制面证书遵循同一生命周期策略。
6. **Cilium / Hubble 网络平面证书协调**：Cilium 与 Hubble 组件按部署模板采用协调一致的动态证书机制，避免控制面长效而网络数据面证书短期失效。

### 2.2 按需切换为 1 年期或多年期轮转策略

若所在组织的安全合规制度要求执行 **1 年期（`8760h` / `1y`）** 或 **多年期（如 `3y`、`5y`、`10y`）** 定期证书轮转，无需修改任何代码或模板，可通过以下任意方式指定：

1. **在 `configs/inventory.yaml` 中全局配置**：

   ```yaml
   vars:
     # 支持小时数（如 "8760h"）或年/天单位（如 "1y"、"3y"、"10y"、"365d"）；默认 "867240h"（99 年）
     cert_validity: "8760h"
   ```

2. **通过命令行 `-s` 临时覆盖**：

   ```bash
   rtkk run configs/deploy.yaml -i configs/inventory.yaml -s cert_validity=1y
   ```

3. **使用 `rtkk cert gen` 与 `rtkk cert verify` 独立签发和巡检证书**：

   ```bash
   # 独立签发 10 年期根 CA 与 1 年期业务证书
   rtkk cert gen --cn "kubernetes-ca" --out-key ./pki/ca.key --out-cert ./pki/ca.crt --is-ca --validity 10y
   rtkk cert gen --cn "kube-apiserver" \
     --out-key ./pki/apiserver.key \
     --out-cert ./pki/apiserver.crt \
     --ca-key ./pki/ca.key \
     --ca-cert ./pki/ca.crt \
     --san kubernetes --san kubernetes.default --san 10.0.0.68 --san 10.233.64.1 \
     --validity 1y

   # 校验叶证书与私钥匹配性、CA 签名链完整性及剩余有效期
   rtkk cert verify ./pki/apiserver.crt --key ./pki/apiserver.key --ca ./pki/ca.crt
   ```

---

## 3. 镜像仓库认证与 Harbor 项目可见性建议

- **默认开箱即用体验**：`rtkk` 内置完整的 OCI Distribution v2 认证客户端，除支持标准 HTTP `Basic` 认证外，还原生支持 `401 Unauthorized` + `WWW-Authenticate: Bearer` 令牌自动交换（覆盖拉取与推送全链路）。面向 Harbor v2 推送镜像时，默认自动调用 API 创建公开项目（`harbor_project_public: true`），减少节点侧拉取排障成本。
- **进阶加固建议**：在多团队共享 Harbor 实例或包含专有业务镜像的环境中，建议将 `registry.harbor_project_public` 设为 `false`（私有项目），使用最小权限的 **Harbor Robot Account** 替代全局 `admin` 账号，并确保各节点 `/etc/containerd/certs.d/<host>/hosts.toml` 保持 `0600` 权限。
- **详细操作指南**：参阅 **[镜像仓库认证与 Harbor 权限配置指南](./docs/registry-auth-and-harbor.md)**。

---

## 4. 内网 HTTP 镜像仓库与 TLS 加固建议

- **默认开箱即用体验**：`rtkk` 支持 `registry.scheme: "auto"`（优先探测 HTTPS，当 `registry.insecure: true` 时支持回退至内网 HTTP）或显式 `"http"`，方便尚未部署内部 PKI 的纯内网环境快速完成镜像同步。即使在 HTTP 便利模式下，`rtkk` 仍强制执行 **Manifest 与 Blob 双重 SHA256 摘要校验**、**未收到 `401` 挑战前绝不盲发凭据**，并将公网上游 TLS 校验（`image_sync.insecure: false`）与内网私库配置严格隔离。
- **进阶加固建议**：在跨机房、共享办公网或具备合规审计要求的环境中，建议使用 `rtkk cert gen` 快速签发内网镜像仓库专用 CA 与 TLS 证书，在 Debian 13 节点通过 `update-ca-certificates` 分发信任，并将仓库配置切换为 `scheme: "https"`、`ca_cert_file: "./pki/registry-ca.crt"` 与 `insecure: false`。
- **详细操作指南**：参阅 **[内网镜像仓库 TLS 加固与 HTTP 风险缓解指南](./docs/registry-tls-and-http.md)**。

---

## 5. SSH Root 直连与非 Root `sudo`/`become` 提权建议

- **默认开箱即用体验**：示例配置默认通过 Debian 13 的 `root` 账号建立 SSH 会话（推荐配合 Ed25519 密钥与 `sshd` 默认的 `PermitRootLogin prohibit-password`），直接完成内核参数、系统服务与 Kubernetes 组件安装；默认关闭 SSH 主机公钥强校验以便频繁重装测试机。
- **进阶加固建议**：针对禁止 `root` 远程直接登录（`PermitRootLogin no`）的堡垒机或生产基线环境，`rtkk` 原生支持普通运维账号（如 `deploy`）登录并通过 `become` / `sudo`（免密 `NOPASSWD` 或 `ansible_become_pass` 密码提权）执行特权操作，同时支持 `ansible_ssh_host_key_checking: "accept-new"` 或 `"strict"`（配合 `ssh-keyscan` 预置 `~/.ssh/known_hosts`）防范中间人攻击。
- **详细操作指南**：参阅 **[SSH 认证、非 Root 提权（become/sudo）与主机指纹校验指南](./docs/ssh-and-privilege-escalation.md)**。
