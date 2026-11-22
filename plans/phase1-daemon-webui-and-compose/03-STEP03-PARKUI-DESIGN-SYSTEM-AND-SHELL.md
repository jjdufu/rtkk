# 03 · Stage B · Step 03：Park UI 设计系统、在线 Theme Editor 与前端应用壳层

本步骤初始化 `web/` 纯离线前端工程（React `19` + TypeScript `5.7` Strict + Vite `6` + `@ark-ui/react 5` + Tailwind CSS `v4`），落地 Park UI Light/Dark 双色 Token 体系、在线 Theme Editor 对比度引擎、零原生 `<select>`/`alert()`/`confirm()` 的无障碍原语库，以及令牌登录页、集群就绪概览大盘与响应式应用壳层。

---

## 1. 目标与边界

1. **纯离线构建与内嵌闭环**：零公网字体、零外部图标链接，`npm run build` 产物输出至 `web/dist/` 后由 `build.rs` 自动打包进 `rtkk` 单一二进制，支持 Gzip 协商与 SHA256 `ETag` `304` 缓存。
2. **Park UI 双色模式与在线 Theme Editor**：提供 Light / Dark 模式切换、4 套中性灰度基底（`Neutral`、`Slate`、`Zinc`、`Sand`）、5 档圆角阶梯（`0px` / `4px` / `6px` / `10px` / `99px`）与 6 组强调色（`Monochrome`、`Blue`、`Teal`、`Emerald`、`Amber`、`Rose`），按 WCAG 相对亮度实时求解 `--accent-fg` 确保对比度 `>= 4.5:1`。
3. **全套 `@ark-ui/react` 受控原语封装**：统一封装下拉框、组合搜索框、确认弹窗、侧边抽屉、标签页、开关与支持「过滤后可见行全选 + `indeterminate` 半选态」的数据表格，全仓零 `<select>`。
4. **内网非安全 HTTP 上下文兼容**：封装 `try / catch` 安全本地存储与 `navigator.clipboard` + 隐藏 `<textarea>` 双模剪贴板工具，保证通过 `http://192.168.x.x:19912` 内网 IP 访问时一键复制功能 100% 可用。

---

## 2. 分队职责与文件变更矩阵

| 负责角色 | 动作 | 文件路径 | 变更内容说明 |
| :--- | :---: | :--- | :--- |
| `orchestrator` | 新建 | `web/{package.json,tsconfig.json,vite.config.ts}` | 严格锁定 [A6](../../docs/standards/A6-DEPENDENCY-BASELINE.md) 准入包版本，开启 `strict: true`、`noUncheckedIndexedAccess: true`；开发态代理 `/api` 至 `http://127.0.0.1:19912` |
| `orchestrator` | 新建 | `web/src/{App.tsx,modules/registry.ts,lib/api/enums.ts}` | 应用路由装配、侧栏导航唯一事实表与同源枚举定义 |
| `squad-web` | 新建 | `web/AGENTS.md` | 前端专属红线记忆文件（`< 24,000` 字节，由 `docs-and-plans-guard.py` 校验） |
| `squad-web` | 新建 | `web/index.html`、`web/src/main.tsx` | 纯离线 HTML 入口与首屏防闪烁（FOUC）内联主题恢复脚本 |
| `squad-web` | 新建 | `web/src/styles/{tokens.css,app.css}` | `[hidden]` 强制隐藏规则、`:root` / `[data-theme="dark"]` 变量表、`:focus-visible` 焦点环与 `.tabular-nums` 工具类 |
| `squad-web` | 新建 | `web/src/lib/{theme.ts,storage.ts,clipboard.ts,sse.ts}` | WCAG 对比度计算器、容错存储封装、HTTP 降级复制器与带断线重连 `after_seq` 的 SSE Hook |
| `squad-web` | 新建 | `web/src/lib/api/{client.ts,auth.ts,jobs.ts}` | 强类型 `fetch` 封装（统一解析 `{code, message, request_id}`，遇 `401` 自动跳转 `/login`） |
| `squad-web` | 新建 | `web/src/components/primitives/{Button,Select,Combobox,Dialog,ConfirmModal,Drawer,Tabs,Checkbox,Switch,Toast,DataTable,StateView}.tsx` | 基于 `@ark-ui/react` 的无障碍基础组件库与四态视图容器 |
| `squad-web` | 新建 | `web/src/components/layout/{AppShell,Sidebar,TopBar,ThemeEditorDrawer,CommandPalette}.tsx` | 桌面固定侧栏 + `<768px` 抽屉折叠壳层、Theme Editor 抽屉与 `Cmd/Ctrl+K` 快捷命令面板 |
| `squad-web` | 新建 | `web/src/pages/{LoginPage,OverviewPage}.tsx` | 高熵令牌登录页与集群离线就绪度概览大盘（含骨架屏、空态与错误 `重试` 态） |
| `squad-ops-qa` | 已预置 | [scripts/gate/step03.sh](../../scripts/gate/step03.sh) | G3 门禁脚本（前端静态守卫、构建、内嵌二进制 SPA 路由与 Gzip/ETag `304` 验收） |

---

## 3. 设计系统与工程细节契约

### 3.1 `tokens.css` 语义变量表

```css
[hidden] {
  display: none !important;
}

:root {
  --bg: #fcfcfc;
  --surface: #f4f4f5;
  --surface-2: #e4e4e7;
  --border: #e4e4e7;
  --text: #18181b;
  --text-muted: #71717a;
  --accent: #18181b;
  --accent-fg: #ffffff;
  --danger: #dc2626;
  --danger-fg: #ffffff;
  --success: #16a34a;
  --warning: #d97706;
  --radius-md: 6px;
  --font-sans: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "PingFang SC", "Microsoft YaHei", sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}

[data-theme="dark"] {
  --bg: #09090b;
  --surface: #121215;
  --surface-2: #18181b;
  --border: #27272a;
  --text: #fafafa;
  --text-muted: #a1a1aa;
  --accent: #fafafa;
  --accent-fg: #09090b;
  --danger: #ef4444;
  --danger-fg: #ffffff;
  --success: #22c55e;
  --warning: #f59e0b;
  --radius-md: 6px;
}
```

### 3.2 导航路由与极简微文案规范

| 路由路径 | 侧栏名称 | 顶栏主动作按钮文案 | 空状态文案与主动作 |
| :--- | :--- | :--- | :--- |
| `/login` | — | `登录` | 占位符：`粘贴 43 位管理令牌` |
| `/` | `概览` | `刷新`、`主题` | `暂无任务记录` + `集群配置` |
| `/inventory` | `集群配置` | `校验`、`保存` | `暂无节点` + `添加节点` |
| `/templates` | `模板预览` | `渲染`、`复制` | `暂无可渲染节点` |
| `/artifacts` | `离线资源` | `探测私库`、`拉取镜像`、`推送私库` | `暂无镜像条目` |
| `/operations` | `编排执行` | `环境预检`、`部署集群`、`清理集群` | `暂无运行日志` |
| `/pki` | `证书备份` | `签发证书`、`导出备份` | `暂无证书文件` + `签发证书` |

---

## 4. 对抗审查要点（Verifier & RedTeam Checklist）

1. **暗色模式对比度防线**：在 Dark 模式下选择 `Monochrome` 预设时，`--accent` 必须反转为浅色 `#fafafa` 且 `--accent-fg` 为 `#09090b`（对比度 `19.3:1`），严禁将 Light 模式的深灰 `#18181b` 直接写入暗底造成黑底黑字。
2. **移动端零溢出**：在 `375px` 宽度下侧边导航自动转为 `@ark-ui/react` `Dialog`/`Drawer` 模态抽屉，顶栏与数据卡片自适应折行，无横向滚动条。
3. **全局监听器回收**：快捷键监听统一挂载于顶层 `CommandPalette` 单个 `useEffect` 并在卸载时 `removeEventListener`，严禁在表格行组件内注册全局事件。

---

## 5. 门禁验收标准（[scripts/gate/step03.sh](../../scripts/gate/step03.sh)）

- `python3 scripts/web-guard.py` 与 `python3 scripts/docs-and-plans-guard.py` 全部返回 `0`。
- `cd web && npm run typecheck && npm run build` 零错误完成打包。
- 追踪编号：`C20`（`"spa-embedded-index"`、`"spa-gzip-etag"`），`step03.sh` 末行输出 `GATE step03 PASS`。
