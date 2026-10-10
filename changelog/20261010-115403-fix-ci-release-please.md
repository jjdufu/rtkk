# 修好首次 CI 和 Release Please 失败

## 背景

GitHub Actions 第一次跑失败了。CI 在 Debian 13 里被 `ldd | head` 的 SIGPIPE 打断，过了这一步后又被 Rust 1.99 的 `clippy::doc_markdown` 拦住。Release Please 已经开出 release PR，但后面查 `releases/latest` 得到 404，把整次工作流打失败了。

## 代码

`src/playbook/helpers.rs` 的文档注释给 `host_vars.name` 加上反引号，不用 `allow`。`debian13-build.sh` 不再把 `ldd` 接到 `head`。`release-please.yml` 把没有 Release 的 404 当成“没有可打包的版本”，不再让工作流失败。

## 数据库

无。

## 配置

无。

## 验证

```bash
source "$HOME/.cargo/env"
rustup run 1.99.0 cargo clippy --workspace --all-targets --locked -- -D warnings
rustup run 1.99.0 cargo test --workspace --locked
bash -n .github/scripts/release/debian13-build.sh
```

clippy 通过，测试 0 failed。没有在本机重跑 GitHub Actions。

## 部署说明

推到 main 后，CI 和 Release Please 会再跑。到 pulls 点两次 Approve and run workflows，绿了再 Merge pull request。

## 文件清单

- `src/playbook/helpers.rs`
- `.github/scripts/release/debian13-build.sh`
- `.github/workflows/release-please.yml`
- `changelog/20261010-115403-fix-ci-release-please.md`
