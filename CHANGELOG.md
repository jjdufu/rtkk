# Changelog

## [0.0.6](https://github.com/jjdufu/rtkk/compare/v0.0.5...v0.0.6) (2026-11-22)


### Features

* **cli,docs:** 新增 push-images --retag 参数并同步更新运维与契约文档 ([b16a51c](https://github.com/jjdufu/rtkk/commit/b16a51c79a484ae8d6354b34d2ae1f29c787b4f7))
* **configs,tests:** 支持公网镜像源 override_path 透明映射、CA 分发与 Harbor 建项测试 ([142e93d](https://github.com/jjdufu/rtkk/commit/142e93df0c55b861f25965eb07ef61a028926064))
* **daemon,web:** 交付 Stage B Step 01-06 常驻服务、内嵌 Park UI 与全量管理接口 ([5d5adac](https://github.com/jjdufu/rtkk/commit/5d5adacd8b800c9be06557f51251c28d3387c3d9))
* **daemon:** 补齐 Inventory 证书有效期预校验、私钥存在性告警与私库重标推送参数 ([e81bba8](https://github.com/jjdufu/rtkk/commit/e81bba8e2ba1fb1ba3c21f942b885d209c01590e))
* **deploy,e2e:** add Debian 13 container stack, compose lifecycle scripts, and offline Park UI Playwright E2E ([c8ee0e4](https://github.com/jjdufu/rtkk/commit/c8ee0e4dc3301cca7c5ea4b02746a403ec5757a3))
* **engine,registry,ssh:** 支持私钥波浪号展开、证书小时规范化与跨私库镜像自动重标推送 ([d4ba1a2](https://github.com/jjdufu/rtkk/commit/d4ba1a212eef3da0c13e586acd893132565dd3c3))
* **engine,registry:** 新增 ensure_harbor_projects 模块与公网镜像源透明映射项目自举 ([7fb158f](https://github.com/jjdufu/rtkk/commit/7fb158f9967fa114f9cba27eb05f13fbcb7d9319))
* **web:** expose SSH key/port, public registry auth, Cilium KPR/L2, custom cert validity, and retag push ([02d106b](https://github.com/jjdufu/rtkk/commit/02d106b6bc2b546716995bfd4ad39e6e7838514e))
* **web:** 在集群配置页新增公网镜像源 override_path 透明映射与 Harbor 自动建项开关 ([6ad2f32](https://github.com/jjdufu/rtkk/commit/6ad2f3211fbf23043ecd3cf0237ce7df01532532))


### Bug Fixes

* **compose:** 补齐 svc_start 当前进程 load_compose_env 变量加载 ([bb70555](https://github.com/jjdufu/rtkk/commit/bb705558b14fe94b45a04eac688249e80eccdbf4))
* **configs:** harden public registries, cilium L2/CRD readiness, cert validity, and inventory hygiene ([cefbcd4](https://github.com/jjdufu/rtkk/commit/cefbcd4e2e9c059fcc14b01977c2828ad8af7b8f))
* **configs:** 修复 deploy.yaml 控制面条件寄存器短路求值保护 ([76889da](https://github.com/jjdufu/rtkk/commit/76889da57dd5a4c12d0a461a3c1682183be8eed5))
* **deploy,gate:** harden compose common.sh env parser and symlink guards per L3 review ([5063fa0](https://github.com/jjdufu/rtkk/commit/5063fa0a165ae2dd1e9d775d3739fde7b67dc633))
* **gate:** 对齐 vps-cluster-e2e 沙盒工作目录与离线 downloads 挂载 ([03ffb9f](https://github.com/jjdufu/rtkk/commit/03ffb9f883dee660a913f53914c5e122902619bf))

## [0.0.5](https://github.com/jjdufu/rtkk/compare/v0.0.4...v0.0.5) (2026-11-22)


### Features

* **configs:** 新增 Debian 13 节点预检、依赖包分级收敛与内核基线配置 ([253dfbb](https://github.com/jjdufu/rtkk/commit/253dfbb67fdb5c3503e0742912bd42d5d2ce7aa7))

## [0.0.4](https://github.com/jjdufu/rtkk/compare/v0.0.3...v0.0.4) (2026-11-22)


### Features

* **connector,configs:** 新增普通用户 su 密码提权支持并保护 prep hosts 主机名映射 ([050d59f](https://github.com/jjdufu/rtkk/commit/050d59fd48336e339313dee9430d564132606b94))

## [0.0.3](https://github.com/jjdufu/rtkk/compare/v0.0.2...v0.0.3) (2026-11-21)


### Features

* **configs:** 升级 K8s 至 v1.37.2 生产版本矩阵并解耦可插拔 CNI/插件编排 ([169ce46](https://github.com/jjdufu/rtkk/commit/169ce46c54123a87f1994437c159a089d780f184))

## [0.0.2](https://github.com/jjdufu/rtkk/compare/v0.0.1...v0.0.2) (2026-11-13)


### Features

* **configs:** auto-switch containerd config.toml between v2 (1.x) and v3 (2.x) ([e384026](https://github.com/jjdufu/rtkk/commit/e384026b24a5ee125fd8481e73674a6c6d35f1ea))

## [0.0.1](https://github.com/jjdufu/rtkk/compare/v0.0.0...v0.0.1) (2026-11-13)


### Features

* **core,cert,connector:** add configurable 99y-default PKI, sudo/host-key SSH support, and deterministic inventory ([6662f46](https://github.com/jjdufu/rtkk/commit/6662f46adeea909d1fdccfa7313fd0111ad910d7))
* **playbook,tasks,cli:** enforce Rust 1.100.0 strict workspace lints, cert CLI, and hardened task/engine execution ([b4310e8](https://github.com/jjdufu/rtkk/commit/b4310e8af19875d8fe8abd930357d00a6a93b6f7))


### Bug Fixes

* **cli,tasks:** support state policy alias in gen_cert and cert CLI flag aliases ([ddc8c23](https://github.com/jjdufu/rtkk/commit/ddc8c23feffce66954f798f445ef074ac9b4a67e))
* **configs:** harden offline K8s playbooks for idempotency, HA kube-vip ordering, and dynamic Cilium PKI ([b8aef19](https://github.com/jjdufu/rtkk/commit/b8aef19887f3ecc6908a8e02d0664f518bd0c2c9))
* **template,registry:** add push Bearer auth, Harbor visibility config, digest verification, and NoValue helpers ([e749ce2](https://github.com/jjdufu/rtkk/commit/e749ce20fc9daa4f0d690eb8c91a963ea879187b))

## 0.0.0 (2026-10-10)


### Features

* import rtkk 0.0.0 with Debian 13 x86_64 releases ([a4f4df1](https://github.com/jjdufu/rtkk/commit/a4f4df1d66fac7576970355293469cc89e0b3183))
