# A5 · 测试夹具生成规范与 `scripts/gate/lib.sh` 公共库规格

本文档规定单元/集成测试与 G3 端到端门禁脚本的测试夹具构造准则，以及 [scripts/gate/lib.sh](../../scripts/gate/lib.sh) 公共函数签名契约。

---

## 1. 确定性测试夹具四项准则

1. **零仓库预置二进制与零静态私钥**：
   - 严禁向 Git 仓库提交预编译 `.tar`、测试私钥 `.key` 或二进制夹具；所有 X.509 根 CA/叶证书、模拟二进制与 OCI 镜像归档均由测试或门禁脚本在临时沙盒目录内即时动态生成。
2. **临时目录生命周期自闭环**：
   - Rust 集成测试统一使用带 `Drop` 自动清理的独立临时目录（含 PID 与纳秒时间戳后缀）；
   - G3 Shell 门禁统一在 `.gate/tmp-<step>-slot${WT_SLOT}/` 创建工作沙盒，并通过 `trap gate_cleanup EXIT INT TERM` 确保任何断言失败时可靠终止子进程（`rtkk serve`、Mock HTTP 服务）并清理临时目录。
3. **纯离线可复现模拟服务**：
   - 测试 OCI 镜像拉取/推送、Harbor v2 自动建项与 HTTP 文件下载时，统一在回环地址 `127.0.0.1:0`（内核随机空闲端口）启动轻量级本地内存/Python HTTP Mock 服务，不依赖公网 Registry 或外部互联网。
4. **跨平台 POSIX / macOS / Debian 13 兼容**：
   - 读取八进制权限位统一调用 `file_mode_octal <path>`（自动兼容 Linux GNU `stat -c '%a'` 与 macOS BSD `stat -f '%Lp'`）；
   - 计算 SHA256 摘要统一调用 `sha256_hex <path>`（自动兼容 `sha256sum` 与 `shasum -a 256`）。

---

## 2. `scripts/gate/lib.sh` 导出环境与公共函数规格

任何 `scripts/gate/stepNN.sh` 首行加载 `source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"` 后，自动获得以下隔离变量与辅助函数：

### 2.1 自动计算的槽位隔离变量

| 变量名 | 计算规则 | 示例（`WT_SLOT=0`） | 示例（`WT_SLOT=2`） |
| :--- | :--- | :--- | :--- |
| `REPO_ROOT` | 仓库绝对根路径 | `/path/to/rtkk` | `/path/to/rtkk-wt2` |
| `WT_SLOT` | `${WT_SLOT:-0}`（校验 `0..4` 整数） | `0` | `2` |
| `GATE_PORT` | `19912 + WT_SLOT * 10000` | `19912` | `39912` |
| `GATE_BIND_ADDR` | `127.0.0.1:${GATE_PORT}` | `127.0.0.1:19912` | `127.0.0.1:39912` |
| `GATE_BASE_URL` | `http://${GATE_BIND_ADDR}` | `http://127.0.0.1:19912` | `http://127.0.0.1:39912` |
| `GATE_WORK_DIR` | `${REPO_ROOT}/.gate/work-slot${WT_SLOT}` | `.gate/work-slot0` | `.gate/work-slot2` |
| `RTKK_BIN` | `${RTKK_BIN:-${REPO_ROOT}/target/debug/rtkk}` | `target/debug/rtkk` | `target/debug/rtkk` |

### 2.2 生命周期与断言函数签名

| 函数调用 | 功能与失败行为 |
| :--- | :--- |
| `gate_init "<step_name>"` | 初始化 `GATE_STEP`、重建干净的 `GATE_TMP_DIR`（`.gate/tmp-<step>-slot${WT_SLOT}`）与 `GATE_WORK_DIR`，注册 `EXIT/INT/TERM` 自动回收钩子 |
| `gate_register_pid <pid>` | 将后台守护进程（如 `rtkk serve` 或本地 Mock Registry）登记至清理队列，退出时优雅 `kill` 并回收 |
| `run_assertion "<assertion-name>" <fn_name>` | 执行指定断言函数；返回 `0` 打印 `PASS [<step>] <assertion-name>`，非 `0` 打印 `FAIL [<step>] <assertion-name>` 并以退出码 `1` 中止 |
| `assert_eq "<actual>" "<expected>" "<msg>"` | 精确字符串相等断言 |
| `assert_contains "<haystack>" "<needle>" "<msg>"` | 子串包含断言 |
| `assert_not_contains "<haystack>" "<needle>" "<msg>"` | 子串排除断言（用于验证敏感口令未泄漏至响应体） |
| `assert_file_exists "<path>"` | 断言目标普通文件存在 |
| `assert_file_mode "<path>" "<octal>"` | 断言文件八进制权限位精确匹配（如 `"600"` 或 `"700"`） |
| `http_status "<method>" "<url>" [extra_curl_args...]` | 发起 HTTP 请求并将响应体写入 `$GATE_HTTP_BODY`、响应头写入 `$GATE_HTTP_HEADERS`，输出三位状态码 |
| `json_field "<file_or_json>" "<jq_expr>"` | 调用 `jq -er` 提取结构化 JSON 字段 |
| `wait_daemon_ready "<health_url>" [timeout_secs]` | 轮询等待 `rtkk serve` 健康端点返回 `200`，超时打印守护进程日志并失败退出 |
| `gate_pass "<step_name>"` | 全部断言通过后输出标准收尾行 `GATE <step_name> PASS` |
