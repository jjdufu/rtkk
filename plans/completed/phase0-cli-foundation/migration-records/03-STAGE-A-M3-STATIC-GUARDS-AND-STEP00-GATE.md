# 03 · Stage A · Step M3：G1 静态守卫套件与 G3 `step00.sh` 基线门禁

本步骤将 `IRON_RULES.md` 与 `docs/QUALITY_GATES_AND_VERIFICATION.md` 转化为可执行、零第三方依赖（仅使用 Bash 与 Python 3 标准库）的自动化拦截脚本，并落地首个端到端门禁脚本 `scripts/gate/step00.sh` 与全量入口 `scripts/gate/all.sh`。

---

## M3.1 目标

1. 实现全部 7 个 G1 静态守卫脚本，单次全套运行耗时 `< 1s`。
2. 实现 `scripts/gate/lib.sh` 公共断言库，支持 `WT_SLOT` 并发槽端口与临时目录自动计算。
3. 实现 `scripts/gate/step00.sh`，对真实编译出的 `rtkk` 二进制执行离线全功能冒烟验收（版本、99 年/1 年 PKI 签发与权限校验、本地 Playbook 幂等运行、错误输入快速失败断言）。
4. 实现 `scripts/gate/all.sh`，串联 G1 → G2 → G3，末行输出 `ALL GATES PASS`。

---

## M3.2 前置条件与负责角色

- **前置条件**：Step M1、Step M2 已完成。
- **负责角色**：`orchestrator`（`scripts/*.py`、`check-write-scope.sh`、`lib.sh`、`all.sh`）+ `squad-ops-qa`（`step00.sh`）

---

## M3.3 文件变更清单

| 动作 | 路径 | 说明 |
| :--- | :--- | :--- |
| 新建 | `scripts/lint-guard.py` | Rust 工具链版本、`Cargo.toml` lint 锁表及 `.rs` 源码零豁免扫描 |
| 新建 | `scripts/web-guard.py` | 前端 TS Strict、Park UI 无原生 `<select>`/`alert()`/`confirm()`、无外部 CDN 与极简微文案扫描 |
| 新建 | `scripts/configs-guard.py` | `configs/` 与 `deploy/` YAML 语法、八进制 `mode` 字符串引号、`0600` 私库凭据权限与占位口令卫生扫描 |
| 新建 | `scripts/docs-and-plans-guard.py` | 全仓 Markdown 相对路径死链核验、记忆文件 `< 24,000` 字节体积核验与文档规范扫描 |
| 新建 | `scripts/check_routes.py` | `src/daemon/routes/**/*.rs` Axum 路由树与 `docs/standards/A4-API-CONTRACT-INDEX.md` 双向比对 |
| 新建 | `scripts/check_traceability.py` | `docs/standards/A7-REQUIREMENTS-TRACEABILITY.md` 与 `scripts/gate/step*.sh` 断言名核对 |
| 新建 | `scripts/check-write-scope.sh` | 分队写入范围白名单核验（支持 `--self-test` 自检模式） |
| 新建 | `scripts/gate/lib.sh` | 门禁公共函数库（断言、HTTP/JSON 校验、文件权限读取、临时沙盒自动清理） |
| 新建 | `scripts/gate/step00.sh` | Phase 0 CLI 核心基线端到端门禁脚本 |
| 新建 | `scripts/gate/all.sh` | 一键运行 G1 + G2 + G3 全量门禁总入口 |

---

## M3.4 各守卫脚本详细拦截规格

### M3.4.1 `scripts/lint-guard.py`
1. 读取 `rust-toolchain.toml` 与 `Cargo.toml`，断言两者均精确锁定 `1.100.0`，且 `edition = "2024"`。
2. 断言 `Cargo.toml` 中 `unsafe_code = "forbid"`，且 `warnings`、`missing_docs`、`clippy::all`、`clippy::pedantic`、`allow_attributes`、`unwrap_used`、`expect_used`、`todo`、`unimplemented`、`panic`、`as_conversions`、`indexing_slicing` 全部保持 `"deny"`。
3. 剥离行注释、块注释与字符串字面量后扫描 `src/**/*.rs`、`test/**/*.rs`、`build.rs`：
   - 拦截任何 `unsafe` 关键字、`#[allow`、`#![allow`、`#[expect`、`#![expect`、`#[warn`、`dbg!`；
   - 扫描 `src/` 下除终端交互白名单文件（`src/main.rs`、`src/cli.rs`、`src/logging/mod.rs`、`src/logging/progress.rs`、`src/playbook/engine.rs`）之外的所有模块（尤其是未来 `src/daemon/`），拦截裸 `println!` 与 `eprintln!`。

### M3.4.2 `scripts/web-guard.py`
- 若 `web/src` 目录尚未创建（Stage A 期间），输出 `ok  web-guard (web/ not initialized yet)` 并返回退出码 `0`。
- 当 `web/src` 存在时：
  1. 拦截 `: any`、`as any`、`<any>`、`@ts-ignore`、`@ts-expect-error`、`eslint-disable`；
  2. 拦截 `<select`、`window.alert(`、`alert(`、`window.confirm(`、`confirm(`、`window.prompt(`；
  3. 拦截 `web/index.html` 与 `web/src/**/*` 中的 `http://` / `https://` 外部样式、字体或脚本链接（如 `fonts.googleapis.com`、`fonts.gstatic.com`、`cdn.jsdelivr.net`、`unpkg.com`）；
  4. 校验全局 CSS 包含 `[hidden] { display: none !important; }` 以及 `:root` 和 `[data-theme="dark"]` 双模式 Token 定义，且 `[data-theme="dark"]` 中 `--accent` 与 `--accent-fg` 不相等、具备明暗反转对比度；
  5. 扫描按钮与菜单文本，拦截包含 `点击`、`点此`、`选择`、`Click to`、`Press` 的违规交互赘词。

### M3.4.3 `scripts/configs-guard.py`
1. 校验 `configs/` 下所有 `.yaml` 文件结构完整（无制表符缩进污染）。
2. 正则核验所有任务模块中的 `mode:` 字段值均以单引号或双引号包裹为字符串（如 `mode: "0600"` 或 `mode: "0755"`），严禁裸写无引号八进制数字（如 `mode: 0644`）。
3. 核验 `configs/deploy.yaml` 中私库 `hosts.toml` 写入任务显式声明 `mode: "0600"`。

### M3.4.4 `scripts/docs-and-plans-guard.py`
1. 校验 `IRON_RULES.md`、`AGENTS.md`、`GEMINI.md`、`CLAUDE.md`（及存在的 `web/AGENTS.md`）单文件字节大小严格 `< 24,000`。
2. 解析全仓所有 `.md` 文件（排除代码块内容）中的相对文件链接（形如 `[label](path.md)`），校验目标文件真实存在于工作区。
3. 扫描全仓文档与注释，拦截外部无关项目的专有残留词以及违反现态陈述规范的历史版本过程词汇。

### M3.4.5 `scripts/check_routes.py` 与 `scripts/check_traceability.py`
- `check_routes.py`：提取 `src/daemon/routes/**/*.rs` 中注册的全部 `(METHOD, /api/v1/...)` 对，与 `docs/standards/A4-API-CONTRACT-INDEX.md` 表格逐条双向集合比对；Stage A 两端集合均为空集（通过），Stage B 任一端多出或缺失路由均返回非 `0`。
- `check_traceability.py`：解析 `docs/standards/A7-REQUIREMENTS-TRACEABILITY.md` 中格式为 ``stepNN `"assertion-name"` `` 的条目，核验对应 `scripts/gate/stepNN.sh` 中真实存在该断言名调用。

### M3.4.6 `scripts/gate/step00.sh` 核心验收断言
在自动清理的临时沙盒目录（`.gate/tmp-step00/`）内执行：
1. `"cli-version"`：`rtkk --version` 输出匹配 `Cargo.toml` 当前版本号。
2. `"cert-99y-and-verify"`：执行 `rtkk cert gen` 生成根 CA 与带 IP/DNS SAN 的叶证书，核验私钥文件权限为 `600`（兼容 Linux `stat -c '%a'` 与 macOS `stat -f '%Lp'`），且 `rtkk cert verify` 链校验返回 `0`。
3. `"cli-local-playbook"`：构造包含 `copy`、`template`、`shell`（带 `register` 与 `changed_when`）的最小本地 Playbook 与 Inventory，连续运行两次 `rtkk run`——首次断言文件生成且内容正确，二次运行断言幂等 `ok`（零失败）。
4. `"fail-fast-bad-inv"`：对包含循环组依赖或非法 `--forks 0` 的输入执行 `rtkk`，断言返回非 `0` 退出码并输出明确诊断信息。
5. 末行调用 `gate_pass step00`，打印 `GATE step00 PASS`。

---

## M3.5 验收标准

本地依次执行以下命令全部返回退出码 `0`：

```bash
source "$HOME/.cargo/env"
python3 scripts/lint-guard.py
python3 scripts/web-guard.py
python3 scripts/configs-guard.py
python3 scripts/docs-and-plans-guard.py
python3 scripts/check_routes.py
python3 scripts/check_traceability.py
bash scripts/check-write-scope.sh --self-test
bash scripts/gate/all.sh
```

且 `bash scripts/gate/all.sh` 标准输出最后一行精确为 `ALL GATES PASS`。
