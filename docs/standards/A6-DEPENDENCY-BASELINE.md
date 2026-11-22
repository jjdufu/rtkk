# A6 · 第三方依赖锁定基线与许可证准入清单

本文档登记 `rtkk` 当前已锁定的全部第三方依赖，以及 Stage B（Daemon + Park UI）预批核准的最小增量依赖集合。依据 [IRON_RULES.md](../../IRON_RULES.md) 铁律一，未在本表完成准入登记与许可证核验的组件严禁写入 `Cargo.toml` 或 `web/package.json`。

---

## 1. 许可证准入红线

- **允许准入**：`MIT`、`Apache-2.0`、`BSD-2-Clause`、`BSD-3-Clause`、`ISC`、`Unicode-3.0`、`Zlib`。
- **严禁引入**：`AGPL-3.0`、`GPL-2.0`、`GPL-3.0`、`SSPL-1.0`、`BUSL-1.1` 及一切带有网络服务源码传染性或商业竞争限制条款的许可证。

---

## 2. Phase 0 / Stage A 已锁定 Rust 依赖基线（`Cargo.toml` 共 27 项）

| Crate 名称 | 锁定版本 | 启用 Features | 开源许可证 | 核心用途 |
| :--- | :--- | :--- | :--- | :--- |
| `tokio` | `1.43` | `rt-multi-thread`, `macros`, `net`, `process`, `fs`, `sync`, `time`, `io-util`, `signal`, `io-std` | `MIT` | 异步多任务运行时、进程/文件 I/O 与系统信号处理 |
| `async-ssh2-lite`（别名 `ssh2-tokio`） | `0.5.0` | `tokio` | `MIT OR Apache-2.0` | 异步 SSH 会话、SFTP 流式传输与 `su` PTY 通道 |
| `serde` | `1.0` | `derive` | `MIT OR Apache-2.0` | Playbook、Inventory 与 API 模型序列化框架 |
| `serde_json` | `1.0` | `preserve_order` | `MIT OR Apache-2.0` | 模板上下文、OCI Manifest 与 JSONL 事件编解码 |
| `serde_yml` | `0.0.13` | 默认最小集 | `MIT OR Apache-2.0` | Inventory 与 Playbook YAML 解析与写出 |
| `gtmpl` | `0.7` | 默认最小集 | `MIT OR Apache-2.0` | Go 风格基础设施模板解析与求值引擎 |
| `gtmpl_value` | `0.5` | 默认最小集 | `MIT OR Apache-2.0` | `gtmpl` 动态值树与自定义辅助函数桥接 |
| `reqwest` | `0.12` | `json`, `socks`, `stream` | `MIT OR Apache-2.0` | HTTP 文件拉取与纯 Rust OCI Distribution v2 传输 |
| `bytes` | `1.0` | 默认最小集 | `MIT` | 网络流分块与零拷贝字节切片缓冲 |
| `futures-util` | `0.3` | 默认最小集 | `MIT OR Apache-2.0` | 异步字节流迭代与并发组合子 |
| `clap` | `4.6.1` | `derive` | `MIT OR Apache-2.0` | 强类型命令行参数与子命令解析器 |
| `thiserror` | `2.0` | 默认最小集 | `MIT OR Apache-2.0` | 结构化 `RtkkError` 错误派生 |
| `rcgen` | `0.14.7` | `pem`, `x509-parser` | `MIT OR Apache-2.0` | ECDSA P-256 X.509 根 CA 与叶证书签发 |
| `x509-parser` | `0.18` | 默认最小集 | `MIT OR Apache-2.0` | X.509 证书有效期、SAN、KeyUsage 与签名链解析校验 |
| `time` | `0.3` | `std` | `MIT OR Apache-2.0` | RFC 5280 99 年起止时间计算 |
| `sha2` | `0.10` | 默认最小集 | `MIT OR Apache-2.0` | 文件、OCI Manifest/Blob 与静态资产 SHA256 摘要 |
| `rand` | `0.8` | 默认最小集 | `MIT OR Apache-2.0` | 重试抖动、随机节点选择与 CSPRNG 高熵令牌生成 |
| `regex` | `1.11` | 默认最小集 | `MIT OR Apache-2.0` | 模板正则辅助函数与 `WWW-Authenticate` 挑战头解析 |
| `ipnet` | `2.10` | 默认最小集 | `MIT OR Apache-2.0` | CIDR 子网计算与 `generateIPsFromCIDR` 辅助函数 |
| `semver` | `1.0` | 默认最小集 | `MIT OR Apache-2.0` | 组件版本语义化比较（如 containerd v2/v3 自动切换） |
| `get_if_addrs` | `0.5` | 默认最小集 | `MIT OR BSD-3-Clause` | 本机网卡 IP 枚举（自动识别 `LocalConnector`） |
| `terminal_size` | `0.4` | 默认最小集 | `MIT OR Apache-2.0` | TTY 终端宽度探测以渲染自适应进度条 |
| `tracing` | `0.1` | 默认最小集 | `MIT` | 结构化诊断埋点 |
| `tracing-subscriber` | `0.3` | `registry`, `fmt` | `MIT` | 日志格式化与订阅层路由 |
| `tracing-appender` | `0.2` | 默认最小集 | `MIT` | 非阻塞日志文件追加器 |
| `tar` | `0.4` | 默认最小集 | `MIT OR Apache-2.0` | OCI `docker-legacy` 镜像归档读写与备份打包 |
| `shell-words` | `1.1` | 默认最小集 | `MIT OR Apache-2.0` | `command` 任务无 Shell 安全参数分词 |

---

## 3. Stage B 预批核准新增依赖基线

### 3.1 Rust Daemon 与静态内嵌新增依赖（Step 02 准入）

| Crate 名称 | 准入版本 | 启用 Features | 开源许可证 | 引入理由（存量依赖无法替代处） |
| :--- | :--- | :--- | :--- | :--- |
| `axum` | `0.8` | `json`, `tokio`, `http1`, `query` | `MIT` | 提供类型安全的 HTTP 路由树、中间件提取器与原生 SSE 响应流（`reqwest` 仅为客户端） |
| `tower-http` | `0.6` | `trace`, `set-header`, `limit` | `MIT` | 请求体大小保护、安全响应头与 `X-Request-Id` 注入中间件 |
| `subtle` | `2.6` | 默认最小集 | `BSD-3-Clause` | 管理令牌与会话签名的常量时间字节比对，防御时序侧信道探测 |
| `flate2` | `1.1` | `rust_backend` | `MIT OR Apache-2.0` | `build.rs` 编译期前端静态资源 Gzip 预压缩与 `rtkk_bak/` `.tar.gz` 流式归档压缩 |

### 3.2 Park UI 前端工程依赖基线（Step 03 / Step 07 准入，`web/package.json`）

| npm 包名称 | 准入版本范围 | 类别 | 开源许可证 | 核心用途 |
| :--- | :--- | :--- | :--- | :--- |
| `react` / `react-dom` | `^19.0.0` | `dependencies` | `MIT` | 前端视图渲染核心 |
| `@ark-ui/react` | `^5.0.0` | `dependencies` | `MIT` | Park UI 官方底层无头状态机原语（替代原生 `<select>` / 弹窗 / 抽屉 / 标签页） |
| `lucide-react` | `^0.475.0` | `dependencies` | `ISC` | Tree-shakable 本地内联 SVG 图标集（零外部图标字体请求） |
| `typescript` | `~5.7.0` | `devDependencies` | `Apache-2.0` | `strict: true` 编译期全量类型检查 |
| `vite` / `@vitejs/plugin-react` | `^6.1.0` | `devDependencies` | `MIT` | 离线静态单页打包器 |
| `tailwindcss` / `@tailwindcss/vite` | `^4.0.0` | `devDependencies` | `MIT` | 基于 CSS 变量的原子样式编译（零运行时开销） |
| `@types/react` / `@types/react-dom` | `^19.0.0` | `devDependencies` | `MIT` | React 19 类型声明 |
| `@playwright/test` | `^1.51.0` | `devDependencies` | `Apache-2.0` | Step 07 纯离线断网、双色对比度、键盘无障碍与响应式 E2E 门禁 |
