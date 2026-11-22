# 01 · Stage A · Step M1：同源工程记忆矩阵与五条全局铁律

本步骤在不改动 `src/` 与 `configs/` 现有业务行为的前提下，建立多端 AI 编码代理共享的顶层工程记忆矩阵与零豁免红线，统一约束后续全部 Rust 引擎、Park UI 前端、YAML Playbook 与容器化交付代码。

---

## M1.1 目标

1. 建立根目录 `IRON_RULES.md`，固化 `rtkk` 五条不可逾越的全局工程铁律。
2. 升级根目录 `AGENTS.md`，并创建同源同步的 `GEMINI.md` 与 `CLAUDE.md`，保证不同开发终端加载完全一致的工具链、代码门禁与协同纪律。
3. 预留 `web/AGENTS.md` 规范契约（在 Stage B Step 03 初始化 `web/` 目录时同步写入），锁定 Park UI 双色 Token、无原生弹层控件与极简微文案标准。
4. 更新 `.gitignore`，纳管门禁运行态目录、Compose 运行态挂载目录与前端构建缓存。

---

## M1.2 前置条件与负责角色

- **前置条件**：仓库处于 `v0.0.5` 基线状态，53 项存量测试全部通过。
- **负责角色**：`orchestrator`

---

## M1.3 文件变更清单

| 动作 | 路径 | 说明 |
| :--- | :--- | :--- |
| 新建 | `IRON_RULES.md` | 五条全局不可逾越红线 |
| 修改 | `AGENTS.md` | 融合现有 Rust 专有规则与全仓文档导航、门禁命令、子栈指引 |
| 新建 | `GEMINI.md` | 与 `AGENTS.md` 同源同步的 Gemini / Antigravity 工程记忆入口 |
| 新建 | `CLAUDE.md` | 与 `AGENTS.md` 同源同步的 Claude Code 工程记忆入口 |
| 修改 | `.gitignore` | 新增 `.gate/`、`.env`、`deploy/compose/{data,logs,backups}/`、`web/{node_modules,dist,test-results,playwright-report}/` |

---

## M1.4 核心规范内容规格

### M1.4.1 `IRON_RULES.md` 五条全局红线

1. **离线零外部运行时依赖**：
   - 受控 Debian 13 集群节点仅依赖 OpenSSH 与基础系统工具，严禁要求预装 Python 或容器守护进程。
   - WebUI 构建产物全部内嵌进 `rtkk` 单一二进制，严禁引用任何外部在线字体、图标库或公网 CDN 资源；在完全断网的内网环境下打开浏览器必须 100% 正常渲染与交互。
   - 新增任何 Rust 或前端第三方依赖前，必须先登记于 `docs/standards/A6-DEPENDENCY-BASELINE.md` 并核验许可证（严禁引入 AGPL / SSPL 等传染性许可证组件）。
2. **契约先行与公共入口写入锁**：
   - 跨模块功能开发必须先由 `orchestrator` 合入契约（Rust DTO / 枚举、`docs/API_CONTRACT.md`、`docs/standards/A1,A4,A7`、`web/src/lib/api/` 类型签名与 `scripts/gate/stepNN.sh`），再由各分队在独立 `git worktree` 并行编码。
   - 非 `orchestrator` 角色严禁修改公共入口文件：`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`src/lib.rs`、`src/main.rs`、`src/cli.rs`、`src/daemon/routes/mod.rs`、`web/src/App.tsx`、`.github/`（由 `scripts/check-write-scope.sh` 强制拦截）。
3. **代码零绕过静态守卫**：
   - **Rust（`1.100.0`，Edition 2024）**：全仓 `unsafe_code = "forbid"`；严禁使用 `#[allow(...)]`、`#![allow(...)]`、`#[expect(...)]`、`#[warn(...)]` 压制告警；业务代码严禁 `unwrap()`、`expect()`、`panic!`、`todo!`、`unimplemented!`、`as` 类型强转与切片下标 `[]`；除 CLI 终端层（`src/main.rs`、`src/cli.rs`、`src/logging/`）外，核心引擎与 Daemon 路由层严禁裸 `println!` / `eprintln!` / `dbg!`；所有公开 API 必须具备完整 Rustdoc 且 `RUSTDOCFLAGS='-D warnings' cargo doc` 零警告。
   - **Web（React 19 + TS Strict + Park UI）**：严禁 `: any`、`as any`、`@ts-ignore`、`@ts-expect-error`、`eslint-disable`；严禁使用浏览器原生 `<select>`、`alert()`、`confirm()`；强制使用 `@ark-ui/react` 状态机原语构建键盘可访问浮层；控件微文案仅用 2–4 字简练动词/名词（如 `保存`、`开始预检`、`停止`），严禁交互赘词（`点击`、`选择`）与回调链描述。
4. **凭据零泄漏与变更互斥保护**：
   - 仓库内所有配置文件、测试夹具与文档严禁包含真实生产密码、Token 或私钥；远端落地敏感文件（私钥、`hosts.toml`、`admin.conf`）权限强制设为 `"0600"`（目录 `"0700"`）。
   - `rtkk serve` 宿主机直接启动时默认绑定 `127.0.0.1:19912`，强制开启高熵访问令牌会话鉴权；所有读取 Inventory 或配置详情的 API 默认对密码与私钥字段执行掩码脱敏（`********`）；针对同一工作区的集群写任务强制单飞互斥锁。
5. **分支隔离与三级门禁纪律**：
   - 禁止向仓库提交构建缓存、运行日志、`.env` 或临时测试产物；任何代码与文档变更合入 `main` 前必须完整通过 `docs/QUALITY_GATES_AND_VERIFICATION.md` 定义的 G1、G2、G3 门禁校验。

### M1.4.2 记忆文件预算与同源要求

- `AGENTS.md`、`GEMINI.md`、`CLAUDE.md` 单文件体积严格控制在 `< 24,000` 字节以内（由 Step M3 的 `scripts/docs-and-plans-guard.py` 校验）。
- 完整保留现有 `AGENTS.md` 中的 Rust 工具链约定：
  - 代理 Shell 不假定 `~/.cargo/bin` 已在 `PATH` 中，执行 `cargo` / `rustc` 前必须 `source "$HOME/.cargo/env"`；
  - 一律使用 `$HOME` 或 `~`，禁止写死特定用户名家目录；
  - 严格遵守 `rust-toolchain.toml` 指定的 `1.100.0`，禁止擅自切换或升级工具链；
  - 默认使用 Debug 构建，先跑单测目标再跑 `--workspace`，汇报结论时附上实际执行的命令与输出。

---

## M1.5 验收标准

1. `IRON_RULES.md`、`AGENTS.md`、`GEMINI.md`、`CLAUDE.md` 均已落盘且字节数 `< 24,000`。
2. 所有文档内引用的文件路径准确无误，不含任何外部无关项目术语或历史迭代叙述词。
3. `.gitignore` 已覆盖 `.gate/`、`.env`、`deploy/compose/{data,logs,backups}/` 与 `web/{node_modules,dist,test-results,playwright-report}/`。
