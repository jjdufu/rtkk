# RTKK

RTKK 是用 **Rust** 编写的轻量级集群编排与配置管理工具（免 Python 的 Ansible 风格引擎）。通过 SSH 或本机连接在多节点并发执行 Playbook，并内置 **纯 Rust OCI Registry 客户端**：公网镜像拉取 → docker-legacy tar 留存 → 私库推送（**不依赖 Docker / crane / containerd 运行时**）。

- **版本**：`0.0.0` <!-- x-release-please-version -->（`Cargo.toml` / `rtkk --version`）
- **Rust**：edition `2024`，`rust-version = "1.85"`
- **Lint**：workspace 级 `warnings` / clippy `all`+`pedantic` 等为 **deny**（含 `unwrap_used` / `expect_used` / `allow_attributes` 等），禁止 `#[allow]` 掩盖

仓库同时提供一套 **离线 Kubernetes** 示例配置（`configs/`）：下载二进制与镜像、`prep hosts`、部署 HA 控制面 / worker / Cilium / OpenEBS / NodeLocalDNS、清理与备份。

---

## 目录

- [1. 核心特性](#1-核心特性)
- [2. 源码结构](#2-源码结构)
- [3. 架构模块](#3-架构模块)
- [4. 内置任务模块](#4-内置任务模块)
- [5. 模板辅助函数](#5-模板辅助函数)
- [6. 命令行](#6-命令行)
- [7. 配置与仓库内示例](#7-配置与仓库内示例)
- [8. 典型工作流（离线 K8s）](#8-典型工作流离线-k8s)
- [9. 编译与质量门禁](#9-编译与质量门禁)
- [10. 环境变量](#10-环境变量)
- [11. 日志](#11-日志)
- [12. 破坏性变更与依赖](#12-破坏性变更与依赖)

---

## 1. 核心特性

1. **被控端零额外依赖**：节点只需 SSH/SFTP；无需 Python、Docker 等。
2. **Playbook 引擎**：`serial` 分批、`forks` 并发连接池、`when` / `loop` / `block`·`rescue`·`always`、`register`、重试退避、`tags` / 任务超时（opt-in）、Ansible 风格 `roles/` 编译。
3. **层级 Inventory**：扁平 `hosts`+`roles` 或嵌套 `all/children`；变量深度合并；加载时注入动态变量（`groups`、`first_control_plane`、etcd 端点等，见 §3.2）。
4. **连接层**：本机自动 `Local`（`localhost` / 本机网卡 IP / `ansible_connection: local`）；远端 SSH（私钥内容 / 路径 / 默认 `~/.ssh/id_*` / 密码，可选密钥失败回落密码）；传输错误可重建会话重试一次。
5. **纯 Rust 镜像链路**：
   - `pull-images`：OCI Distribution 拉取 + 写 docker-legacy tar（含 `rtkk-media.json`）
   - `push-images`：单 tar **或** 清单批量推私库（互斥）
   - `sync-images`：pull + 与批量 push 相同的上传编排
6. **模板**：Go 风格 `gtmpl` + 约 39 个集群部署向辅助函数。
7. **证书**：`gen_cert`（ECDSA P-256）本地签发后上传，支持幂等策略。

---

## 2. 源码结构

```text
src/
  main.rs                 # 二进制入口 → rtkk::cli::run_cli
  lib.rs                  # 库根（模块声明）
  cli.rs                  # clap 子命令、load_inventory、dispatch
  error.rs                # RtkkError
  prep.rs                 # prep hosts
  cert.rs / utils.rs
  connector/              # Local / SSH + create_connector
  inventory/              # model / graph / resolver
  playbook/               # parser、engine、动态 inventory 注入、条件/循环 helpers
  tasks/                  # 任务模块分发与实现（非单文件 tasks.rs）
  template/               # gtmpl + helpers/*
  registry/               # pull / legacy tar / push / media / manifest
  logging/                # 文件日志 + TTY 进度条
configs/
  inventory.yaml          # 主机、版本、registry、网段、public_registries 等 vars
  images.yaml             # 镜像 source/target 清单（可模板）
  download.yaml           # 二进制下载 + pull-images + 批量 push-images
  deploy.yaml             # 离线 K8s 部署（Play 0–8）
  cleanup.yaml            # 集群组件与目录清理
  templates/              # .gtmpl（kubeadm/containerd/etcd/cilium/local-volume/nodelocaldns）
```

约 50 个 Rust 源文件、~8.7k 行（`src/`）。

---

## 3. 架构模块

### 3.1 连接层（`src/connector/`）

- **`LocalConnector`**：`localhost`、解析到本机网卡的 IP，或 `ansible_connection` / `connector_type: local`。
  - `command`：`shell-words` 拆 argv 后直接 `exec`（**不经 shell**）。
  - `shell`：`/bin/sh -c`。
- **`SshConnector`**（`async-ssh2-lite`，crate 名 `ssh2-tokio`）：
  - 认证顺序：私钥 **内容** → 显式 `ansible_ssh_private_key_file`（路径存在且失败时默认硬失败）→ 未配置显式密钥时默认 `~/.ssh/id_ed25519` / `id_ecdsa` / `id_rsa` → 密码。
  - `ansible_ssh_private_key_fallback_password: true` 时，显式密钥失败才可回落密码。
  - 其它 inventory 字段：`ansible_user`、`ansible_ssh_pass`、`ansible_port`。
  - `command`：argv join 后 `channel.exec`（不经 `/bin/sh`）；`shell`：`/bin/sh -c '…'`。
  - 传输错误可重建会话重试一次；`fetch` 使用 64KB 流式写本地。
  - 命令 stdout/stderr 受 `RTKK_MAX_CMD_OUTPUT_BYTES` 限制（默认 16MiB）。

引擎侧为每主机维护 **连接池**（`PooledConnector` + `OnceCell` 并发建连去重），playbook 结束时统一 `close`。

### 3.2 Inventory（`src/inventory/`）

- **结构一**（仓库 `configs/inventory.yaml`）：顶层 `vars` + `hosts` + `roles`。
  - `hosts` 值可为 IP 字符串（注入 `ip`）或对象（直接作 host vars）。
  - `roles` 成员可为主机名或其它 role 名（递归展开）。
  - 自动补齐 `all` / `ungrouped` / `localhost`。
- **结构二**：`all` / `children` / 组内 `hosts`·`vars`（支持循环依赖检测）。
- 嵌套形态合并顺序：`all` vars → 组祖先链（拓扑序）→ 主机变量（`deep_merge`）。
- **动态 inventory 变量**（`inject_dynamic_inventory_vars`，加载时向**所有** host 注入）：

  | 键 | 含义 |
  |----|------|
  | `groups` | 解析后的组名 → 主机名列表（对象） |
  | `first_control_plane` | `kube_control_plane` 列表首个主机名（deploy 用其替代写死主机名） |
  | `etcd_initial_cluster` / `etcd_endpoints_str` / `etcd_endpoints` | 由 `etcd` 组（否则回落 `kube_control_plane`）推导 |

`cli::load_inventory` **单次加载**后复用；同时构建 `TargetConfig` 映射。

### 3.3 Playbook 引擎（`src/playbook/`）

- `compile_playbook` 解析 YAML；支持 play 级 `roles:` 展开为 block（读取 playbook 旁 `roles/<name>/tasks/main.y{a}ml` 与 `defaults/`）。
- `ExecutionEngine` 按 play 的 `hosts` 解析目标集合：
  - 组名 / 主机名
  - 索引：`kube_control_plane[0]`
  - 随机：`etcd|random`
- **`serial`**：固定数 / 百分比字符串 / 批次数组（数字或 `%`）。
- **`when`**：Go 模板表达式；渲染失败默认视为 false 并 warn，`RTKK_STRICT_WHEN=1` 硬失败。数组 `when` 为 AND。
- **`failed_when`**：若存在则**完全决定**任务是否 failed（注入 `stdout`/`stderr`/`rc`）。
- **`changed_when`**：可覆盖 `command`/`shell` 的 changed 判定。
- **`tags`**（任务级）+ CLI `--tags`（逗号分隔；过滤非空时无 tag 任务不跑）。
- **`timeout`**（任务级）+ CLI `--task-timeout`（秒；`0`/省略不限制）。
- **`run_once` / `delegate_to` / `ignore_errors`**（任务或 play 级）/ **`retries`+`delay`**（指数退避 + jitter）。
- **`loop` / `with_items`** + **`break_on_error`**。
  - YAML 数组字面量：逐项模板渲染后循环。
  - 纯变量引用字符串（如 `with_items: "{{ .public_registries }}"`）：若 host 上下文中该键为**数组**，直接按数组循环（gtmpl 字符串渲染无法保留 Array，故由引擎解析）。
- 上下文自动注入：`inventory_hostname`、`inventory_dir`、`global`（`set_global` / `run_once` 的 register）、按需 `hostvars`，以及 §3.2 的动态变量。

**变量优先级（低 → 高，role 任务执行时）**：

1. role `defaults`（仅填充缺失键）
2. inventory 主机/组变量
3. CLI `-s` extra_vars（启动时写入 bus）
4. play `vars`（play 开始时写入目标主机 bus）
5. role 参数（`roles: [{ role: x, k: v }]`，覆盖 bus）

### 3.4 模板（`src/template/`）

基于 `gtmpl`，在 `helpers/` 注册函数（见 §5）。Playbook 字符串、`images.yaml`、`.gtmpl` 文件共用同一套渲染能力。

`template` 任务解析 `src`：路径不存在时回落到 `{inventory_dir}/templates/{文件名}`（因此 playbook 里写 `./tmpl/templates/foo.gtmpl` 仍可命中 `configs/templates/foo.gtmpl`）。

叶子字符串渲染结果**保持字符串**（不再静默 JSON parse）；含 `{{` 且渲染失败时默认 warn 保留原串，`RTKK_STRICT_TEMPLATE=1` 硬失败。

### 3.5 日志与进度（`src/logging/`）

- 全局 `-l` / `--log-file`（默认文件名 `deploy.log`，tracing 无 ANSI）。
- TTY 下多主机任务进度条；非显式进度默认不假涨百分比（`RTKK_LEGACY_FAKE_PROGRESS=1` 可恢复旧动画）。
- 子命令结束调用 `shutdown_logging` 刷盘。

### 3.6 OCI Registry（`src/registry/`）

| 文件 | 职责 |
|------|------|
| `pull.rs` | 引用解析、HTTPS/HTTP probe、Basic/Bearer、manifest list 选 platform、blob 并发下载与 digest 校验 |
| `legacy_write.rs` | 写 docker-legacy tar、原子 rename、幂等 skip 检查 |
| `media.rs` | `rtkk-media.json` media type 元数据；gzip magic 嗅探回落 |
| `manifest.rs` | manifest / list 结构 |
| `image_list.rs` | `image_sync` 配置、渲染并加载 `images.yaml`、`source_auths` |
| `tar.rs` | 读 tar、`RepoTags` 前缀过滤、blob 上传调度 |
| `client.rs` | Registry V2 推送：blob HEAD/流式 PUT、manifest PUT、Harbor 项目自动创建 |
| `mod.rs` | `pull-images` / `push-images`（单/批）/ `sync-images` 编排 |

**镜像数据流：**

```text
公网 source  →  pull-images  →  downloads/images/{name}.tar
                                  (RepoTags = 私库 target；含 rtkk-media.json)
             →  push-images --images-file …  →  私库 registry.url
```

- 默认 platform：`linux/amd64`（`vars.image_sync.platform` 或 CLI `--platform`）。
- **TLS**：push / pull 默认**校验证书**；自签或明文私库设 `vars.registry.insecure: true` 与/或 `vars.image_sync.insecure: true`，或 CLI `--insecure`（pull/sync；`image_sync.insecure` 也会对齐 registry TLS）。
- **Layer media type**：pull 写入 `rtkk-media.json`；push 保真；旧 tar 无元数据时按 gzip magic 嗅探。
- **`image_sync` 可选项**：`output_dir`、`platform`、`insecure`、`retries`（默认 3）、`layer_concurrency`（默认 4）、`http_timeout_secs`（默认 600）、`source_auths`（按 registry host 的 pull 凭据）。

---

## 4. 内置任务模块

分发入口：`src/tasks/mod.rs`。同一任务只能有一个 action 模块（block 任务除外）。

| 模块 | 主要参数 | 说明 |
|------|----------|------|
| **`gen_cert`** | `cn`, `out_key`, `out_cert`, `root_*`, `sans`, `is_ca`, `date_hours`/`date`, `policy` | 本地签发/检查后上传；key `0600` / cert `0644`；CA 路径全局锁 |
| **`template`** | `src`, `dest`, `mode` | 渲染 gtmpl；回落 inventory 旁 `templates/`；SHA256+mode 幂等 |
| **`fetch`** | `src`, `dest` | 远端 → 本机 |
| **`http_get_file`** | `url`, `dest`, `sha256?` | 本机下载；可选校验；流式进度；原子 rename |
| **`command` / `shell`** | 字符串或 map（`_raw_params`） | 见 §3.1；支持 `changed_when` / `failed_when` |
| **`copy`** | `src` 或 `content`, `dest`, `mode` | 父目录 `mkdir -p`；`.tmp` 原子 `mv`；内容+mode 幂等跳过 |
| **`systemd`** | `name`, `state`, `enabled`, `daemon_reload` | 状态检测后变更 |
| **`sysctl`** | `name`, `value`, `sysctl_file` | 运行值与配置文件双检，幂等持久化 |
| **`modprobe`** | `name`, `state` | 加载/卸载；`present` 时写 `modules-load.d`；模块名校验 |

**`gen_cert.policy`**：`always`（默认）| `ifnotpresent` / `if_not_present` | `none`。

任务级公共字段：`when`、`loop`/`with_items`、`break_on_error`、`register`、`register_type`（`json`|`yaml`）、`set_global`、`retries`、`delay`、`ignore_errors`、`run_once`、`delegate_to`、`block`/`rescue`/`always`、`tags`、`timeout`、`changed_when`、`failed_when`。

**`register` 结果结构**（单次）：

```json
{ "stdout": "...", "stderr": "...", "rc": 0, "changed": true, "failed": false, "error": null }
```

- `register_type: json|yaml` 时尝试解析 `stdout`，失败则保留字符串。
- **loop** 时 register 值为同结构对象的 **数组**。
- `set_global: true` 或 `run_once` 时镜像到引擎 `global_results`，模板中通过 `{{ .global.<name> }}` 访问。

---

## 5. 模板辅助函数

注册于 `src/template/helpers/`（`add_func` 约 39 个）：

| 类别 | 函数 |
|------|------|
| 编解码 | `toJson`, `toYaml`, `fromYaml` |
| 字符串 | `trimPrefix`, `trimSuffix`, `indent`, `unquote`, `splitList`, `join`, `cat`, `regexMatch`, `regexReplaceAll` |
| 集合/逻辑 | `has`, `append`, `slice`, `first`, `index`, `subtractList`, `default`, `ternary`, `eq`, `ne`, `empty`, `not`, `getStringSlice`, `toLowerByteUnit`, `mapToNamedStringArgs` |
| 数值 | `sub`, `int`, `len`, `pow`, `randInt` |
| 网络 | `ipFamily`, `ipInCIDR`, `generateIPsFromCIDR`（大网段有下限限制） |
| 其它 | `fileExists`, `semverCompare`, `tpl`, `include` |

---

## 6. 命令行

全局：

```bash
rtkk [-l|--log-file PATH] <COMMAND>
rtkk --version
```

尾部参数 `help`（大小写不敏感）会被改写为 `--help`。

### 6.1 `run` — 执行 Playbook

```bash
rtkk run <playbook_path> \
  -i <inventory> \
  [-s key=val ...] \
  [-f <forks>] \
  [--tags a,b] \
  [--task-timeout SECS] \
  [-l logs/run.log]
```

| 参数 | 含义 |
|------|------|
| `-i` | Inventory 路径（必填） |
| `-s` | 额外变量，覆盖 host 上下文（写入所有主机 bus） |
| `-f` | 并发；否则 inventory `vars.forks` / `all.vars.forks`，再否则 `50` |
| `--tags` | 只跑带这些 tag 的任务；省略则全部 |
| `--task-timeout` | 全局每任务超时秒数；`0`/省略不限制（任务级 `timeout` 仍可单独设） |

### 6.2 `prep hosts` — 写 `/etc/hosts` 映射

```bash
rtkk prep hosts -i <inventory> [--best-effort] [-l logs/prep-hosts.log]
```

1. 各节点取 OS `hostname`  
2. 生成 `# RTKK HOSTS START/END` 块（IP、inventory 名、OS 名）  
3. 写入各节点 `/etc/hosts`（替换旧块）  
4. 本机 inventory 名 / OS 名映射到 `127.0.0.1`  
5. 默认 fail-fast；`--best-effort` 允许部分主机解析失败后继续  

**说明**：kube-vip 的 `control_plane_endpoint` 若已是 **IP:port**（如 `10.0.0.68:6443`），**不必**把 VIP 写进 hosts；hosts 只服务主机名解析。

### 6.3 `pull-images` — 纯 Rust 拉取并留存 tar

```bash
rtkk pull-images \
  -i <inventory> \
  --images-file configs/images.yaml \
  [--output-dir DIR] \
  [--platform linux/amd64] \
  [--force] \
  [--insecure] \
  [-l logs/pull-images.log]
```

- 清单条目：`name` / `source` / `target`（Go 模板，上下文为 inventory `vars`）。
- 输出：默认 `vars.image_sync.output_dir`（如 `./downloads/images/{name}.tar`）。
- 已存在且 `RepoTags` 等契约匹配时默认 **skip**；`--force` 覆盖。
- 空清单 **失败**（禁止静默成功）。

### 6.4 `push-images` — 推私库（单 tar XOR 清单批量）

```bash
# 单 tar（调试 / 补推）
rtkk push-images <tar_path> -i <inventory> [-l logs/push-images.log]

# 清单批量（生产）
rtkk push-images -i <inventory> \
  --images-file configs/images.yaml \
  [--output-dir DIR] \
  [-l logs/push-images.log]
```

| 规则 | 行为 |
|------|------|
| 仅 tar | 单包推送；无匹配 `registry.url` 前缀的 RepoTags 时由 `registry.push_empty_policy` 决定：`warn_ok`（默认）或 `fail` |
| 仅 `--images-file` | 对每个 `name` 推 `output_dir/{name}.tar`；**L2 校验**（存在 + `RepoTags` 精确含 `target` + 成员可读）；失败 fail-fast；空匹配策略固定 **Fail** |
| 两者都有 / 都无 | 错误退出 |
| 单 tar + `--output-dir` | 错误（`--output-dir` 仅批量） |

凭据：`vars.registry`（`url` / `username` / `password` / `insecure` / `push_empty_policy`）。  
批量 **不以扫目录为权威**；清单外 orphan tar 不推。

### 6.5 `sync-images` — pull + 批量 push

```bash
rtkk sync-images \
  -i <inventory> \
  --images-file configs/images.yaml \
  [--output-dir DIR] [--platform ...] [--force] [--insecure] \
  [-l logs/sync-images.log]
```

等价于 `pull-images` 后复用与批量 `push-images` **相同的上传编排**（非 shell for 循环）。

---

## 7. 配置与仓库内示例

### 7.1 Inventory 要点（`configs/inventory.yaml`）

离线 K8s 示例的关键 vars（密码等请按环境修改；完整列表以仓库文件为准）：

```yaml
vars:
  ansible_user: "root"
  ansible_ssh_pass: "..."
  rtkk_bin: "./target/release/rtkk"      # download playbook 调用自身
  rtkk_inventory: "configs/inventory.yaml"
  images_file: "configs/images.yaml"
  registry:
    url: "10.0.0.10/rtkk"
    host: "10.0.0.10"
    username: "admin"
    password: "..."
    auth_basic: "..."                   # containerd hosts.toml Basic
    insecure: true                      # 自签/HTTP 私库
    push_empty_policy: warn_ok          # 单 tar 无匹配：warn_ok | fail
  image_sync:
    output_dir: "./downloads/images"
    platform: "linux/amd64"             # 仅 OCI 镜像 pull/push；二进制下载架构见 download.yaml（H8 未完成多架构）
    insecure: true
    retries: 3
    layer_concurrency: 4
    http_timeout_secs: 600
  pki_dir: "./pki"
  # 必须为 IP:port；kube-vip port / kubeadm bindPort 均从此解析，勿只写 IP
  control_plane_endpoint: "10.0.0.68:6443"
  vip_interface: "ens33"                # kube-vip ARP 网卡，须与主网卡一致
  # 网段单一来源：kubeadm / Cilium / NodeLocalDNS 共用
  pod_subnet: "10.233.0.0/18"
  service_subnet: "10.233.64.0/18"
  cluster_dns_ip: "10.233.64.10"      # 通常与 kube-dns ClusterIP 一致
  cluster_domain: "cluster.local"
  cluster_pool_mask_size: "24"
  nodelocaldns_ip: "169.254.20.10"    # 节点本地 DNS 监听（link-local）
  public_registries:                    # containerd certs.d；playbook: with_items: "{{ .public_registries }}"
    - docker.io
    - registry-1.docker.io
    - registry.k8s.io
    - quay.io
    - gcr.io
    - ghcr.io
    # ...
  kube_version: "v1.35.6"
  etcd_version: "v3.6.12"
  # ... 其余组件版本、dns_upstream 等见仓库 inventory

hosts:
  c23-p61: 10.0.0.61
  # ...

roles:
  kube_control_plane: [c23-p61, c23-p62, c23-p63]
  kube_node: [c23-p64, c23-p65, c23-p66]
  etcd: [c23-p61, c23-p62, c23-p63]
  k8s_cluster: [kube_control_plane, kube_node]
```

引擎还会注入 `first_control_plane` / `groups` / etcd 端点（§3.2）；`deploy.yaml` 中 fetch/join/addons 条件使用 `.first_control_plane`，**不要**在 playbook 写死主机名。

### 7.2 镜像清单（`configs/images.yaml`）

当前清单覆盖控制面与插件相关镜像（`name` → `downloads/images/{name}.tar`）：

| name | 来源（模板化） |
|------|----------------|
| `kube-apiserver` / `kube-controller-manager` / `kube-scheduler` / `kube-proxy` | `registry.k8s.io/...:{{ .kube_version }}` |
| `kube-vip` | `docker.io/plndr/kube-vip:{{ .kube_vip_version }}` |
| `coredns` | `registry.k8s.io/coredns/coredns:{{ .coredns_version }}` |
| `pause` | `registry.k8s.io/pause:{{ .pause_version }}` |
| `k8s-dns-node-cache` | `registry.k8s.io/dns/...:{{ .nodelocaldns_version }}` |
| `cilium` / `operator-generic` | `quay.io/cilium/...:{{ .cilium_version }}` |
| `cilium-envoy` | `quay.io/cilium/cilium-envoy:{{ .cilium_envoy_version }}` |
| `provisioner-localpv` | `docker.io/openebs/provisioner-localpv:{{ .openebs_version }}` |

`target` 统一为 `{{ .registry.url }}/<name>:<tag>`。

### 7.3 仓库内 Playbook

| 文件 | 作用 |
|------|------|
| `configs/download.yaml` | `http_get_file` 下二进制/归档 + 解压 helm + 调用 `pull-images` / 批量 `push-images`（二进制 URL 仍为 amd64，见文件内 H8 TODO） |
| `configs/deploy.yaml` | 离线装 containerd/etcd/kubeadm、HA（kube-vip）、Cilium、OpenEBS local PV、NodeLocalDNS、备份等 |
| `configs/cleanup.yaml` | 停止服务、`kubeadm reset`、清目录与二进制 |
| `configs/templates/*.gtmpl` | 6 个模板：kubeadm、containerd、etcd、Cilium、local-volume、NodeLocalDNS（镜像清单以 `images.yaml` 为准，无独立 manifest 模板） |

**`deploy.yaml` Play 概览：**

| Play | hosts | 内容 |
|------|-------|------|
| 0 | `k8s_cluster` | 检测旧安装并清理 |
| 1 | `k8s_cluster` | swap off、modprobe、sysctl、DNS |
| 2 | `k8s_cluster` | containerd / runc / CNI / `public_registries` + 私库 certs.d |
| 3 | `etcd` | etcd 二进制、证书、`etcd` 与 snap-watcher unit |
| 4 | `kube_control_plane` | kubelet、kube-vip（VIP/端口来自 `control_plane_endpoint`）、`kubeadm init`、join |
| 5 | `kube_node` | worker 组件与 join |
| 6 | `kube_control_plane` | Cilium / OpenEBS local-volume / NodeLocalDNS apply（网段 vars） |
| 7–8 | 控制面 / 全集群 | 配置与状态备份到 `./rtkk_bak/` |

### 7.4 最小 Playbook 示例

```yaml
- name: Prepare kernel
  hosts: k8s_cluster
  serial: "50%"
  tasks:
    - name: Load overlay
      modprobe:
        name: overlay
        state: present
      tags: [kernel]

    - name: Sysctl bridge
      sysctl:
        name: net.bridge.bridge-nf-call-iptables
        value: 1
        sysctl_file: /etc/sysctl.d/99-kubernetes.conf
```

### 7.5 Role 目录布局（可选）

play 中：

```yaml
roles:
  - common
  - { role: app, app_port: 8080 }
```

相对 playbook 目录查找：

```text
roles/<name>/tasks/main.yaml   # 或 main.yml
roles/<name>/defaults/main.yaml
```

---

## 8. 典型工作流（离线 K8s）

工作目录为仓库根；先 `cargo build --release`。

```bash
mkdir -p logs downloads/{bin,tar,images}

# 1) 二进制 + 镜像 pull + 私库批量 push
./target/release/rtkk run configs/download.yaml \
  -i configs/inventory.yaml -l logs/download.log

# 2) （可选）清理旧集群
./target/release/rtkk run configs/cleanup.yaml \
  -i configs/inventory.yaml -l logs/cleanup.log

# 3) hosts
./target/release/rtkk prep hosts \
  -i configs/inventory.yaml -l logs/prep-hosts.log

# 4) 部署
./target/release/rtkk run configs/deploy.yaml \
  -i configs/inventory.yaml -l logs/deploy.log
```

验收（在 master 上）：

```bash
export KUBECONFIG=/etc/kubernetes/admin.conf
kubectl get nodes -o wide
kubectl get pods -A
```

产物目录：

| 路径 | 内容 |
|------|------|
| `downloads/bin/` | kubeadm、kubelet、kubectl、runc、helm |
| `downloads/tar/` | containerd、etcd、cni、helm 归档 |
| `downloads/images/*.tar` | 每镜像一个 docker-legacy tar |
| `pki/` | `gen_cert` 本地落盘（`vars.pki_dir`） |
| `rtkk_bak/` | deploy 备份输出 |

---

## 9. 编译与质量门禁

```bash
cargo fmt
cargo clippy --all-targets
cargo test
cargo build --release
./target/release/rtkk --version
```

要求：无 error；**不使用** `#[allow]` 掩盖 deny lint。  
当前内置单测较少（连接错误分类、hostvars 探测等）；回归以 clippy + 实际 playbook 执行为主。

发版由 Release Please 打 tag 后，拉取最新的 `debian:13` 镜像，只打包 Linux x86_64（仅在 Debian 13 上测试过）。安装已发布的二进制：

```bash
curl -fsSL https://github.com/jjdufu/rtkk/releases/latest/download/install.sh | bash
rtkk --version
```

---

## 10. 环境变量

| 变量 | 作用 |
|------|------|
| `RTKK_STRICT_WHEN=1` | `when` / `failed_when` / `changed_when` 渲染失败硬失败 |
| `RTKK_STRICT_TEMPLATE=1` | 含 `{{` 的模板渲染失败硬失败（否则 warn 保留原串） |
| `RTKK_EMPTY_HOSTS=fail\|skip\|warn\|ok` | 空 hosts 匹配策略；默认 fail；`skip`/`warn`/`ok` 均跳过该 play |
| `RTKK_MAX_CMD_OUTPUT_BYTES` | SSH 命令 stdout/stderr 上限，默认 16MiB，超限报错 |
| `RTKK_HTTP_TIMEOUT_SECS` | `http_get_file` 下载超时，默认 600 |
| `RTKK_LEGACY_FAKE_PROGRESS=1` | 恢复进度条假涨百分比动画 |

---

## 11. 日志

全局 `-l` / `--log-file` 写入指定文件（tracing 无 ANSI）。TTY 另有进度条。

| 场景 | 建议路径 |
|------|----------|
| download playbook | `logs/download.log` |
| pull-images | `logs/pull-images.log`（download 内嵌调用时也写此路径） |
| push-images | `logs/push-images.log` |
| sync-images | `logs/sync-images.log` |
| prep hosts | `logs/prep-hosts.log` |
| deploy | `logs/deploy.log` |
| cleanup | `logs/cleanup.log` |
| 未指定 `-l` | 当前目录 `deploy.log` |

节点侧排障（非 rtkk 日志）：`journalctl -u kubelet -u containerd -u etcd`、`/etc/kubernetes/admin.conf`、containerd `certs.d`。

---

## 12. 破坏性变更与依赖

### 破坏性变更（相对早期行为）

| 变更 | 迁移 |
|------|------|
| 模板渲染结果不再自动 JSON parse | 用 YAML 原生类型或 `toJson` helper |
| push / pull TLS 默认校验 | 设 `registry.insecure: true` 与/或 `image_sync.insecure: true` |
| Local `command` 不经 shell | 需要 `$` 展开/管道请改 `shell:` |
| 空 hosts 默认失败 | 修正 inventory / 设 `RTKK_EMPTY_HOSTS=skip` |
| role 参数覆盖 inventory / play vars | 依赖「host 压 role 参数」的 playbook 改 vars 布局 |
| 批量 push 要求 tar `RepoTags` 精确含 target | 用本工具 `pull-images` 产出，或修正 tar |

### 依赖（摘要）

运行时库见 `Cargo.toml`：`tokio`、`async-ssh2-lite`（`ssh2-tokio`）、`reqwest`、`clap`、`serde`/`serde_yml`/`serde_json`、`gtmpl`、`tar`、`sha2`、`rcgen`、`tracing` 等。  
**不**将 Docker、crane、skopeo 作为运行时依赖。
