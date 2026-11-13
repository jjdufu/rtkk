# RTKK

RTKK 是用 **Rust** 编写的轻量级集群编排与配置管理工具（免 Python 的 Ansible 风格执行引擎）。它通过 SSH 或本机执行器在多节点并发运行 Playbook，并内置 **纯 Rust OCI Distribution v2 客户端**与 **X.509 PKI 签发引擎**，无需在控制机或目标节点预装 Python、Docker、crane 或 skopeo 即可完成二进制分发、镜像同步、证书签发与离线 Kubernetes 高可用集群部署。

- **版本**：`0.0.1` <!-- x-release-please-version -->（`Cargo.toml` / `rtkk --version`）
- **Rust 工具链**：`edition = "2024"`，`rust-version = "1.100.0"`
- **安全与静态检查门禁**：Workspace 级 `unsafe_code = "forbid"`；`rustc`（`warnings`、`missing_docs`）与 `clippy`（`all`、`pedantic`、`missing_docs`、`missing_errors_doc`、`missing_panics_doc`、`allow_attributes`、`unwrap_used`、`expect_used`、`panic`、`as_conversions`、`indexing_slicing`）全部设为 **deny**，禁止通过 `#[allow]` 压制告警

仓库随附一套面向 **Debian 13 (`Trixie`)** 的离线高可用 Kubernetes 部署配置（`configs/`）与运维准备手册（[OPERATION_GUIDE.md](./OPERATION_GUIDE.md)），覆盖离线制品拉取、`/etc/hosts` 编排、etcd 集群、kube-vip 控制面高可用、Cilium、OpenEBS LocalPV、NodeLocalDNS 部署及状态备份。

---

## 目录

- [1. 核心特性](#1-核心特性)
- [2. 仓库目录结构](#2-仓库目录结构)
- [3. 架构设计](#3-架构设计)
- [4. 内置任务模块](#4-内置任务模块)
- [5. 模板辅助函数](#5-模板辅助函数)
- [6. 命令行参考](#6-命令行参考)
- [7. 配置文件与离线 K8s 编排](#7-配置文件与离线-k8s-编排)
- [8. 典型工作流（离线 K8s 部署）](#8-典型工作流离线-k8s-部署)
- [9. 运维准备与安全指南](#9-运维准备与安全指南)
- [10. 编译、测试与质量门禁](#10-编译测试与质量门禁)
- [11. 环境变量与日志](#11-环境变量与日志)
- [12. 核心运行时依赖](#12-核心运行时依赖)

---

## 1. 核心特性

1. **受控节点零运行时依赖**：目标主机仅需标准 OpenSSH（SSH / SFTP）与基础系统工具，无需安装 Python 解释器或容器守护进程。
2. **并发 Playbook 引擎**：支持 `serial` 滚动分批、`forks` 连接池并发控制、`when` / `failed_when` / `changed_when` 严格条件求值、`loop` / `with_items` 循环聚合、`block`·`rescue`·`always` 异常恢复、`register` 变量捕获、指数退避重试、`tags` 过滤、任务超时以及 `roles/` 目录组织。
3. **确定性层级 Inventory**：支持扁平 `vars`+`hosts`+`roles` 与嵌套 `all/children` 两种拓扑结构；主机与组成员保持确定性顺序；`localhost` 作为独立隐式本机目标且排除在 `all` 组之外；自动注入集群拓扑变量与运行时上下文变量。
4. **连接池、提权与主机指纹校验**：自动识别本机（`LocalConnector`）与远端 SSH（`SshConnector`）；支持密钥/密码有序认证、`become`/`sudo` 非 `root` 提权（`PrivilegeConfig`）以及三档 `known_hosts` 主机公钥校验策略（`HostKeyPolicy`）。
5. **纯 Rust OCI 镜像同步链路**：原生实现 `Basic` 与 `401 WWW-Authenticate: Bearer` 令牌自动交换（覆盖拉取与推送）、Manifest 与 Blob 全链路 SHA256 摘要校验、`rtkk-media.json` 媒体类型保真、Harbor v2 项目自动创建及 `pull-images` / `push-images` / `sync-images` 编排。
6. **内置 ECDSA P-256 PKI 引擎**：支持通过 Playbook `gen_cert` 任务或 `rtkk cert gen` / `rtkk cert verify` 子命令签发与校验 X.509 证书；默认 99 年（`867240h`）免维护有效期，支持通过 `cert_validity` 按需切换为 1 年期或多年期，内置 RFC 5280 `GeneralizedTime` 编码与叶证书有效期自动截齐。
7. **严格模板引擎**：基于 Go 风格 `gtmpl` 语法，内置 40 个面向基础设施编排的辅助函数（含 `b64enc`、`toYaml`、`generateIPsFromCIDR`、`semverCompare` 等），对模板语法及条件求值错误执行快速失败（Fail-Fast）诊断。

---

## 2. 仓库目录结构

```text
.
├── OPERATION_GUIDE.md      # 运维准备与安全建议手册（行前检查清单与 Debian 13 操作命令）
├── docs/                   # 专题技术与加固指南
│   ├── registry-auth-and-harbor.md
│   ├── registry-tls-and-http.md
│   └── ssh-and-privilege-escalation.md
├── src/                    # 核心库与 CLI 源码
│   ├── main.rs             # 二进制入口 -> rtkk::cli::run_cli
│   ├── lib.rs              # 库根模块导出与全局 lint 契约
│   ├── cli.rs              # Clap 命令行定义、Inventory 加载与子命令分发
│   ├── error.rs            # 统一结构化错误类型 RtkkError
│   ├── prep.rs             # prep hosts 主机名映射编排
│   ├── cert.rs             # X.509 ECDSA P-256 证书签发、链校验与有效期控制
│   ├── utils.rs            # 通用路径、权限与工具函数
│   ├── connector/          # LocalConnector / SshConnector、连接池、become 提权、HostKeyPolicy
│   ├── inventory/          # Inventory 模型、组依赖图解析、确定性排序与变量深度合并
│   ├── playbook/           # Playbook 解析、Roles 展开、执行引擎、动态变量注入
│   ├── tasks/              # 内置任务模块分发与具体实现
│   ├── template/           # gtmpl 严格渲染封装与辅助函数注册（helpers/*）
│   ├── registry/           # OCI V2 拉取、docker-legacy tar 读写、推送、Bearer 认证、Harbor API
│   └── logging/            # 结构化文件日志与 TTY 实时进度条
├── configs/                # 离线高可用 Kubernetes 部署配置与模板
│   ├── inventory.yaml      # 集群主机、组件版本、私库配置、网段等统一变量源
│   ├── images.yaml         # 集群镜像 source/target 模板化清单
│   ├── download.yaml       # 二进制下载与镜像拉取/推送编排 Playbook
│   ├── deploy.yaml         # 离线高可用 K8s 全流程部署 Playbook（Play 0–8）
│   ├── cleanup.yaml        # 集群组件卸载与状态清理 Playbook
│   └── templates/          # kubeadm / containerd / etcd / cilium / openebs / nodelocaldns 模板
└── test/                   # 独立集成测试与仿真验证套件
```

---

## 3. 架构设计

### 3.1 连接层（`src/connector/`）

- **`LocalConnector`**：当目标为 `localhost`、匹配本机网卡 IP，或显式声明 `ansible_connection: local` / `connector_type: local` 时自动启用。
  - `command`：经 `shell-words` 拆分参数后直接调用系统 `exec`（不经过 `/bin/sh`）。
  - `shell`：通过 `/bin/sh -c` 执行，支持管道与环境变量展开。
- **`SshConnector`**（基于 `async-ssh2-lite` / `ssh2-tokio`）：
  - **认证顺序**：内联私钥内容 → 显式 `ansible_ssh_private_key_file`（配置路径存在但认证失败时默认立即报错；设 `ansible_ssh_private_key_fallback_password: true` 时允许回退密码）→ 默认 `$HOME/.ssh/id_ed25519` / `id_ecdsa` / `id_rsa` → `ansible_ssh_pass` 密码认证。
  - **非 `root` 提权（`PrivilegeConfig`）**：支持通过 Inventory 变量（`ansible_become`、`ansible_become_user`、`ansible_become_pass`）以及 Play/Task 级 `become` / `become_user` 开启 `sudo` 提权，覆盖命令执行与受控文件上传/拉取。详见 [docs/ssh-and-privilege-escalation.md](./docs/ssh-and-privilege-escalation.md)。
  - **SSH 主机指纹校验（`HostKeyPolicy`）**：通过 `ansible_ssh_host_key_checking` 支持 `"off"`/`false`（默认跳过）、`"accept-new"`（首次连接信任并写入 `known_hosts`）与 `"strict"`/`true`（强制匹配预置 `known_hosts`，默认路径 `$HOME/.ssh/known_hosts`，可通过 `ansible_ssh_known_hosts_file` 指定）。
  - **连接池与流控**：引擎按主机维护 `PooledConnector`（`OnceCell` 并发建连去重），遇到瞬断传输错误可自动重建会话重试一次；`fetch` 采用 64 KiB 分块流式落盘；单次命令标准输出/错误内存上限受 `RTKK_MAX_CMD_OUTPUT_BYTES` 保护（默认 **64 MiB**）。

### 3.2 Inventory 与注入变量（`src/inventory/`）

支持两种 YAML 组织结构，解析过程对主机与组成员均保持**确定性排序**：

- **扁平结构**（如 `configs/inventory.yaml`）：顶层 `vars` + `hosts`（IP 字符串自动注入 `ip` 字段，或传对象）+ `roles`（支持角色嵌套展开）。
- **嵌套结构**：顶层 `all` / `children` / 组内 `hosts` 与 `vars`（内置有向图循环依赖检测），按 `all.vars` → 祖先组拓扑序 → 主机变量执行 `deep_merge`。
- **`localhost` 隔离规则**：`localhost` 作为内置隐式本机目标可直接用于 `hosts: localhost` 或 `delegate_to: localhost`，且**不包含在 `all` 组内**，确保面向 `hosts: all` 的集群操作不会误作用于控制机本身。

加载 Inventory 时自动派生并注入以下动态集群变量与运行时变量：

| 变量名 | 说明 |
|--------|------|
| `groups` | 组名 → 有序主机名列表映射对象 |
| `first_control_plane` | `kube_control_plane` 组的首个主机名（供初始化、证书分发与插件部署条件引用） |
| `etcd_initial_cluster` / `etcd_endpoints_str` / `etcd_endpoints` | 根据 `etcd` 组（未单独定义时取 `kube_control_plane` 组）推导的 etcd 集群端点集合 |
| `inventory_file` / `inventory_dir` | 当前 `-i` Inventory 文件的解析路径及其所在父目录 |
| `rtkk_exe` | 当前正在运行的 `rtkk` 可执行文件路径（供 Playbook 自举调用自身子命令） |
| `registry.auth_basic` | 当配置了 `registry.username` 与 `registry.password` 且 `registry.auth_basic` 缺省或为空时，自动派生 `base64(username:password)` |

### 3.3 Playbook 执行引擎（`src/playbook/`）

- **目标选择器**：支持组名、单主机名、确定性下标（如 `kube_control_plane[0]`）以及随机单节点（如 `etcd|random`）。
- **批次与并发**：`serial` 支持固定数量、百分比字符串（如 `"50%"`）或分阶段数组；每批内部受 `forks` 并发度控制。
- **条件与状态判定**：
  - `when`：Go 模板布尔表达式（数组形式按逻辑 `AND` 求值），表达式渲染错误立即中止报错。
  - `failed_when` / `changed_when`：注入本次执行的 `stdout`、`stderr`、`rc` 上下文，支持精确自定义成败与变更判定。
- **循环（`loop` / `with_items`）与 `break_on_error`**：支持 YAML 数组字面量，也支持直接引用上下文中的数组变量（如 `with_items: "{{ .public_registries }}"`）。
- **控制流与容错**：支持任务级/Play 级 `ignore_errors`、`become`/`become_user`、`run_once`、`delegate_to`、`retries`+`delay`（指数退避加随机抖动）、`tags` 过滤、`timeout` 超时控制，以及 `block` / `rescue` / `always` 结构化异常处理。
- **变量优先级（低 → 高）**：
  1. Role `defaults/main.yaml`（仅填充缺失键）
  2. Inventory 组变量与主机变量
  3. CLI `-s key=val` 额外变量
  4. Play `vars`
  5. Role 调用内联参数（`roles: [{ role: app, port: 8080 }]`）

### 3.4 模板引擎（`src/template/`）

基于 `gtmpl` 为 Playbook 字段、`images.yaml` 清单以及 `.gtmpl` 配置文件提供统一的模板渲染能力：

- **严格错误诊断**：所有包含 `{{ ... }}` 的模板字符串在语法错误、未定义函数或求值异常时均立即返回明确错误，杜绝把未展开的模板原文静默写入目标系统配置文件。
- **类型保持**：字符串字段渲染结果始终保持字符串类型，避免意外的类型隐式转换。
- **模板路径查找**：`template` 模块的 `src` 若相对当前路径不存在，自动查找 `{inventory_dir}/templates/{文件名}`。

### 3.5 X.509 证书引擎（`src/cert.rs`）

- 采用 **ECDSA P-256** 签发根 CA 与服务端/客户端叶证书；私钥落盘权限 `0600`，证书落盘权限 `0644`，并对同一 CA 路径加并发互斥锁。
- **有效期控制**：默认有效期为 **99 年（`867240h`）**；可通过全局 `vars.cert_validity`、任务参数 `date` / `date_hours` 或 CLI `--validity` 指定自定义时长（如 `"1y"`、`"3y"`、`"10y"`、`"8760h"`）。
- **标准合规性**：2050 年及以后到期时间自动采用 RFC 5280 `GeneralizedTime` 编码；严格设置 RFC 5480 `KeyUsage` 与 `ExtendedKeyUsage`；叶证书到期时间自动截齐至不超过签发 CA 的 `not_after`；内置 Go `time.Duration` 安全上限保护。

### 3.6 OCI Registry 客户端（`src/registry/`）

| 模块文件 | 核心职责 |
|----------|----------|
| `pull.rs` | 镜像引用解析、HTTPS/HTTP 协议协商、Basic/Bearer 令牌认证、多架构 Manifest List 平台筛选、Blob 并发下载与 Manifest+Blob SHA256 校验 |
| `legacy_write.rs` | 生成标准 docker-legacy tar 归档、临时文件原子重命名、幂等跳过检查 |
| `media.rs` | 读写 `rtkk-media.json` 保存原始 OCI/Docker Layer MediaType；外部无元数据归档按 gzip 魔数推断 |
| `manifest.rs` | OCI Image Manifest / Index 与 Docker Manifest V2 Schema 2 模型定义 |
| `image_list.rs` | 解析 `image_sync` 参数、渲染 `images.yaml` 清单、管理按域名隔离的 `source_auths` |
| `tar.rs` | 读取本地 tar 归档、`RepoTags` 目标前缀匹配校验、并发上传调度 |
| `client.rs` | OCI Distribution v2 推送（Blob `HEAD` 判重、流式上传、Manifest `PUT`）、Bearer Push 令牌协商、Harbor v2 项目自动创建 |
| `mod.rs` | `pull-images`、`push-images`（单包/清单批量）与 `sync-images` 顶层编排 |

**核心机制要点**：

- **认证**：完整支持 HTTP `Basic` 与 `401 WWW-Authenticate: Bearer realm="...",service="...",scope="..."` 自动令牌交换（拉取与推送均支持）；未收到 `401` 挑战前不向未知端点预发凭据。
- **Harbor 项目可见性**：推送至 Harbor v2 时自动调用 `POST /api/v2.0/projects` 确保目标项目存在；通过 `registry.harbor_project_public` 控制新建项目为公开（默认 `true`）或私有（`false`）。
- **传输与 TLS**：支持 `registry.scheme`（`"auto"` | `"https"` | `"http"`）、自定义根证书 `registry.ca_cert_file` 以及相互隔离的 `image_sync.insecure`（上游源）与 `registry.insecure`（目标私库）。
- **完整性校验**：全链路校验 **Manifest 原文 SHA256 摘要**与所有 **Config / Layer Blob SHA256 摘要**。

---

## 4. 内置任务模块

任务分发入口位于 `src/tasks/mod.rs`，每个原子任务声明唯一模块动作（`block` 组合任务除外）：

| 模块 | 主要参数 | 功能说明 |
|------|----------|----------|
| **`gen_cert`** | `cn`, `out_key`, `out_cert`, `root_key`, `root_cert`, `sans`, `is_ca`, `date` / `date_hours`, `policy` | 本地签发/校验 ECDSA P-256 证书后上传至目标主机；`policy` 支持 `always`（默认）、`ifnotpresent` / `if_not_present`、`none` |
| **`template`** | `src`, `dest`, `mode` | 渲染 `.gtmpl` 模板；远端文件 SHA256 与权限位一致时幂等跳过 |
| **`copy`** | `src` 或 `content`, `dest`, `mode` | 自动创建父目录，经临时文件原子替换；内容 SHA256 与权限位一致时幂等跳过 |
| **`fetch`** | `src`, `dest` | 将远端文件按 64 KiB 分块流式拉取至控制机 |
| **`http_get_file`** | `url`, `dest`, `sha256?` | 控制机流式 HTTP/HTTPS 下载，支持可选 SHA256 强校验与原子落盘 |
| **`command` / `shell`** | 命令字符串或参数对象 | 远端或本机命令执行（`command` 直调参数，`shell` 经 `/bin/sh -c`） |
| **`systemd`** | `name`, `state`, `enabled`, `daemon_reload` | 管理系统服务启停、重载与开机自启，具备状态幂等检测 |
| **`sysctl`** | `name`, `value`, `sysctl_file` | 双重核验内核运行时参数与持久化配置文件，幂等更新 |
| **`modprobe`** | `name`, `state` | 加载或卸载内核模块，模块名白名单字符校验，`present` 时持久化至 `/etc/modules-load.d/` |

### `register` 变量结构

- **单次任务执行**：

  ```json
  {
    "stdout": "...",
    "stderr": "...",
    "rc": 0,
    "changed": true,
    "failed": false,
    "error": null
  }
  ```

- **循环任务（`loop` / `with_items`）聚合结果**：
  循环任务捕获的 `register` 变量为统一聚合对象，既保留顶层汇总状态字段（便于后续 `when` / `failed_when` 直接判断），也在 `results` 数组中完整保留每一轮迭代明细：

  ```json
  {
    "results": [
      { "item": "...", "stdout": "...", "stderr": "...", "rc": 0, "changed": true, "failed": false, "error": null }
    ],
    "stdout": "...",
    "stderr": "...",
    "rc": 0,
    "changed": true,
    "failed": false
  }
  ```

- 设置 `register_type: json` 或 `yaml` 时自动解析 `stdout` 为结构化对象；设置 `set_global: true` 或 `run_once: true` 时同步写入全局命名空间，可通过 `{{ .global.<name> }}` 跨主机引用。

---

## 5. 模板辅助函数

注册于 `src/template/helpers/`，共 40 个内置函数：

| 分类 | 函数列表 |
|------|----------|
| **编码与序列化** | `b64enc`（标准 Base64 编码）, `toJson`, `toYaml`, `fromYaml` |
| **字符串处理** | `trimPrefix`, `trimSuffix`, `indent`, `unquote`, `splitList`, `join`, `cat`, `regexMatch`, `regexReplaceAll` |
| **集合与条件逻辑** | `has`, `append`, `slice`, `first`, `index`, `subtractList`, `default`, `ternary`, `eq`, `ne`, `empty`, `not`, `getStringSlice`, `toLowerByteUnit`, `mapToNamedStringArgs` |
| **数值运算** | `sub`, `int`, `len`, `pow`, `randInt` |
| **网络与地址计算** | `ipFamily`, `ipInCIDR`, `generateIPsFromCIDR` |
| **系统与嵌套渲染** | `fileExists`, `semverCompare`, `tpl`, `include` |

---

## 6. 命令行参考

全局用法：

```bash
rtkk [-l|--log-file <PATH>] <COMMAND>
rtkk --version
rtkk --help
```

### 6.1 `run` — 执行 Playbook

```bash
rtkk run <playbook_path> \
  -i <inventory> \
  [-s key=val ...] \
  [-f <forks>] \
  [--tags tag1,tag2] \
  [--task-timeout <SECS>] \
  [-l logs/run.log]
```

| 参数 | 说明 |
|------|------|
| `-i, --inventory` | Inventory 配置文件路径（必填） |
| `-s, --set` | 注入额外键值变量，覆盖主机与组上下文（可多次指定） |
| `-f, --forks` | 最大并发主机数；缺省取 Inventory `vars.forks`，未配置时默认 `50` |
| `--tags` | 仅执行带有匹配标签的任务（逗号分隔）；省略时执行全部任务 |
| `--task-timeout` | 全局单任务超时秒数；省略或 `0` 表示不启用全局超时限制 |

### 6.2 `prep hosts` — 编排集群 `/etc/hosts`

```bash
rtkk prep hosts -i <inventory> [--best-effort] [-l logs/prep-hosts.log]
```

并发采集各受管节点的操作系统 `hostname`，生成幂等的 `# RTKK HOSTS START` / `# RTKK HOSTS END` 映射块写入各节点 `/etc/hosts`（将本机名绑定至 `127.0.0.1` 并写入全集群 IP 与主机名映射）。默认任一节点不可达时立即报错退出；传入 `--best-effort` 允许跳过失败节点继续处理其余主机。

### 6.3 `cert gen` — 独立签发 X.509 证书

```bash
# 签发根 CA 证书（默认有效期 867240h 即 99 年；可通过 --validity 10y 等覆盖）
rtkk cert gen \
  --cn "rtkk-ca" \
  --out-key ./pki/ca.key \
  --out-cert ./pki/ca.crt \
  --is-ca \
  [--validity 867240h] \
  [--policy always|ifnotpresent|none]

# 使用根 CA 签发带 SAN 的服务端/客户端证书
rtkk cert gen \
  --cn "kube-apiserver" \
  --out-key ./pki/apiserver.key \
  --out-cert ./pki/apiserver.crt \
  --ca-key ./pki/ca.key \
  --ca-cert ./pki/ca.crt \
  --san kubernetes \
  --san 10.0.0.68 \
  [--validity 1y]
```

### 6.4 `cert verify` — 校验 X.509 证书、私钥与信任链

```bash
rtkk cert verify <cert_path> [--key <key_path>] [--ca <ca_cert_path>]
```

离线检查目标证书的时间有效性（`not_before` / `not_after`）、公钥与私钥（`--key`）配对一致性，以及是否由指定根 CA（`--ca`）合法签发。

### 6.5 `pull-images` — 拉取上游镜像并留存 tar 归档

```bash
rtkk pull-images \
  -i <inventory> \
  --images-file configs/images.yaml \
  [--output-dir <DIR>] \
  [--platform linux/amd64] \
  [--force] \
  [--insecure] \
  [-l logs/pull-images.log]
```

渲染 `images.yaml` 中的 `name` / `source` / `target` 条目，并发下载目标平台镜像层并校验 Manifest 与 Blob SHA256 摘要，输出为 `{output_dir}/{name}.tar`（内置目标 `RepoTags` 与 `rtkk-media.json`）。若本地归档已存在且校验契约完整匹配则幂等跳过；传入 `--force` 强制重新拉取覆盖；空清单直接报错拒绝执行。

### 6.6 `push-images` — 推送镜像至私有仓库（单包与清单批量互斥）

```bash
# 模式 A：单 tar 归档推送（排障或单包补推）
rtkk push-images <tar_path> -i <inventory> [-l logs/push-images.log]

# 模式 B：按清单批量严格推送（生产标准流程）
rtkk push-images \
  -i <inventory> \
  --images-file configs/images.yaml \
  [--output-dir <DIR>] \
  [-l logs/push-images.log]
```

- **单包模式**：推送单个归档中匹配 `registry.url` 前缀的标签；无匹配标签时按 `registry.push_empty_policy`（`warn_ok` 默认告警成功，或 `fail` 报错）处理。
- **清单批量模式**：严格以 `images.yaml` 为权威来源（忽略目录内不在清单中的残留文件），推送前对每个 `{name}.tar` 执行存在性、`RepoTags` 精确匹配及归档可读性预检，任一失败立即中止。

### 6.7 `sync-images` — 一体化拉取并批量推送私库

```bash
rtkk sync-images \
  -i <inventory> \
  --images-file configs/images.yaml \
  [--output-dir <DIR>] \
  [--platform linux/amd64] \
  [--force] \
  [--insecure] \
  [-l logs/sync-images.log]
```

依次执行 `pull-images` 与清单批量 `push-images` 的完整纯 Rust 编排流程。

---

## 7. 配置文件与离线 K8s 编排

### 7.1 Inventory 核心配置示例（`configs/inventory.yaml`）

> [!IMPORTANT]
> 正式执行前请参照 [OPERATION_GUIDE.md](./OPERATION_GUIDE.md) 将示例占位口令 `change-me` 替换为强口令，并配置专用 SSH 密钥。

```yaml
vars:
  ansible_user: "root"
  ansible_ssh_pass: "change-me"
  # 推荐使用密钥认证并开启主机指纹校验（详见 OPERATION_GUIDE.md）：
  # ansible_ssh_private_key_file: "~/.ssh/rtkk_ed25519"
  # ansible_ssh_host_key_checking: "accept-new"
  # 非 root 账号提权可选配置：
  # ansible_become: true
  # ansible_become_user: "root"

  images_file: "configs/images.yaml"
  pki_dir: "./pki"
  cert_validity: "867240h"              # 默认 99 年；可按需设为 "1y" / "8760h" / "3y" / "10y"

  registry:
    url: "10.0.0.10/rtkk"
    host: "10.0.0.10"
    scheme: "auto"                      # "auto" | "https" | "http"
    # ca_cert_file: "./pki/registry-ca.crt"
    username: "admin"
    password: "change-me"
    # auth_basic 省略时由 rtkk 自动根据 username:password 派生
    harbor_project_public: true         # 设为 false 可将新建 Harbor 项目设为私有
    insecure: true                      # 启用受信任 HTTPS 后建议设为 false
    push_empty_policy: warn_ok

  image_sync:
    output_dir: "./downloads/images"
    platform: "linux/amd64"
    insecure: false                     # 公网上游源默认强制校验证书
    retries: 3
    layer_concurrency: 4
    http_timeout_secs: 600

  control_plane_endpoint: "10.0.0.68:6443"
  vip_interface: "ens33"
  pod_subnet: "10.233.0.0/18"
  service_subnet: "10.233.64.0/18"
  cluster_dns_ip: "10.233.64.10"
  cluster_domain: "cluster.local"
  cluster_pool_mask_size: "24"
  nodelocaldns_ip: "169.254.20.10"

  public_registries:
    - docker.io
    - registry-1.docker.io
    - registry.k8s.io
    - quay.io
    - gcr.io
    - ghcr.io

  kube_version: "v1.35.6"
  etcd_version: "v3.6.12"
  containerd_version: "1.7.32"          # 自动适配 1.x (config v2) 与 2.x (config v3)

hosts:
  c23-p61: 10.0.0.61
  c23-p62: 10.0.0.62
  c23-p63: 10.0.0.63
  c23-p64: 10.0.0.64
  c23-p65: 10.0.0.65
  c23-p66: 10.0.0.66

roles:
  kube_control_plane: [c23-p61, c23-p62, c23-p63]
  kube_node: [c23-p64, c23-p65, c23-p66]
  etcd: [c23-p61, c23-p62, c23-p63]
  k8s_cluster: [kube_control_plane, kube_node]
```

### 7.2 仓库内 Playbook 与模板说明

| 文件路径 | 职责说明 |
|----------|----------|
| `configs/download.yaml` | 在 `localhost` 下载离线二进制与归档（kubeadm / kubelet / kubectl / runc / containerd / etcd / CNI / helm），并通过内置变量 `{{ .rtkk_exe }}` 与 `{{ .inventory_file }}` 执行 `pull-images` 和批量 `push-images` |
| `configs/images.yaml` | 定义 Kubernetes 控制面、kube-vip、CoreDNS、pause、NodeLocalDNS、Cilium、OpenEBS LocalPV 的源镜像与目标私库地址映射 |
| `configs/deploy.yaml` | Play 0–8 全自动部署：环境检测清理 → 关闭 swap / 内核模块 / sysctl / DNS → containerd 与私库 `0600` `hosts.toml` → 外部高可用 etcd 集群与证书 → kube-vip 与控制面初始化/扩容 → Worker 加入 → Cilium / OpenEBS / NodeLocalDNS 部署 → 集群状态备份至 `./rtkk_bak/` |
| `configs/cleanup.yaml` | 停止相关服务、执行 `kubeadm reset`、清理运行时目录、证书与二进制文件 |
| `configs/templates/*.gtmpl` | `kubeadm`（`v1beta4`，面向 K8s `>= v1.31`）、`containerd`（按 `containerd_version` 自动切换 `1.x` `version = 2` 与 `2.x` `version = 3`）、`etcd`、`cilium`、`local-volume`、`nodelocaldns` 渲染模板 |

---

## 8. 典型工作流（离线 K8s 部署）

在控制机（推荐 Debian 13 `Trixie`）仓库根目录执行：

```bash
mkdir -p logs downloads/{bin,tar,images}

# 0) 参照 OPERATION_GUIDE.md 完成密码轮换与 SSH 密钥分发后开始：

# 1) 在联网准备阶段拉取二进制与容器镜像并推送至内网私库
./target/release/rtkk run configs/download.yaml \
  -i configs/inventory.yaml -l logs/download.log

# 2) （可选）若目标节点存在历史残留组件，先执行清理
./target/release/rtkk run configs/cleanup.yaml \
  -i configs/inventory.yaml -l logs/cleanup.log

# 3) 同步全集群 /etc/hosts 主机名解析
./target/release/rtkk prep hosts \
  -i configs/inventory.yaml -l logs/prep-hosts.log

# 4) 执行离线高可用 Kubernetes 集群部署
./target/release/rtkk run configs/deploy.yaml \
  -i configs/inventory.yaml -l logs/deploy.log
```

部署完成后在任一控制面节点验证集群状态：

```bash
export KUBECONFIG=/etc/kubernetes/admin.conf
kubectl get nodes -o wide
kubectl get pods -A
```

本地生成产物目录说明：

| 目录路径 | 内容说明 |
|----------|----------|
| `downloads/bin/` | `kubeadm`、`kubelet`、`kubectl`、`runc`、`helm` 二进制文件 |
| `downloads/tar/` | `containerd`、`etcd`、CNI 插件、`helm` 压缩归档 |
| `downloads/images/*.tar` | 按镜像名归档的 docker-legacy tar 包（含 `rtkk-media.json`） |
| `pki/` | 本地签发并缓存的集群 CA 与组件证书（由 `vars.pki_dir` 指定） |
| `rtkk_bak/` | 部署收尾阶段自动拉取的控制面配置与关键状态备份 |

---

## 9. 运维准备与安全指南

`rtkk` 默认提供低门槛的内网极简部署体验，同时内置完整的认证、加密、提权与证书周期配置能力。面向正式环境交付前，建议阅读以下运维文档：

- **[运维准备与安全建议手册（OPERATION_GUIDE.md）](./OPERATION_GUIDE.md)**：行前必做检查清单（Pre-Flight Checklist）、Debian 13 密码与 SSH 密钥初始化命令、99 年默认证书原理及 1 年/多年期切换方法。
- **[镜像仓库认证与 Harbor 权限配置指南](./docs/registry-auth-and-harbor.md)**：OCI Distribution v2 `Basic` / `Bearer` 自动令牌协商、Harbor 项目公开/私有控制、Robot Account 最小权限拆分、节点 `0600` `hosts.toml` 配置。
- **[内网镜像仓库 TLS 加固与 HTTP 风险缓解指南](./docs/registry-tls-and-http.md)**：`scheme` 协议模式、明文 HTTP 风险与 Manifest+Blob SHA256 强校验机制、使用 `rtkk cert gen` 在 Debian 13 上快速启用严格 HTTPS。
- **[SSH 认证、非 Root 提权（become/sudo）与主机指纹校验指南](./docs/ssh-and-privilege-escalation.md)**：Debian 13 `root` 密钥登录加固、普通账号 `deploy` + `sudoers` 配置、Play/Task 级 `become` 提权、`accept-new` / `strict` `known_hosts` 指纹校验。

---

## 10. 编译、测试与质量门禁

本地开发与校验命令：

```bash
cargo fmt --check
cargo clippy --all-targets
cargo test --workspace
cargo build --release
./target/release/rtkk --version
```

此外，`test/` 目录包含独立的集成与仿真测试套件，覆盖 Inventory 解析、Playbook 调度、模板渲染、证书签发校验与 OCI Registry 协议交互等场景。

在线安装已发布的 Linux `x86_64`（基于 `debian:13` 构建）发行版二进制：

```bash
curl -fsSL https://github.com/jjdufu/rtkk/releases/latest/download/install.sh | bash
rtkk --version
```

---

## 11. 环境变量与日志

### 11.1 环境变量

| 变量名 | 默认值 | 说明 |
|--------|--------|------|
| `RTKK_EMPTY_HOSTS` | `fail` | 当 Play 匹配到的目标主机列表为空时的处理策略：`fail`（报错中止）或 `skip` / `warn` / `ok`（记录并跳过该 Play） |
| `RTKK_MAX_CMD_OUTPUT_BYTES` | `67108864`（64 MiB） | 单次远端 SSH 命令 `stdout` / `stderr` 输出字节上限，超出上限立即报错防止内存耗尽 |
| `RTKK_HTTP_TIMEOUT_SECS` | `600` | `http_get_file` 任务单文件下载超时时间（秒） |

### 11.2 日志管理

通过全局参数 `-l` / `--log-file <PATH>` 指定无 ANSI 控制符的结构化日志文件（未指定时默认写入当前目录 `deploy.log`）；交互式终端（TTY）下同步渲染实时任务进度条，子命令退出前自动刷盘。

| 执行场景 | 推荐日志路径 |
|----------|--------------|
| 下载二进制与同步镜像 | `logs/download.log`、`logs/pull-images.log`、`logs/push-images.log`、`logs/sync-images.log` |
| 主机名映射准备 | `logs/prep-hosts.log` |
| 集群部署与清理 | `logs/deploy.log`、`logs/cleanup.log` |

目标节点侧组件排障可结合 `journalctl -u kubelet -u containerd -u etcd`、`/etc/kubernetes/admin.conf` 与 `/etc/containerd/certs.d/` 进行核查。

---

## 12. 核心运行时依赖

主要 Rust 生态依赖（详见 `Cargo.toml`）：`tokio`（异步运行时）、`async-ssh2-lite` / `ssh2-tokio`（SSH2 / SFTP 会话）、`reqwest`（HTTP/TLS 客户端）、`clap`（命令行解析）、`serde` / `serde_yml` / `serde_json`（配置序列化）、`gtmpl`（Go 风格模板引擎）、`tar` / `flate2`（归档流处理）、`sha2`（SHA256 摘要校验）、`rcgen` / `x509-parser`（X.509 证书签发与解析）、`tracing` / `indicatif`（日志与终端进度展示）。
