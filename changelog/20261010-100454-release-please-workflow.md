# Release Please 发版工作流

## 背景

仓库还没有自动化发版。按 ggok 的 Release Please 流程补一套适合 rtkk 的二进制打包，打包用的 GitHub 托管 runner 镜像用当前最新正式版，不沿用 `ubuntu-latest`（仍是 24.04）和 `macos-14`。

## 代码

新增 CI、Release Please、四平台打包和 `Cargo.lock` 版本对齐工作流。`Cargo.toml` / `README.md` 的版本行加了 `x-release-please-version`，供 Release Please 改版本。没有改 Rust 业务代码。

打包 runner：

- Linux x86_64：`ubuntu-26.04`
- Linux aarch64：`ubuntu-26.04-arm`
- macOS Intel：`macos-26-intel`
- macOS Apple silicon：`macos-26`

工具链钉在当前 stable `1.99.0`。Linux 二进制链接 glibc 2.43，需要 Ubuntu 26.04 或同等环境。

## 数据库

无。

## 配置

新增 `release-please-config.json` 和 `.release-please-manifest.json`（当前版本 `0.1.0`，下一 patch 为 `0.1.1`）。1.0 前 feat/fix 都涨 patch。

推到 GitHub 的 `configs/inventory.yaml` 和 README 示例使用占位口令与 `10.0.0.0/8` 地址。本地 inventory 在推送后恢复，不进入这次提交。

仓库外新增 skill：`~/.pi/agent/skills/rtkk-github/`。

## 验证

```bash
bash -n .github/scripts/release/pack-release.sh
bash -n .github/scripts/install/install.sh
bash -n ~/.pi/agent/skills/rtkk-github/release.sh
python3 -m json.tool release-please-config.json
python3 -m json.tool .release-please-manifest.json
python3 .github/scripts/release/sync-workspace-lock-version.py
```

`sync-workspace-lock-version.py` 输出 `Cargo.lock already at 0.1.0`。没有在本机跑 GitHub Actions。

## 部署说明

推到 `main` 后，到 pulls 点两次 Approve and run workflows，绿了再 Merge pull request。合并后的二进制由 Release Please 调用 `release.yml` 打包，不要手打 tag，也不要对 `release.yml` 做 workflow_dispatch。

## 文件清单

- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `.github/workflows/release-please.yml`
- `.github/workflows/sync-lockfile.yml`
- `.github/scripts/release/pack-release.sh`
- `.github/scripts/release/sync-workspace-lock-version.py`
- `.github/scripts/install/install.sh`
- `release-please-config.json`
- `.release-please-manifest.json`
- `Cargo.toml`
- `README.md`
- `configs/inventory.yaml`
- `changelog/20261010-100454-release-please-workflow.md`

首次提交同时纳入既有源码：`src/`、`configs/`、`Cargo.lock`、`AGENTS.md`、`.gitignore`。
