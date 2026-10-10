# Rust 项目规则
本文件只写 Rust 专有的规则，通用规则见 ~/.pi/agent/AGENTS.md；两者冲突时以本文件为准。

## 工具链
cargo、rustc、rustup 一般通过 rustup 装在当前用户的 $HOME/.cargo/bin/ 下。代理用的 shell 不加载 ~/.bashrc 和 ~/.profile，不要假定这些命令已经在 PATH 里；先执行 source "$HOME/.cargo/env"，或者用 "$HOME/.cargo/bin/cargo" 这样的全路径。
不要在规则、脚本或配置里写死某个用户的家目录，一律用 $HOME 或 ~。
仓库里有 rust-toolchain.toml 的，以它指定的版本为准，不要自己切换工具链，也不要随手 rustup update。

## 构建
项目有自己的构建脚本（build.sh、Makefile、justfile、xtask 等）时，一律走它，不要另起 cargo build。没有的话，再用 cargo build。
默认用 debug 编译，没经要求不要切到 release。
项目规定了 CARGO_TARGET_DIR 的，照它来；磁盘空间不够时，先清 target/debug/incremental 和明显过期的 deps、build 产物，不要把 target 挪到别处来绕过。
只改了个别 crate 时，用 -p <crate> 只编译受影响的包，不要每次全量编译整个 workspace。

## 格式与 lint
改完代码要跑 cargo fmt，提交里不留格式差异。
跑 cargo clippy --all-targets，所有告警都要修掉；项目在 CI 里用了 -D warnings 的，本地也照这个标准来。
不许用 #[allow(...)]、#![allow(...)]、当永久静音用的 #[expect(...)]、allow(dead_code) 或 crate 级的整体 allow 来压告警。没用的代码直接删，不要改名成 _ 留着。
确实是 clippy 误报的，按通用规则里的例外流程处理。

## 错误处理
业务代码里不要用 unwrap() 和 expect() 糊弄过去，错误要用 ? 往上传，或者正经处理掉。测试代码和确定不会失败的地方可以用，但要写清楚为什么不会失败。
不要为了让编译器闭嘴随手加 clone()、Box、Arc 或 'static，先弄清楚所有权和生命周期的问题出在哪里。
unsafe 只在确实需要时用，每一处都要写 // SAFETY: 注释，说明为什么是安全的。

## 依赖
加新依赖前先说一声，并说明为什么需要它、为什么现有的依赖做不到。
尽量用 workspace 里已经统一的版本（[workspace.dependencies]），不要在各个 crate 里各写各的版本号。
不要随手改 Cargo.lock 里跟本次改动无关的依赖版本；要升级依赖，单独做一次改动。
只开需要的 feature，不要图省事开 full 或 default 全家桶。

## 测试
先跑 cargo test -p <crate> 或单个测试，通过后再跑 cargo test --workspace。
单元测试写在同一个文件的 #[cfg(test)] mod tests 里，集成测试放在 tests/ 下，按项目已有的习惯来。
异步代码的测试用项目统一的运行时（比如 #[tokio::test]），不要自己混用别的运行时。

## 完成标准
cargo fmt 没有差异，clippy 没有告警，相关测试通过；说这些结论时，附上实际执行的命令和输出。

