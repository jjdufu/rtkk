# 04 · Stage B · Step 04：集群与全量参数配置工作台、DNS 劫持免证书开关及实时模板预览器

本步骤打通 `configs/inventory.yaml` 的结构化脱敏编辑与双向 YAML 同步、`root` / `sudo` / `su` 三模式只读在线连通性探测、公网镜像源内网 DNS 劫持免证书双栈配置（`public_registries_insecure`），以及面向任意目标节点的 6 大 `.gtmpl` 配置文件与 `images.yaml` 实时渲染预览。

---

## 1. 目标与边界

1. **敏感凭据脱敏与无损回写闭环**：`GET /api/v1/inventory` 将所有 SSH/提权/私库/源站口令及 `auth_basic` 掩码为 `"********"`；`PUT /api/v1/inventory` 对保持 `"********"` 的字段自动还原磁盘原密文，对修改后的私库口令自动重算 `registry.auth_basic`，写前留存 `0600` `.bak` 快照。
2. **公网域名 DNS 劫持免证书双栈开关**：在 `configs/inventory.yaml` 与 `configs/deploy.yaml` 落地布尔参数 `public_registries_insecure`（默认 `false`）；设为 `true` 时为 `public_registries` 列表生成同时启用 `https://`（`skip_verify = true`）与 `http://` 自动回退的 `hosts.toml`，适配内网将 `docker.io` / `registry.k8s.io` 等域名劫持至透明缓存站的场景。
3. **只读在线节点探测**：并发测试指定节点的 SSH 认证、`sudo` 或 `su` PTY 提权（执行 `id -u`）、`/etc/os-release` Debian 13 识别、CPU 架构、物理网卡名列表与关键系统包就绪态，零远端文件写入。
4. **多节点实时模板预览台**：选定任一集群节点后，实时调用 Rust `gtmpl` 引擎渲染 `kubeadm-config.yaml`、`containerd-config.toml`、`etcd.service`、`cilium-values.yaml`、`local-volume.yaml`、`nodelocaldns.yaml` 与 `images.yaml`，同步执行 YAML/TOML 结构合法性校验。

---

## 2. 分队职责与文件变更矩阵

| 负责角色 | 动作 | 文件路径 | 变更内容说明 |
| :--- | :---: | :--- | :--- |
| `orchestrator` | 修改 | `src/daemon/routes/mod.rs`、[A4](../../docs/standards/A4-API-CONTRACT-INDEX.md)、`web/src/App.tsx` | 挂载 Step 04 共 5 条路由并迁入 `A4` 已挂载表；绑定 `/inventory` 与 `/templates` 页面路由 |
| `squad-daemon` | 新建 | `src/daemon/mask.rs` | 深度敏感键路径掩码器、`"********"` 原密文合并器与 `registry.auth_basic` 自动派生器 |
| `squad-daemon` | 新建 | `src/daemon/routes/{inventory.rs,templates.rs}` | Inventory 读写/校验/在线探测接口与多模板实时渲染预览接口 |
| `squad-web` | 新建 | `web/src/lib/api/{inventory.ts,templates.ts}` | 强类型配置、探测结果与模板预览 API 客户端 |
| `squad-web` | 新建 | `web/src/pages/{InventoryPage,TemplatesPage}.tsx` | 7 标签分区配置台（含过滤可见行全选/半选主机表、IP 段批量生成器、表单⇄YAML 同步）与节点模板预览器 |
| `squad-ops-qa` | 修改 | `configs/{inventory.yaml,deploy.yaml}`、`test/configs_render_tests.rs` | 增加 `public_registries_insecure: false` 默认项与双栈 `hosts.toml` 模板分支及单测 |
| `squad-ops-qa` | 已预置 | [scripts/gate/step04.sh](../../scripts/gate/step04.sh) | G3 门禁脚本（覆盖 `C21`、`C22`、`C23`） |

---

## 3. HTTP 接口与配置渲染契约

### 3.1 本步挂载的 5 条接口规格

| 方法与路径 | 鉴权 / 互斥 | 请求体摘要 | 成功响应摘要 |
| :--- | :---: | :--- | :--- |
| `GET /api/v1/inventory` | 已登录 / 只读 | 无 | `{"inventory":MaskedInventory,"groups":Record<string,string[]>,"derived":DerivedVars,"masked_yaml":"..."}` |
| `PUT /api/v1/inventory` | 已登录 / 变更 | `{"mode":"structured","inventory":...}` 或 `{"mode":"yaml","raw_yaml":"..."}` | `{"saved":true,"backup_path":"configs/inventory.yaml.bak","derived":DerivedVars}` |
| `POST /api/v1/inventory/validate` | 已登录 / 只读 | 同 `PUT` 请求体 | `{"valid":true,"hosts_count":6,"derived":DerivedVars,"warnings":[]}` |
| `POST /api/v1/inventory/probe` | 已登录 / 只读 | `{"hosts":["c23-p61"]}`（空数组表示全量非本机节点） | `{"results":[{"host":"c23-p61","reachable":true,"uid":0,"os_pretty":"Debian GNU/Linux 13 (trixie)","arch":"x86_64","nics":["ens33"],"latency_ms":18,"missing_packages":[]}]}` |
| `POST /api/v1/templates/render` | 已登录 / 只读 | `{"host":"c23-p61","templates":["all"]}` | `{"host":"c23-p61","files":[{"name":"containerd-config.toml","content":"...","syntax_valid":true}]}` |

### 3.2 公网域名 DNS 劫持免证书 `hosts.toml` 模板规格（`configs/deploy.yaml`）

```yaml
- name: Write hosts.toml for public registries
  copy:
    content: |
      server = "https://{{ .item }}"
      {{- if and .public_registries_insecure (ne (printf "%v" .public_registries_insecure) "false") }}
      [host."https://{{ .item }}"]
        capabilities = ["pull", "resolve"]
        skip_verify = true
      [host."http://{{ .item }}"]
        capabilities = ["pull", "resolve"]
        skip_verify = true
      {{- else }}
      [host."https://{{ .item }}"]
        capabilities = ["pull", "resolve"]
      {{- end }}
    dest: "/etc/containerd/certs.d/{{ .item }}/hosts.toml"
    mode: "0644"
  with_items: "{{ .public_registries }}"
```

### 3.3 前端交互与表格契约（`InventoryPage.tsx`）

1. **7 个 `@ark-ui/react` `Tabs` 分区**：`主机拓扑`、`连接提权`、`镜像私库`、`集群网络`、`组件版本`、`系统基线`、`YAML 源码`。
2. **主机多选表行为**：
   - 搜索框按主机名、IP、角色组实时过滤后，表头复选框**仅作用于当前可见过滤行**；当可见行部分选中时精确设置 `indeterminate` 半选态。
   - IP 段快捷扩展器支持如前缀 `c23-p` + `192.168.26.61-66` 一次性生成连续节点并绑定目标角色组。
   - 所有 IP、端口、CIDR 掩码位、探测耗时列统一应用 `.tabular-nums`。
3. **按钮微文案**：仅使用 `保存`、`校验`、`批量添加`、`探测连通性`、`删除`、`复制`。

---

## 4. 对抗审查要点（Verifier & RedTeam Checklist）

1. **YAML 模式脱敏防误擦**：用户在 `YAML 源码` 标签页直接编辑非密码字段并提交含 `"********"` 的 YAML 文本时，`mask.rs` 必须解析新旧抽象语法树按精确键路径还原磁盘原密文，禁止将字面量 `"********"` 落盘覆盖真实口令。
2. **探测任务超时保护**：`POST /api/v1/inventory/probe` 单主机连接与提权探测强制施加 `8s` 超时上限，离线不可达主机在结果数组内标记 `reachable: false` 与具体错误原因，不拖死整批请求。

---

## 5. 门禁验收标准（[scripts/gate/step04.sh](../../scripts/gate/step04.sh)）

- `python3 scripts/check_routes.py` 核验累计 14 条已挂载路由双向对齐。
- 追踪编号：`C21`（`"inventory-secret-masked"`、`"inventory-preserve-unmodified-secret"`）、`C22`（`"public-registries-hijack-hosts-toml"`）、`C23`（`"template-preview-render-ok"`）。
- `step04.sh` 末行输出 `GATE step04 PASS`。
