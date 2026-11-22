# RTKK Park UI 前端工程约束（`web/`）

本文件定义 `web/` 子栈的专属工程纪律，与仓库根目录 [IRON_RULES.md](../IRON_RULES.md)、[AGENTS.md](../AGENTS.md)、[API_CONTRACT.md](../docs/API_CONTRACT.md) 及 [A1-CONVENTIONS-ENUMS-ROUTES.md](../docs/standards/A1-CONVENTIONS-ENUMS-ROUTES.md) 保持一致。

---

## 1. 技术栈与离线红线

1. **锁定依赖集**：仅允许使用 [A6-DEPENDENCY-BASELINE.md](../docs/standards/A6-DEPENDENCY-BASELINE.md) 核准的前端包（React `19`、TypeScript `5.7`、Vite `6`、`@ark-ui/react 5`、Tailwind CSS `v4`、`lucide-react`）。
2. **100% 纯离线渲染**：`index.html`、`src/**/*.css` 与 `src/**/*.{ts,tsx}` 严禁引用任何公网字体、CDN 脚本或外部 HTTP/HTTPS 资源；图标统一由 `lucide-react` 或内联 `<svg>` 提供。
3. **内网非 HTTPS 兼容**：`localStorage` 读写统一经 `src/lib/storage.ts` 的 `try / catch` 容错封装；剪贴板复制统一经 `src/lib/clipboard.ts`（优先 `navigator.clipboard.writeText`，回退隐藏 `<textarea>` + `document.execCommand("copy")`）。

---

## 2. TypeScript 严格模式与无障碍原语

1. **零静态绕过**：`tsconfig.json` 开启 `strict: true` 与 `noUncheckedIndexedAccess: true`；严禁出现 `: any`、`as any`、`<any>`、`@ts-ignore`、`@ts-expect-error`、`eslint-disable`。
2. **零原生阻塞控件**：严禁使用 `<select>`、`alert()`、`confirm()`、`prompt()`。下拉框、模态确认框、侧边抽屉、标签页、复选框（含过滤可见行 `indeterminate` 半选态）与开关统一基于 `@ark-ui/react` 封装，具备完整键盘焦点环与 `Tab` / `Arrow` / `Enter` / `Escape` 导航。

---

## 3. Park UI 双色 Token、响应式与四态覆盖

1. **语义变量与对比度**：`src/styles/tokens.css` 必须包含 `[hidden] { display: none !important; }`、`:root`（Light）与 `[data-theme="dark"]`（Dark）语义变量表；Theme Editor 动态计算 `--accent-fg` 保证前景与背景对比度 `>= 4.5:1`。
2. **等宽数字对齐**：所有 IP、端口、CIDR、字节体积、数量、耗时、时间戳与版本号统一应用 `.tabular-nums`（`font-variant-numeric: tabular-nums`）。
3. **页面四态与视口弹性**：所有业务页面覆盖骨架加载态、空状态、错误 `重试` 态与正常数据态；`< 768px` 窄屏视口侧栏转为模态抽屉，零横向滚动溢出。

---

## 4. 产品级极简微文案

1. **控件文案**：按钮、导航项与标签页仅用 2–4 字动词或领域名词（如 `登录`、`刷新`、`校验`、`保存`、`探测私库`、`部署集群`、`清理集群`、`签发证书`、`导出备份`），严禁出现 `点击`、`点此`、`选择`、`Click to`、`Press to` 等操作修饰词。
2. **表单与弹窗**：输入框标签仅写字段名，占位符仅给字面格式示例（如 `10.0.0.10/rtkk`）；高危确认框在正文陈述后果并要求输入确认词 `CLEANUP`，底部按钮使用 `取消` / `确认清理`。

---

## 5. 本地校验命令

```bash
cd web && npm run typecheck && npm run build
python3 scripts/web-guard.py
python3 scripts/docs-and-plans-guard.py
```
