# RTKK 全局工程铁律（Iron Rules）

本文档定义 `rtkk` 仓库不可豁免的五条顶层工程红线，统一约束 Rust 核心编排引擎、常驻 Daemon（`:19912`）、内嵌 Park UI 前端工作台、离线 Kubernetes Playbook 配置与容器化交付产物。任何代码、配置或文档变更违反任一条款，均由 G1/G2/G3 自动化门禁直接拦截。

---

## 铁律一：离线零外部运行时依赖

1. **受控节点零额外依赖**：目标 Debian 13（`Trixie`）集群节点仅依赖标准 OpenSSH（SSH / SFTP）与基础系统工具，严禁要求目标节点预装 Python 解释器、Ansible、crane、skopeo 或预置容器守护进程。
2. **单二进制纯离线 WebUI**：前端构建产物（`web/dist/`）在编译期完整内嵌至单一 `rtkk` 可执行文件；严禁在 HTML、CSS 或 TypeScript 中引用任何公网字体、外部图标库或第三方 CDN 地址（如 `fonts.googleapis.com`、`cdn.jsdelivr.net`、`unpkg.com`）。在完全物理隔离断网的内网环境下打开浏览器，界面必须 100% 正常渲染与交互。
3. **依赖准入与许可证红线**：新增任何 Rust crate 或前端 npm 依赖前，必须先在 [A6-DEPENDENCY-BASELINE.md](./docs/standards/A6-DEPENDENCY-BASELINE.md) 完成准入登记、说明现有依赖无法覆盖的原因并核验开源许可证；严禁引入 `AGPL`、`SSPL`、`BUSL` 等传染性或商业限制许可证组件。

---

## 铁律二：契约先行与公共入口写入锁

1. **契约先行闭环**：跨模块特性开发必须先由 `orchestrator` 提交契约变更（包含 Rust 公共结构体/枚举、[API_CONTRACT.md](./docs/API_CONTRACT.md)、[A1](./docs/standards/A1-CONVENTIONS-ENUMS-ROUTES.md) / [A4](./docs/standards/A4-API-CONTRACT-INDEX.md) / [A7](./docs/standards/A7-REQUIREMENTS-TRACEABILITY.md) 标准表、前端强类型接口签名以及对应 `scripts/gate/stepNN.sh` 验收门禁），随后各实现分队方可在独立 `git worktree` 并行编码。
2. **公共入口独占保护**：非 `orchestrator` 角色严禁修改以下公共入口与全局装配文件，违者由 [scripts/check-write-scope.sh](./scripts/check-write-scope.sh) 强制阻断：
   - 构建与工具链：`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`build.rs`、`.github/`
   - Rust 根入口与总路由：`src/lib.rs`、`src/main.rs`、`src/cli.rs`、`src/error.rs`、`src/daemon/mod.rs`、`src/daemon/state.rs`、`src/daemon/routes/mod.rs`
   - 前端根入口与全局注册表：`web/package.json`、`web/package-lock.json`、`web/tsconfig.json`、`web/vite.config.ts`、`web/src/App.tsx`、`web/src/lib/api/enums.ts`、`web/src/modules/registry.ts`
   - 治理文档与守卫底座：`IRON_RULES.md`、`AGENTS.md`、`GEMINI.md`、`CLAUDE.md`、`docs/`、`plans/`、`scripts/*.py`、`scripts/check-write-scope.sh`、`scripts/gate/lib.sh`、`scripts/gate/all.sh`

---

## 铁律三：代码零绕过静态守卫

1. **Rust 严格静态契约（`1.100.0`，Edition `2024`）**：
   - 全仓强制 `unsafe_code = "forbid"`；严禁任何 `unsafe` 块或声明。
   - 严禁使用 `#[allow(...)]`、`#![allow(...)]`、`#[expect(...)]`、`#![expect(...)]`、`#[warn(...)]` 局部或全局压制编译器与 Clippy 告警。
   - 全仓强制拒绝 `unwrap()`、`expect()`、`panic!`、`todo!`、`unimplemented!`、`unreachable!`、`as` 数值/指针强转与切片下标 `[]`；错误统一通过 `Result<T, RtkkError>` 与 `?` 显式向上传播。
   - 除 CLI 终端输出白名单（`src/main.rs`、`src/cli.rs`、`src/logging/mod.rs`、`src/logging/progress.rs`、`src/playbook/engine.rs`）外，其余业务模块与 `src/daemon/` 严禁出现裸 `println!`、`eprintln!` 或 `dbg!`。
   - 所有公开结构体、枚举、函数与模块必须具备完整 Rustdoc（含 `# Errors` 说明），且 `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` 零警告。
2. **Web 前端严格契约（React 19 + TypeScript Strict + Park UI）**：
   - 严禁使用 `: any`、`as any`、`<any>`、`@ts-ignore`、`@ts-expect-error`、`eslint-disable`。
   - 严禁使用浏览器原生 `<select>`、`alert()`、`confirm()`、`prompt()`；所有下拉选择、模态确认框、抽屉、菜单与标签页必须基于 `@ark-ui/react` 无障碍状态机原语构建。
   - 必须同时提供 Light 与 Dark 双套 Park UI 中性色阶 Token，保证交互控件与正文对比度 `>= 4.5:1`（严防暗色模式下强调色前景与背景同色）。
   - 界面控件微文案严格遵循 2–4 字简练动词或领域名词（如 `保存`、`探测私库`、`部署集群`、`停止`），禁止出现交互动作修饰词（`点击`、`点此`、`选择`、`Click`、`Press`）或冗长引导副标题。

---

## 铁律四：凭据零泄漏与变更互斥保护

1. **仓库与节点凭据卫生**：
   - 仓库内所有配置文件、测试夹具、门禁脚本与文档严禁包含真实生产口令、访问令牌、机器人密钥或私钥原文。
   - 分发至受控节点或控制机本地生成的敏感文件（私钥 PEM、含凭据 `hosts.toml`、`admin.conf`、`.rtkk-daemon/admin.token`）权限必须强制收敛为 `"0600"`，所属私密目录（`pki/`、`.rtkk-daemon/`）权限必须强制收敛为 `"0700"`。
   - Playbook YAML 中所有 `mode:` 字段值必须使用引号包裹为八进制字符串（如 `mode: "0600"`、`mode: "0755"`），严禁裸写无引号整数。
2. **Daemon 最小暴露与脱敏互斥**：
   - `rtkk serve` 在宿主机直接启动时默认绑定回环地址 `127.0.0.1:19912`（容器内监听 `0.0.0.0:19912` 并通过 Compose 端口映射暴露），强制开启高熵 CSPRNG 访问令牌与常量时间比对会话鉴权。
   - 所有读取 Inventory 或系统配置的 HTTP API 默认将 `ansible_ssh_pass`、`ansible_become_pass`、`registry.password`、`registry.auth_basic`、`source_auths.*.password` 掩码为 `"********"`；提交未修改的掩码占位符时后端自动保留原密文，绝不把星号写回磁盘。
   - 同一工作区（`Workspace`）内的集群变更型异步任务强制执行全局单飞互斥锁（并发写请求立即返回 `409 CONFLICT_ACTIVE_JOB`），仅允许只读诊断与状态查询并发放行。

---

## 铁律五：分支隔离与三级门禁纪律

1. **物理隔离与并发槽**：多代理分队开发必须使用独立 `git worktree` 分支隔离工作区，并按 `WT_SLOT=0..4` 绑定互不冲突的本地监听端口（主槽 `19912`，并发槽 `29912` / `39912` / `49912` / `59912`）与独立沙盒目录（`.gate/work-slot*`）。
2. **工作区洁净纪律**：严禁向 Git 仓库提交 `target/`、`web/node_modules/`、`web/dist/`、`.gate/`、`.env`、`logs/`、`downloads/`、`pki/`、`rtkk_bak/` 或任何运行态挂载数据。
3. **三级门禁全绿合入**：任何代码、配置或文档合入 `main` 分支前，必须完整通过 [QUALITY_GATES_AND_VERIFICATION.md](./docs/QUALITY_GATES_AND_VERIFICATION.md) 定义的 **G1（静态守卫）**、**G2（严格编译、文档与单测）**、**G3（端到端闭环门禁）** 全链路验证，并以 `bash scripts/gate/all.sh` 末行输出 `ALL GATES PASS` 作为唯一放行凭证。
