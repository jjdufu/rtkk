# 07 · Stage B · Step 07：独立 Docker Compose 部署栈（`:19912`）、Docker Hub 自动发版与全链路终验

本步骤完成 `rtkk` 容器化生产交付与发布自动化收官：构建基于 `debian:13-slim` 非特权用户（`uid=10001`）的运行时镜像，落地解压即用的独立 `deploy/compose/` 生命周期脚本栈（默认映射 `:19912`），升级 GitHub Actions 自动推送 `dex2all/rtkk` 至 Docker Hub 并上传双归档 Release 产物，最后通过纯离线 Playwright E2E 与 `all.sh` 全量终验。

---

## 1. 目标与边界

1. **最小权限 `debian:13-slim` 容器镜像**：在 `deploy/docker/Dockerfile` 中仅安装 `ca-certificates`、`openssh-client`、`tzdata`，创建固定 `uid=10001, gid=10001` 的 `rtkk` 用户，内置 `rtkk` 单二进制（已编译内嵌前端与默认 `configs/`），默认启动 `rtkk serve --listen 0.0.0.0:19912 --work-dir /data` 并配置原生 `rtkk healthcheck`。
2. **零外部耦合的独立 `deploy/compose/` 部署包**：完整自包含 `config/{.env,compose.yml}`、`lib/common.sh` 与 `scripts/{env,start,stop,down,status,remove}.sh`，自动收敛宿主机挂载卷 `10001:10001` 属主与 `0700` 私密目录权限。
3. **CI / Release / Docker Hub 自动化流水线**：
   - `.github/scripts/release/debian13-build.sh` 在 `ci` 与 `release` 模式接入 Node.js `24` 构建 `web/dist/`；
   - `.github/workflows/release-please.yml` 向 `release.yml` 透传仓库 Secrets（`DOCKERHUB_USERNAME`、`DOCKERHUB_TOKEN`）；
   - `.github/workflows/release.yml` 自动打包 `rtkk_<ver>_linux_amd64.tar.gz` 与 `rtkk_<ver>_compose.tar.gz`，构建验证并推送 `dex2all/rtkk:<ver>` 与 `dex2all/rtkk:latest`。
4. **Park UI 五项强制交付门禁（Playwright 离线 E2E）**：在 `page.route('https://**', r => r.abort())` 断网拦截下验证双色对比度 `>= 4.5:1`、零原生 `<select>`、全键盘无障碍导航、`<768px` 零横向溢出及完整业务旅程。

---

## 2. 分队职责与文件变更矩阵

| 负责角色 | 动作 | 文件路径 | 变更内容说明 |
| :--- | :---: | :--- | :--- |
| `squad-ops-qa` | 新建 | `deploy/docker/Dockerfile` | `debian:13-slim` 运行时镜像定义（`uid=10001`、`:19912`、内置 `healthcheck`） |
| `squad-ops-qa` | 新建 | `deploy/compose/config/{.env,compose.yml}` | 版本与端口配置（`RTKK_PORT=19912`）及容器服务编排定义 |
| `squad-ops-qa` | 新建 | `deploy/compose/lib/common.sh` | 自包含容器编排生命周期库（`svc_start`/`stop`/`down`/`status`/`remove`、健康轮询日志） |
| `squad-ops-qa` | 新建 | `deploy/compose/scripts/{env,start,stop,down,status,remove}.sh` | 权限预处理钩子与六大运维入口脚本 |
| `squad-ops-qa` | 新建 | `web/playwright.config.ts`、`web/e2e/parkui-and-journey.spec.ts` | 纯离线 Park UI 规范审计与端到端业务旅程测试套件 |
| `squad-ops-qa` | 已预置 | [scripts/gate/step07.sh](../../../scripts/gate/step07.sh) | G3 收官门禁脚本（覆盖 `C28`、`C29`） |
| `orchestrator` | 修改 | `.github/scripts/release/{debian13-build.sh,pack-release.sh}` | 构建前编译 `web/dist/`，新增 `rtkk_<ver>_compose.tar.gz` 版本号注入与打包 |
| `orchestrator` | 修改 | `.github/workflows/{release-please.yml,release.yml}` | `secrets: inherit` 透传、新增 `docker` 构建自检推送作业与 Release 资产上传 |
| `orchestrator` | 修改 | [scripts/gate/all.sh](../../../scripts/gate/all.sh) | 将默认 `ACTIVE_GATES` 推进至 `"step00 step01 step02 step03 step04 step05 step06 step07"` |

---

## 3. 容器与 Compose 核心文件规格

### 3.1 `deploy/docker/Dockerfile`

```dockerfile
FROM debian:13-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates \
       openssh-client \
       tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 rtkk \
    && useradd --uid 10001 --gid 10001 --create-home --shell /usr/sbin/nologin rtkk \
    && install -d -m 0755 -o 10001 -g 10001 /data

COPY target/release/rtkk /usr/local/bin/rtkk
RUN chmod 0755 /usr/local/bin/rtkk

USER 10001:10001
WORKDIR /data
EXPOSE 19912

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=12 \
  CMD ["/usr/local/bin/rtkk", "healthcheck", "--url", "http://127.0.0.1:19912/api/v1/health"]

ENTRYPOINT ["/usr/local/bin/rtkk"]
CMD ["serve", "--listen", "0.0.0.0:19912", "--work-dir", "/data"]
```

### 3.2 `deploy/compose/config/compose.yml`（引号合规权限模式）

```yaml
name: rtkk
services:
  rtkk:
    image: dex2all/rtkk:${RTKK_VERSION?}
    container_name: rtkk-daemon
    restart: unless-stopped
    user: "10001:10001"
    environment:
      TZ: ${TZ:-Asia/Shanghai}
      RTKK_BIND_ADDR: "0.0.0.0:19912"
      RTKK_WORK_DIR: "/data"
    volumes:
      - ../data:/data
      - ../logs:/data/logs
      - ../backups:/data/rtkk_bak
    ports:
      - "0.0.0.0:${RTKK_PORT?}:19912"
    extra_hosts:
      - "host.docker.internal:host-gateway"
    mem_limit: 1g
    cpus: "2"
    healthcheck:
      test: ["CMD", "/usr/local/bin/rtkk", "healthcheck", "--url", "http://127.0.0.1:19912/api/v1/health"]
      interval: 10s
      timeout: 3s
      retries: 12
      start_period: 10s
    networks:
      - rtkk_net

networks:
  rtkk_net:
    name: rtkk-net
```

---

## 4. Playwright 五项强制自检断言规格（`web/e2e/parkui-and-journey.spec.ts`）

1. **离线零外部请求门禁**：全局注册网络拦截器阻断所有非 `127.0.0.1` 请求并统计外部请求数组，全流程结束后断言 `externalRequests.length === 0`。
2. **双色模式对比度门禁**：遍历 Light 与 Dark 两种主题及 4 套灰度/强调色组合，读取主按钮、输入框、侧栏激活项与模态标题的计算样式（`getComputedStyle`），断言前景与背景 WCAG 对比度全部 `>= 4.5`。
3. **零原生 `<select>` 门禁**：逐页挂载 `/login`、`/`、`/inventory`、`/templates`、`/artifacts`、`/operations`、`/pki`，断言 `await page.locator('select').count() === 0`。
4. **全键盘无障碍与 `<768px` 响应式门禁**：纯键盘 `Tab` / `ArrowDown` / `Enter` / `Escape` 驱动自定义下拉框、Theme Editor 与确认弹窗；切换视口至 `375×812` 断言 `scrollWidth <= innerWidth`。
5. **端到端业务旅程**：读取 `.rtkk-daemon/admin.token` 登录 → 开启 DNS 劫持免证书开关保存配置 → 预览节点 `containerd-config.toml` → 签发测试证书 → 触发沙盒编排任务并通过 SSE 观察状态收敛为 `succeeded`。

---

## 5. 门禁验收标准（[scripts/gate/step07.sh](../../../scripts/gate/step07.sh)）

- 追踪编号：`C28`（`"compose-lifecycle-19912"`）、`C29`（`"playwright-parkui-offline-e2e"`）。
- `bash scripts/gate/step07.sh` 输出 `GATE step07 PASS`，且 `bash scripts/gate/all.sh` 跑通 G1 + G2 + `step00..step07` 末行输出 `ALL GATES PASS`。
