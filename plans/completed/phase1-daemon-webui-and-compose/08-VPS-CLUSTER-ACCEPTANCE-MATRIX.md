# 08 · Stage B 实机联调专案：Debian 13 虚拟 VPS 多节点集群与 Harbor 全链路验收矩阵

本文档定义当 Debian 13（`Trixie`）虚拟 VPS 主机准备就绪后，针对 `rtkk` CLI 与 Daemon（`:19912`）开展真实跨机 SSH / `sudo` / `su` 提权、私有 Harbor 镜像同步、DNS 劫持透明镜像缓存及离线高可用 Kubernetes `v1.37.2` 部署的实机验收方案，配套自动化门禁脚本为 [scripts/gate/vps-cluster-e2e.sh](../../../scripts/gate/vps-cluster-e2e.sh)。

---

## 1. 虚拟 VPS 推荐拓扑规划

| 角色分组 | 推荐台数 | 最低规格（每台） | 操作系统与基础要求 | 验收职责 |
| :--- | :---: | :--- | :--- | :--- |
| **控制与镜像服务机（Control / Harbor）** | 1 | 2 vCPU / 4 GiB / 40 GiB | Debian 13 `x86_64` + Docker / Compose | 运行 `deploy/compose/`（`:19912`）与私有 Harbor v2（或 `registry:2`） |
| **控制面与 etcd 节点（`kube_control_plane` + `etcd`）** | 3（或单机 1） | 2 vCPU / 4 GiB / 30 GiB | 纯净 Debian 13 `x86_64` + `openssh-server` | 验证外部 etcd 集群、`kube-vip` ARP VIP 高可用与控制面扩容 |
| **计算工作节点（`kube_node`）** | 1 ～ 3 | 2 vCPU / 4 GiB / 30 GiB | 纯净 Debian 13 `x86_64` + `openssh-server` | 验证 Worker 加入、Cilium 跨节点 Pod 互通、OpenEBS LocalPV 与 NodeLocalDNS |

---

## 2. 虚拟 VPS 就绪后的前置初始化清单

1. **准备实机专用 Inventory（切勿提交至 Git）**：
   - 将 `configs/inventory.yaml` 复制至 `.gate/vps-inventory.yaml`（已被 `.gitignore` 忽略），填入虚拟 VPS 真实内网 IP、VIP 地址、主网卡名（如 `eth0` / `ens3` / `ens18`）及测试私库凭据。
2. **覆盖三类节点权限模型抽检**：
   - 至少 1 台节点验证 `root` + Ed25519 公钥直连；
   - 至少 1 台节点验证普通账号 + `ansible_become: true` + `ansible_become_method: "sudo"`；
   - （可选）在禁改 `/etc/sudoers` 的节点验证 `ansible_become_method: "su"` PTY 帧隔离提权。
3. **准备离线制品目录**：
   - 在联网准备环境执行一次 `./target/release/rtkk run configs/download.yaml -i .gate/vps-inventory.yaml -l logs/vps-download.log`（或通过 WebUI `/artifacts` 页面触发预下载），就绪 `downloads/{bin,tar,images}`。

---

## 3. 八阶段实机闭环验收流程（`vps-cluster-e2e.sh`）

当环境变量 `RTKK_VPS_INVENTORY=.gate/vps-inventory.yaml` 指向已就绪的虚拟 VPS 清单时，执行 `bash scripts/gate/vps-cluster-e2e.sh` 按顺序完成以下八项实机门禁断言：

| 序号 | 断言名称 | 执行动作与核验标准 |
| :---: | :--- | :--- |
| 1 | `"vps-compose-daemon-up"` | 在控制机通过 `deploy/compose/scripts/start.sh` 拉起 `rtkk-daemon` 容器（`:19912`），校验 `rtkk healthcheck` 返回 `0` 且挂载卷属主为 `10001:10001` |
| 2 | `"vps-ssh-and-become-probe"` | 调用 `POST /api/v1/inventory/probe` 并发探测全部 VPS 节点，断言每台节点 `reachable == true`、提权后 `uid == 0`、系统识别为 `Debian GNU/Linux 13` |
| 3 | `"vps-registry-ping-and-push"` | 调用 `POST /api/v1/registry/ping` 验证私库握手成功，随后触发 `POST /api/v1/registry/push` 将本地 `downloads/images/*.tar` 全量推送至 Harbor 并等待任务 `succeeded` |
| 4 | `"vps-prep-hosts-idempotent"` | 触发 `POST /api/v1/operations/prep-hosts` 同步全集群 `/etc/hosts` `# RTKK HOSTS START..END` 块，并校验各节点原 OS 主机名未被篡改 |
| 5 | `"vps-preflight-and-dns-hijack"` | 依次触发 `deploy` 的 `--tags check`（只读体检）与 `--tags preflight`（离线 `.deb`/APT 依赖收敛、关闭 Swap、加载内核模块、`sysctl` 与 `iptables-nft` 对齐） |
| 6 | `"vps-ha-k8s-deploy-ready"` | 触发完整 `deploy.yaml` 异步部署，通过 SSE 流跟踪 Play 0～9 全部完成；在首台控制面验证所有节点处于 `Ready` 状态，Cilium、CoreDNS、NodeLocalDNS、OpenEBS Pod 全部 `Running` |
| 7 | `"vps-pki-and-backup-export"` | 核验目标节点 `/etc/containerd/certs.d/<reg>/hosts.toml` 权限精确为 `600`；调用 `GET /api/v1/backups/archive` 流式下载状态备份包并确认内含有效 `admin.conf` |
| 8 | `"vps-cleanup-and-redeploy-option"` | 当设置 `RTKK_VPS_RUN_CLEANUP=1` 时，触发 `cleanup.yaml` 验证集群干净卸载重置（服务停止、端口释放、目录清空） |

---

## 4. 启动命令参考（待虚拟 VPS 主机就绪后人工触发）

```bash
source "$HOME/.cargo/env"
export RTKK_VPS_INVENTORY="$PWD/.gate/vps-inventory.yaml"
bash scripts/gate/vps-cluster-e2e.sh
```
