# RTKK 工程记忆与协作总纲

本文件与根目录 [AGENTS.md](./AGENTS.md)、[CLAUDE.md](./CLAUDE.md) 同源同步，供所有开发终端与 AI 编码代理加载一致的全局铁律、Rust / Web 工具链规范、分队边界与三级门禁指令。通用规则见 `~/.pi/agent/AGENTS.md`；两者冲突时以本仓库 [IRON_RULES.md](./IRON_RULES.md) 与本文件为准。

---

## 1. 必读宪章与文档导航

开始任何调研、方案修订、编码或审查前，按任务类型查阅对应权威文档：

| 文档路径 | 职责边界 |
| :--- | :--- |
| [IRON_RULES.md](./IRON_RULES.md) | 五条零豁免全局红线（离线内嵌、契约先行、零绕过守卫、凭据安全、三级门禁） |
| [docs/QUALITY_GATES_AND_VERIFICATION.md](./docs/QUALITY_GATES_AND_VERIFICATION.md) | G1 静态守卫、G2 编译/Rustdoc/单测、G3 `step00`～`step07` 端到端门禁标准 |
| [docs/MULTI_AGENT_TEAM_WORKFLOW.md](./docs/MULTI_AGENT_TEAM_WORKFLOW.md) | 五大角色独占写入路径白名单、`git worktree` 隔离与 `WT_SLOT` 端口槽规范 |
| [docs/API_CONTRACT.md](./docs/API_CONTRACT.md) | Daemon HTTP REST（`/api/v1/*`，默认 `:19912`）、令牌会话鉴权、脱敏与 SSE 流契约 |
| [docs/standards/A1-CONVENTIONS-ENUMS-ROUTES.md](./docs/standards/A1-CONVENTIONS-ENUMS-ROUTES.md) | 模块词表、CLI 子命令、前端路由表、公共枚举与 `RTKK_*` 环境变量全集 |
| [docs/standards/A4-API-CONTRACT-INDEX.md](./docs/standards/A4-API-CONTRACT-INDEX.md) | Daemon 路由契约索引表（由 `scripts/check_routes.py` 双向强校验） |
| [docs/standards/A5-TEST-FIXTURES-AND-GATES.md](./docs/standards/A5-TEST-FIXTURES-AND-GATES.md) | 确定性测试夹具生成规范与 `scripts/gate/lib.sh` 断言函数手册 |
| [docs/standards/A6-DEPENDENCY-BASELINE.md](./docs/standards/A6-DEPENDENCY-BASELINE.md) | Rust 与 Web 锁定依赖基线及许可证准入清单 |
| [docs/standards/A7-REQUIREMENTS-TRACEABILITY.md](./docs/standards/A7-REQUIREMENTS-TRACEABILITY.md) | `C01`～`C29` 能力需求追踪矩阵（由 `scripts/check_traceability.py` 核验） |
| [plans/README.md](./plans/README.md) | 分阶段演进实施方案仓与历史基线归档索引（与 `docs/` 严格动静分离） |

---

## 2. Rust 项目专有规则（`1.100.0` · Edition `2024`）

### 2.1 工具链
- `cargo`、`rustc`、`rustup` 通过 `rustup` 安装在当前用户的 `$HOME/.cargo/bin/` 下。代理 Shell 不加载 `~/.bashrc` 和 `~/.profile`，切勿假定这些命令已在 `PATH` 中；执行前必须先运行 `source "$HOME/.cargo/env"`，或使用 `"$HOME/.cargo/bin/cargo"` 全路径。
- 严禁在规则、脚本、配置或测试夹具中写死特定用户名家目录，一律使用 `$HOME` 或 `~`。
- 仓库以 [rust-toolchain.toml](./rust-toolchain.toml) 锁定的 `1.100.0` 为唯一标准，禁止自行切换工具链，禁止执行 `rustup update`。

### 2.2 构建
- 执行全量门禁时统一调用 `bash scripts/gate/all.sh`；日常模块开发默认使用 Debug 编译（`cargo build` / `cargo check`），未经明确要求不切换至 `--release`。
- 遵守项目 `CARGO_TARGET_DIR` 约定；磁盘空间紧张时优先清理 `target/debug/incremental` 与过期 `deps`、`build` 产物，禁止将 `target` 挪往外部路径绕过。
- 仅修改局部包时使用 `-p <crate>`（如 `cargo test -p rtkk`）按需编译验证。

### 2.3 格式与 Lint
- 修改代码后必须运行 `cargo fmt --all`，提交中不留任何格式差异。
- 必须运行 `cargo clippy --workspace --all-targets --locked -- -D warnings` 与 `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked`，所有告警全部清零。
- 严禁使用 `#[allow(...)]`、`#![allow(...)]`、`#[expect(...)]`、`#![expect(...)]`、`#[warn(...)]`、`allow(dead_code)` 或 crate 级 allow 压制告警；无用代码直接删除，禁止加 `_` 前缀保留。
- 全仓开启 `unsafe_code = "forbid"`、`as_conversions = "deny"`、`indexing_slicing = "deny"`。数值转换统一使用 `try_from` / `from`，集合访问统一使用 `.get()` / 迭代器模式。

### 2.4 错误处理与日志纪律
- 业务代码与集成测试均禁止使用 `unwrap()`、`expect()`、`panic!`、`todo!`、`unimplemented!`；错误统一通过 `?` 与结构化 `RtkkError` 向上传递（测试函数返回 `Result<(), String>`）。
- 禁止为讨好借用检查器盲目添加 `.clone()`、`Box`、`Arc` 或 `'static`，必须先厘清所有权与生命周期边界。
- 除终端交互白名单模块（`src/main.rs`、`src/cli.rs`、`src/logging/mod.rs`、`src/logging/progress.rs`、`src/playbook/engine.rs`）外，其余引擎与 `src/daemon/` 模块严禁裸写 `println!`、`eprintln!`、`dbg!`。

### 2.5 依赖与测试
- 新增任何依赖前必须先向用户说明原因及现有依赖无法满足的理由，同步登记至 [A6-DEPENDENCY-BASELINE.md](./docs/standards/A6-DEPENDENCY-BASELINE.md)；严格按需启用最小 `features`，禁止开启 `full` 全家桶；禁止随手改动 `Cargo.lock` 中无关依赖版本。
- 先运行单个测试或 `cargo test -p rtkk --test <target>`，通过后再跑 `cargo test --workspace --locked`。
- 单元测试置于同文件 `#[cfg(test)] mod tests`，集成测试置于 `test/*.rs`（在 `Cargo.toml` 显式声明 `[[test]]`），异步测试统一使用 `#[tokio::test]`。

---

## 3. Park UI 前端与文档微文案契约（Stage B `web/` 子栈）

初始化 `web/` 目录（Stage B Step 03）时同步写入 `web/AGENTS.md`，前端与全仓文档严格执行以下准则：

1. **纯离线与技术栈锁定**：React `19` + TypeScript `5.7`（`strict: true`）+ Vite `6` + `@ark-ui/react` + Tailwind CSS `v4`；使用系统字体栈与本地内联 SVG 图标，零外部在线请求。
2. **零绕过与零原生阻塞控件**：禁止 `any`、`@ts-ignore`、`@ts-expect-error`、`eslint-disable`；禁止原生 `<select>`、`alert()`、`confirm()`，交互原语统一由 `@ark-ui/react` 封装并保证完整键盘无障碍导航（`Tab` / `Arrow` / `Enter` / `Escape`）。
3. **双色 Token 与响应式四态**：样式统一消费 `tokens.css` 中的 `:root` 与 `[data-theme="dark"]` 语义变量，正文与主按钮对比度 `>= 4.5:1`；所有数值、IP、端口、体积、耗时列启用 `tabular-nums`；所有页面完备覆盖加载、空态、错误重试与正常四态，`<768px` 视口零横向溢出。
4. **极简产品微文案**：按钮、菜单与标签仅用 2–4 字简练动词/名词（如 `保存`、`校验`、`部署集群`、`取消` / `删除`）；表单占位符仅展示字面格式示例（如 `127.0.0.1:19912`）；文档坚持现态事实陈述，不写受众标签、修辞设问或迭代过程词。

---

## 4. 端口锁定、多代理隔离与门禁执行指令

- **独立默认端口**：Daemon 全局锁定端口 **`19912`**（宿主机默认 `127.0.0.1:19912`，Compose 容器映射 `${RTKK_PORT:-19912}:19912`）。多 `git worktree` 并发验证通过 `WT_SLOT=1..4` 自动映射至 `29912`、`39912`、`49912`、`59912`。
- **分队写入边界自检**：非 `orchestrator` 分队提交前必须运行 `bash scripts/check-write-scope.sh --role <role>`。
- **本地门禁标准命令序列**：
  ```bash
  source "$HOME/.cargo/env"
  # G1 静态守卫套件 (< 1s)
  python3 scripts/lint-guard.py
  python3 scripts/web-guard.py
  python3 scripts/configs-guard.py
  python3 scripts/docs-and-plans-guard.py
  python3 scripts/check_routes.py
  python3 scripts/check_traceability.py
  bash scripts/check-write-scope.sh --self-test
  # 一键执行 G1 + G2 + G3 全量激活门禁
  bash scripts/gate/all.sh
  ```

## 5. 完成汇报标准

`cargo fmt --all -- --check` 无差异、`cargo clippy` 与 `cargo doc` 零告警、G1 静态守卫与相关单测/门禁通过；向用户汇报结论时，必须附上实际执行的命令与真实终端输出摘要。
