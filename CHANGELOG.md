# Changelog

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
