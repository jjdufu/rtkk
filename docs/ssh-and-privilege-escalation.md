# SSH 认证、非 Root 提权（`become`/`sudo`）与主机指纹校验指南

> [!NOTE]
> 本文档是 [运维准备与安全建议手册](../OPERATION_GUIDE.md) 的配套技术指南，详细说明 Debian 13 (`Trixie`) 下的 `root` 密钥直连基线、普通账号 `sudo`/`become` 提权配置，以及 SSH 主机指纹校验（`known_hosts`）的实操流程。

---

## 目录

- [1. 默认模式：Debian 13 `root` SSH 部署与加固提醒](#1-默认模式debian-13-root-ssh-部署与加固提醒)
- [2. 进阶模式：创建 Debian 13 非 Root 部署账号与配置 `sudoers`](#2-进阶模式创建-debian-13-非-root-部署账号与配置-sudoers)
- [3. 在 `rtkk` 中配置 `become` 提权（Inventory / Play / Task）](#3-在-rtkk-中配置-become-提权inventory--play--task)
- [4. SSH 主机指纹校验（`HostKeyPolicy` 与 `known_hosts` 管理）](#4-ssh-主机指纹校验hostkeypolicy-与-known_hosts-管理)

---

## 1. 默认模式：Debian 13 `root` SSH 部署与加固提醒

### 1.1 `rtkk` 的 SSH 认证优先级

`rtkk` 的远端连接器（`SshConnector`）建立 SSH 会话时，按以下确定性顺序协商身份认证：

1. **显式私钥内容**：当提供内联私钥文本时优先使用。
2. **显式私钥文件路径（`ansible_ssh_private_key_file`）**：运行时自动展开前导 `~/` 至 `$HOME/`；当指定路径不存在或认证失败时默认**立即返回包含展开后路径的明确错误并中止**（仅当显式设置 `ansible_ssh_private_key_fallback_password: true` 时才允许回退密码），防止密钥路径配置错误时静默降级为弱口令尝试。
3. **默认用户目录密钥探测**：未指定显式密钥路径时，依次探测 `$HOME/.ssh/id_ed25519`、`$HOME/.ssh/id_ecdsa`、`$HOME/.ssh/id_rsa`。
4. **SSH 密码认证（`ansible_ssh_pass`）**：作为无可用密钥或显式开启回退时的认证方式。

### 1.2 临时口令引导 vs Ed25519 密钥登录（`PermitRootLogin prohibit-password`）

Kubernetes 集群安装涉及内核模块加载（`modprobe`）、内核网络参数持久化（`sysctl`）、系统服务管理（`systemctl`）以及 `/etc/kubernetes`、`/etc/containerd`、`/opt/cni/bin` 等特权目录写入。在内网独立部署环境中，采用 `root` + **Ed25519 公钥认证** 是最简洁可靠的默认方式：

- **临时口令引导提醒**：仅建议在初始化裸机并首次执行 `ssh-copy-id` 分发公钥时短暂使用密码登录。公钥验证通过后，应立即从 `configs/inventory.yaml` 中移除明文 `ansible_ssh_pass`。
- **Debian 13 `sshd` 安全基线**：Debian 13 (`Trixie`) 默认的 `openssh-server` 配置已启用 `PermitRootLogin prohibit-password`（即允许 `root` 使用公钥登录，拒绝 `root` 密码登录）。为进一步关闭所有账号的 SSH 密码暴力猜测面，可在确认密钥登录正常后，在各节点写入独立加固配置片段：

```bash
# ⚠️ 执行前请务必在另一个终端确认 ssh -i ~/.ssh/rtkk_ed25519 root@10.0.0.61 免密登录已成功！
cat <<'EOF' > /etc/ssh/sshd_config.d/99-rtkk-hardening.conf
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
EOF

# 校验 sshd 语法并平滑重载 Debian 13 的 ssh.service
sshd -t && systemctl reload ssh
```

---

## 2. 进阶模式：创建 Debian 13 非 Root 部署账号与配置 `sudoers`

在启用了安全基线审计（要求 `PermitRootLogin no`，禁止任何形式的 `root` 直接 SSH 会话）的企业环境中，可在目标节点创建专用普通账号（如 `deploy`），由 `rtkk` 登录后通过 `sudo`（`become`）提权完成部署。

### 2.1 在 Debian 13 节点创建 `deploy` 账号并注入公钥

> [!TIP]
> Debian 13 安装器若在安装阶段设置了 `root` 密码，默认可能未预装 `sudo` 软件包。请先确认安装 `sudo`。

在各目标节点以 `root` 执行以下初始化脚本（在禁用 `root` SSH 登录前完成）：

```bash
# 1) 确保已安装 sudo
apt update && apt install -y sudo

# 2) 创建普通部署账号 deploy（使用 bash 作为登录 shell）
id -u deploy >/dev/null 2>&1 || useradd -m -s /bin/bash deploy

# 3) 为 deploy 配置强口令（供带密码 sudo 或控制台应急登录使用）
printf 'deploy:%s\n' '<STRONG_PASSWORD>' | chpasswd

# 4) 初始化 ~deploy/.ssh 目录并写入控制机公钥（权限严格设为 0700 / 0600）
DEPLOY_HOME="$(getent passwd deploy | cut -d: -f6)"
install -d -m 0700 -o deploy -g deploy "${DEPLOY_HOME}/.ssh"
cat <<'EOF' > "${DEPLOY_HOME}/.ssh/authorized_keys"
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI... rtkk-deploy@control-host
EOF
chown deploy:deploy "${DEPLOY_HOME}/.ssh/authorized_keys"
chmod 0600 "${DEPLOY_HOME}/.ssh/authorized_keys"
```

### 2.2 配置 `/etc/sudoers.d/deploy` 提权策略

根据组织的审计要求，可从以下三种 `sudoers` 策略中择一写入 `/etc/sudoers.d/deploy`，并用 `visudo -cf` 校验语法：

#### 方案 A：免密 `NOPASSWD` 提权（推荐用于无人值守流水线）

配合高强度 Ed25519 密钥登录且完全禁用 SSH 密码登录时，免密 `sudo` 避免了在配置或参数中传递提权密码：

```bash
cat <<'EOF' > /etc/sudoers.d/deploy
Defaults:deploy !requiretty
deploy ALL=(root) NOPASSWD: ALL
EOF
chmod 0440 /etc/sudoers.d/deploy
visudo -cf /etc/sudoers.d/deploy
```

#### 方案 B：口令保护的 `sudo` 提权（配合 `ansible_become_pass`）

若安全规范要求每次提权必须二次验证用户口令：

```bash
cat <<'EOF' > /etc/sudoers.d/deploy
Defaults:deploy !requiretty
deploy ALL=(root) ALL
EOF
chmod 0440 /etc/sudoers.d/deploy
visudo -cf /etc/sudoers.d/deploy
```

#### 方案 C：按命令集合收敛的最小权限 `sudoers`

若执行日常运维巡检或定制的受限 Playbook（不跑任意 `/bin/sh -c` 脚本，仅使用特定模块如 `systemd`、`sysctl`、`modprobe` 等），可通过 `Cmnd_Alias` 限制可执行程序路径：

```bash
cat <<'EOF' > /etc/sudoers.d/deploy
Defaults:deploy !requiretty
Cmnd_Alias RTKK_OPS = /usr/bin/systemctl *, /usr/sbin/sysctl *, /usr/sbin/modprobe *, /usr/bin/kubeadm *, /usr/bin/kubectl *, /usr/bin/crictl *
deploy ALL=(root) NOPASSWD: RTKK_OPS
EOF
chmod 0440 /etc/sudoers.d/deploy
visudo -cf /etc/sudoers.d/deploy
```

> [!IMPORTANT]
> 完整的离线集群首次部署（`configs/deploy.yaml`）包含通过 `/bin/sh -c` 执行的归档解压、目录权限编排与状态检查任务及特权文件落盘，首次全量部署建议使用**方案 A** 或**方案 B**；集群交付转入日常受限巡检阶段后，可按需收紧为**方案 C**，并将 `/etc/ssh/sshd_config.d/99-rtkk-hardening.conf` 中的 `PermitRootLogin` 改为 `no` 后执行 `sshd -t && systemctl reload ssh`。

---

## 3. 在 `rtkk` 中配置 `become` 提权（Inventory / Play / Task）

`rtkk` 连接层内置 `PrivilegeConfig` 提权模型，同时支持 **`sudo`**（默认）与 **`su`**（基于 `PTY` 与目标账号 `root` 密码）两种提权方式，对远端命令执行（`command` / `shell` / `systemd` / `sysctl` / `modprobe`）及特权文件传输（`copy` / `template` / `fetch` 先经受控临时路径中转再提权原子落盘或读取）提供一致封装。

### 3.1 Inventory 全局或主机级配置（`configs/inventory.yaml`）

```yaml
vars:
  ansible_user: "deploy"
  ansible_ssh_private_key_file: "~/.ssh/rtkk_ed25519"
  ansible_become: true
  # 提权方式: "sudo"（默认，验证普通用户口令或免密） | "su"（无需配置 sudoers，验证 root 口令）
  ansible_become_method: "sudo"
  ansible_become_user: "root"
  # 若采用「口令保护的 sudo」或「su 提权」，可在此配置对应口令或通过 CLI -s 传入：
  # ansible_become_pass: "<STRONG_PASSWORD>"
```

若目标主机禁止修改 `/etc/sudoers`、仅允许普通用户登录后通过 `root` 密码执行 `su` 提权，只需指定 `ansible_become_method: "su"` 并通过 `-s` 传入 `root` 密码：

```bash
rtkk run configs/deploy.yaml -i configs/inventory.yaml \
  -s ansible_become=true \
  -s ansible_become_method=su \
  -s "ansible_become_pass=<ROOT_PASSWORD>"
```

### 3.2 Play 级与 Task 级细粒度覆盖

在自定义 Playbook 中，也可以在单个 Play 或单个 Task 上按需开启或关闭 `become`、切换 `become_method`（`sudo` / `su`）及指定 `become_user`（Task 级优先级高于 Play 级，Play 级高于 Inventory 默认值）：

```yaml
- name: Node maintenance with privilege escalation
  hosts: k8s_cluster
  become: true
  become_user: root
  tasks:
    - name: Run precheck as unprivileged SSH user
      command: id -un
      become: false

    - name: Restart kubelet service with root privilege
      systemd:
        name: kubelet
        state: restarted
      become: true
```

---

## 4. SSH 主机指纹校验（`HostKeyPolicy` 与 `known_hosts` 管理）

### 4.1 三种主机公钥校验策略

`rtkk` 通过 Inventory 变量 `ansible_ssh_host_key_checking`（以及可选的 `ansible_ssh_known_hosts_file`，默认指向 `$HOME/.ssh/known_hosts`）提供三档策略：

| 配置值 | 对应策略 | 行为说明 | 适用场景 |
|--------|----------|----------|----------|
| `false` 或 `"off"`（默认） | `Off` | 不校检远端 SSH 主机公钥 | 频繁重装系统的实验虚拟机、纯离线封闭测试台 |
| `"accept-new"` | `AcceptNew` | **首次连接信任（TOFU）**：若目标主机尚未记录于 `known_hosts`，自动接受并写入；若已有记录但远端公钥发生变化，**立即阻断连接并报错** | 内网长期运行集群（兼顾首次接入便利性与后续防劫持能力） |
| `true` 或 `"strict"` | `Strict` | **严格预置校验**：目标主机公钥必须预先存在于 `known_hosts` 且完全一致；未知主机或指纹变化均**立即阻断连接** | 跨机房生产网段、高合规等级生产环境 |

### 4.2 在 Debian 13 控制机预采集并核验节点指纹

采用 `"strict"`（`true`）模式前，可在 Debian 13 控制机通过 `ssh-keyscan` 采集各节点的 Ed25519 主机公钥并核对指纹：

```bash
# 1) 确保 ~/.ssh 与 known_hosts 权限合规
mkdir -p -m 0700 ~/.ssh
touch ~/.ssh/known_hosts
chmod 0600 ~/.ssh/known_hosts

# 2) 扫描集群节点 Ed25519 主机公钥写入 ~/.ssh/known_hosts（按实际节点 IP 替换）
ssh-keyscan -t ed25519 10.0.0.61 10.0.0.62 10.0.0.63 10.0.0.64 10.0.0.65 10.0.0.66 >> ~/.ssh/known_hosts

# 3) 查看已录入的主机 SHA256 指纹（可与节点本机 ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub 输出比对）
ssh-keygen -lf ~/.ssh/known_hosts
```

随后在 `configs/inventory.yaml` 中启用指纹校验：

```yaml
vars:
  ansible_user: "deploy"
  ansible_ssh_private_key_file: "~/.ssh/rtkk_ed25519"
  ansible_become: true
  ansible_become_user: "root"
  # 可选值: false / "off" | "accept-new" | true / "strict"
  ansible_ssh_host_key_checking: "accept-new"
  # 可选自定义路径（默认 ~/.ssh/known_hosts）：
  # ansible_ssh_known_hosts_file: "~/.ssh/known_hosts"
```

> [!TIP]
> 若某台节点因计划内重装操作系统更换了主机密钥，可在控制机执行 `ssh-keygen -R 10.0.0.61` 清除原有记录后重新采集。

---

## 相关文档

- [运维准备与安全建议手册](../OPERATION_GUIDE.md)
- [镜像仓库认证与 Harbor 权限配置指南](./registry-auth-and-harbor.md)
- [内网镜像仓库 TLS 加固与 HTTP 风险缓解指南](./registry-tls-and-http.md)
