# 04 · Stage A · Step M4：Debian 13 CI 门禁合流与 Stage A 归档封板

本步骤将 Step M1～M3 建立的 G1/G2/G3 门禁接入 GitHub Actions Debian 13 容器流水线，并在 Stage A 全绿通过后完成 `docs/tmp/` 方案文件的正式归位（Stage A 迁入 `plans/completed/`，Stage B 迁入 `plans/phase1-daemon-webui-and-compose/`）。

---

## M4.1 目标

1. 升级 `.github/scripts/release/debian13-build.sh`，使每次 PR 与 `main` 推送在纯净 `debian:13` 容器内完整跑通 G1 静态守卫、G2（含 `RUSTDOCFLAGS='-D warnings' cargo doc`）与 G3 `step00.sh`。
2. 完成 `docs/tmp/` 的正式归类：
   - `01`～`04`（Stage A 迁移记录）归入 `plans/completed/phase0-cli-foundation/migration-records/`；
   - `00` 与 `05`～`11`（Stage B 实施方案）正式迁入 `plans/phase1-daemon-webui-and-compose/` 并清理空的 `docs/tmp/`。
3. 通过 GitHub Actions 远程 CI 验证全绿，正式封板 Stage A。

---

## M4.2 前置条件与负责角色

- **前置条件**：Step M1、M2、M3 已在本地通过 `bash scripts/gate/all.sh`。
- **负责角色**：`orchestrator`

---

## M4.3 文件变更清单

| 动作 | 路径 | 说明 |
| :--- | :--- | :--- |
| 修改 | `.github/scripts/release/debian13-build.sh` | 安装 `python3` 与 `jq`；在 `ci` 分支调用 `bash scripts/gate/all.sh`（覆盖 G1 + G2 + G3） |
| 归档迁移 | `docs/tmp/01..04*.md` → `plans/completed/phase0-cli-foundation/migration-records/` | 将已完成的 Stage A 迁移方案归档保存 |
| 正式立项 | `docs/tmp/{00,05..11}*.md` → `plans/phase1-daemon-webui-and-compose/` | 将 Stage B 蓝图转为正式推进阶段计划并更新 `plans/README.md` 索引 |

---

## M4.4 `debian13-build.sh` 升级规格

在 `.github/scripts/release/debian13-build.sh` 中：
1. `apt-get install -y --no-install-recommends` 列表补充 `python3` 与 `jq`（支撑 Python 3 标准库静态守卫与 `gate/lib.sh` JSON 断言）。
2. `MODE == "ci"` 执行序列统一对齐 `docs/QUALITY_GATES_AND_VERIFICATION.md`：
   ```bash
   # G1: Static Guards
   python3 scripts/lint-guard.py
   python3 scripts/web-guard.py
   python3 scripts/configs-guard.py
   python3 scripts/docs-and-plans-guard.py
   python3 scripts/check_routes.py
   python3 scripts/check_traceability.py
   bash scripts/check-write-scope.sh --self-test

   # G2: Rust Compile, Lint, Doc & Unit/Integration Tests
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
   cargo test --workspace --locked

   # G3: End-to-End Baseline Gate
   cargo build --locked
   bash scripts/gate/step00.sh
   ```

---

## M4.5 验收标准

1. 本地执行 `bash scripts/gate/all.sh` 末行输出 `ALL GATES PASS`。
2. `docs/tmp/` 已清空移除，所有方案文档已按动静分离规则归位至 `plans/completed/` 与 `plans/phase1-daemon-webui-and-compose/`，`python3 scripts/docs-and-plans-guard.py` 死链校验返回 `0`。
3. 推送分支后 GitHub Actions `CI` 作业在 `debian:13` 容器内全绿通过。
