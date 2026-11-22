import { expect, test, type BrowserContext, type Locator, type Page } from "@playwright/test";

function readEnvVar(name: string): string | undefined {
  const proc = (
    globalThis as { process?: { env?: Record<string, string | undefined> } }
  ).process;
  return proc?.env?.[name];
}

function getAdminToken(): string {
  const token = readEnvVar("RTKK_E2E_TOKEN")?.trim() ?? "";
  if (!token) {
    throw new Error("RTKK_E2E_TOKEN environment variable is required for Playwright E2E");
  }
  return token;
}

async function installOfflineGuard(context: BrowserContext, bucket: string[]): Promise<void> {
  await context.route("**/*", async (route) => {
    const rawUrl = route.request().url();
    if (rawUrl.startsWith("data:") || rawUrl.startsWith("blob:")) {
      await route.continue();
      return;
    }
    try {
      const parsed = new URL(rawUrl);
      if (parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost") {
        await route.continue();
        return;
      }
    } catch {
      // Malformed non-local URL falls through to block
    }
    bucket.push(`${route.request().method()} ${rawUrl}`);
    await route.abort();
  });
}

async function waitPageSettled(page: Page): Promise<void> {
  await expect(page.locator(".park-skeleton")).toHaveCount(0);
}

async function selectOptionByKeyboard(
  page: Page,
  trigger: Locator,
  targetValue: string,
): Promise<void> {
  await expect(trigger).toBeFocused();
  await page.keyboard.press("Enter");
  const menu = page.locator('[data-scope="select"][data-part="content"]:not([hidden])');
  await expect(menu).toBeVisible();
  await expect(menu).toBeFocused();

  const targetItem = menu.locator(
    `[data-scope="select"][data-part="item"][data-value="${targetValue}"]`,
  );
  for (let step = 0; step < 8; step += 1) {
    const highlighted = await targetItem.getAttribute("data-highlighted");
    if (highlighted !== null) {
      break;
    }
    await page.keyboard.press("ArrowDown");
  }
  await expect(targetItem).toHaveAttribute("data-highlighted", "");
  await page.keyboard.press("Enter");
  await expect(menu).toBeHidden();
}

async function loginToConsole(page: Page, token: string): Promise<void> {
  await page.goto("/login");
  await waitPageSettled(page);
  await expect(page.locator("h1", { hasText: "控制台认证" })).toBeVisible();
  await page.locator("#rtkk-admin-token").fill(token);
  await page.locator('button[type="submit"]').click();
  await expect(page).toHaveURL(/\/$/);
  await waitPageSettled(page);
  await expect(page.locator("header h1", { hasText: "概览" })).toBeVisible();
}

interface ContrastSample {
  target: string;
  fg: string;
  bg: string;
  ratio: number;
}

const THEME_COMBINATIONS: ReadonlyArray<{
  neutral: string;
  accent: string;
}> = [
  { neutral: "Zinc", accent: "Monochrome" },
  { neutral: "Neutral", accent: "Blue" },
  { neutral: "Slate", accent: "Emerald" },
  { neutral: "Sand", accent: "Amber" },
  { neutral: "Zinc", accent: "Teal" },
  { neutral: "Slate", accent: "Rose" },
];

test.describe.serial("Park UI Five Mandatory Offline Gates & Full Business Journey", () => {
  let externalRequests: string[] = [];

  test.beforeEach(async ({ context }) => {
    externalRequests = [];
    await installOfflineGuard(context, externalRequests);
  });

  test.afterEach(async () => {
    expect(externalRequests, "WebUI must emit zero external network requests").toEqual([]);
  });

  test("Gates 1, 3, 4: offline isolation, zero native <select>, full keyboard navigation, and 375x812 responsive layout", async ({
    page,
  }) => {
    const token = getAdminToken();

    // Check /login unauthenticated state first (Gate 3 & Gate 4)
    await page.goto("/login");
    await waitPageSettled(page);
    expect(await page.locator("select").count()).toBe(0);

    await page.setViewportSize({ width: 375, height: 812 });
    const loginMetrics = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      innerWidth: window.innerWidth,
    }));
    expect(loginMetrics.scrollWidth).toBeLessThanOrEqual(loginMetrics.innerWidth);

    await page.setViewportSize({ width: 1280, height: 800 });
    await loginToConsole(page, token);

    // Gate 3: Visit every authenticated route and assert zero native <select>
    const routes: ReadonlyArray<{
      key: string;
      path: string;
      title: string;
      expectsArkSelect: boolean;
    }> = [
      { key: "overview", path: "/", title: "概览", expectsArkSelect: false },
      { key: "inventory", path: "/inventory", title: "集群配置", expectsArkSelect: true },
      { key: "templates", path: "/templates", title: "模板预览", expectsArkSelect: true },
      { key: "artifacts", path: "/artifacts", title: "离线资源", expectsArkSelect: false },
      { key: "operations", path: "/operations", title: "编排执行", expectsArkSelect: true },
      { key: "pki", path: "/pki", title: "证书备份", expectsArkSelect: false },
    ];

    for (const route of routes) {
      await page.locator(`aside button[data-nav-item="${route.key}"]`).click();
      await expect(page).toHaveURL(new RegExp(`${route.path === "/" ? "\\/$" : route.path}`));
      await expect(page.locator("header h1", { hasText: route.title })).toBeVisible();
      await waitPageSettled(page);
      expect(await page.locator("select").count(), `native <select> forbidden on ${route.path}`).toBe(0);
      if (route.expectsArkSelect) {
        expect(
          await page.locator('[data-scope="select"][data-part="trigger"]').count(),
          `Ark UI custom Select must mount on ${route.path}`,
        ).toBeGreaterThan(0);
      }
    }

    // Also verify modals/drawers containing dropdowns have zero native <select>
    await page.locator("main button", { hasText: "签发证书" }).first().click();
    const pkiDialogTitle = page.locator('[data-scope="dialog"][data-part="title"]', {
      hasText: "签发证书",
    });
    await expect(pkiDialogTitle).toBeVisible();
    expect(await page.locator("select").count()).toBe(0);
    await page.keyboard.press("Escape");
    await expect(pkiDialogTitle).toBeHidden();

    // Gate 4A: Pure keyboard navigation on Ark UI Select (/inventory)
    await page.locator('aside button[data-nav-item="inventory"]').click();
    await waitPageSettled(page);

    const ipRangeInput = page.locator('input[placeholder="192.168.26.61-66"]');
    await ipRangeInput.focus();
    await page.keyboard.press("Tab");

    const roleSelectTrigger = page.locator(
      '[data-scope="select"][data-part="trigger"][aria-label="默认角色组"]',
    );
    await expect(roleSelectTrigger).toBeFocused();
    await expect(roleSelectTrigger).toContainText("工作节点");

    await selectOptionByKeyboard(page, roleSelectTrigger, "control_and_etcd");
    await expect(roleSelectTrigger).toContainText("控制面 + etcd");

    await roleSelectTrigger.focus();
    await page.keyboard.press("Enter");
    const openSelectMenu = page.locator('[data-scope="select"][data-part="content"]:not([hidden])');
    await expect(openSelectMenu).toBeVisible();
    await expect(openSelectMenu).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(openSelectMenu).toBeHidden();

    // Gate 4B: Pure keyboard navigation on Theme Editor Drawer
    const cmdPaletteBtn = page.locator('header button[aria-label="快捷导航"]');
    await cmdPaletteBtn.focus();
    await page.keyboard.press("Tab");
    const themeTriggerBtn = page.locator("header button", { hasText: "主题" });
    await expect(themeTriggerBtn).toBeFocused();
    await page.keyboard.press("Enter");

    const themeDrawerTitle = page.locator('[data-scope="dialog"][data-part="title"]', {
      hasText: "主题设置",
    });
    await expect(themeDrawerTitle).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(themeDrawerTitle).toBeHidden();

    // Gate 4C: Pure keyboard navigation on high-danger ConfirmModal (/operations)
    await page.locator('aside button[data-nav-item="operations"]').click();
    await waitPageSettled(page);
    expect(await page.locator("select").count()).toBe(0);

    const deployBtn = page.locator("main button", { hasText: "部署集群" });
    await deployBtn.focus();
    await page.keyboard.press("Tab");
    const cleanupBtn = page.locator("main button", { hasText: "清理集群" });
    await expect(cleanupBtn).toBeFocused();
    await page.keyboard.press("Enter");

    const confirmTitle = page.locator('[data-scope="dialog"][data-part="title"]', {
      hasText: "清理集群",
    });
    await expect(confirmTitle).toBeVisible();
    const confirmSubmitBtn = page.locator('[data-scope="dialog"][data-part="content"] button', {
      hasText: "确认清理",
    });
    await expect(confirmSubmitBtn).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(confirmTitle).toBeHidden();

    // Gate 4D: <768px (375x812) responsive zero horizontal overflow + mobile drawer navigation
    await page.setViewportSize({ width: 375, height: 812 });
    const mobileMenuBtn = page.locator('header button[aria-label="导航菜单"]');
    await expect(mobileMenuBtn).toBeVisible();

    for (const route of routes) {
      await mobileMenuBtn.click();
      const mobileDrawer = page.locator('[data-scope="dialog"][data-part="content"]');
      await expect(mobileDrawer).toBeVisible();
      await mobileDrawer.locator(`button[data-nav-item="${route.key}"]`).click();
      await expect(page.locator("header h1", { hasText: route.title })).toBeVisible();
      await waitPageSettled(page);

      const dims = await page.evaluate(() => ({
        scrollWidth: document.documentElement.scrollWidth,
        innerWidth: window.innerWidth,
      }));
      expect(
        dims.scrollWidth,
        `375px viewport horizontal overflow detected on ${route.path} (${dims.scrollWidth} > ${dims.innerWidth})`,
      ).toBeLessThanOrEqual(dims.innerWidth);
    }

    await page.setViewportSize({ width: 1280, height: 800 });
  });

  test("Gate 2: WCAG 2.1 contrast >= 4.5:1 across Light & Dark modes and Neutral/Accent combinations", async ({
    page,
  }) => {
    const token = getAdminToken();
    await loginToConsole(page, token);

    await page.locator('aside button[data-nav-item="inventory"]').click();
    await waitPageSettled(page);

    await page.locator("header button", { hasText: "主题" }).click();
    const drawerContent = page.locator('[data-scope="dialog"][data-part="content"]');
    await expect(drawerContent).toBeVisible();

    const modeLabels: ReadonlyArray<"浅色" | "深色"> = ["浅色", "深色"];

    for (const modeLabel of modeLabels) {
      await drawerContent.locator("button", { hasText: modeLabel }).click();

      for (const combo of THEME_COMBINATIONS) {
        await drawerContent.locator("button", { hasText: combo.neutral }).click();
        await drawerContent.locator("button", { hasText: combo.accent }).click();

        await page.evaluate(async () => {
          await new Promise<void>((resolve) => {
            window.requestAnimationFrame(() => {
              window.requestAnimationFrame(() => resolve());
            });
          });
          const transitions = document
            .getAnimations()
            .filter((anim) => "transitionProperty" in anim);
          await Promise.all(transitions.map((t) => t.finished.catch(() => undefined)));
        });

        const samples = await page.evaluate((): ContrastSample[] => {
          type Rgba = [number, number, number, number];

          const parseCssColor = (raw: string): Rgba | null => {
            const trimmed = raw.trim();
            if (trimmed === "transparent") {
              return [0, 0, 0, 0];
            }
            const rgbaMatch = trimmed.match(
              /^rgba?\(\s*([0-9.]+)[,\s]+([0-9.]+)[,\s]+([0-9.]+)(?:[,/\s]+([0-9.]+))?\s*\)$/i,
            );
            if (!rgbaMatch) {
              return null;
            }
            const r = Number.parseFloat(rgbaMatch[1] ?? "0") / 255;
            const g = Number.parseFloat(rgbaMatch[2] ?? "0") / 255;
            const b = Number.parseFloat(rgbaMatch[3] ?? "0") / 255;
            const a = rgbaMatch[4] !== undefined ? Number.parseFloat(rgbaMatch[4]) : 1;
            return [r, g, b, a];
          };

          const compositeOver = (fg: Rgba, bg: [number, number, number]): [number, number, number] => {
            const alpha = Math.max(0, Math.min(1, fg[3]));
            return [
              fg[0] * alpha + bg[0] * (1 - alpha),
              fg[1] * alpha + bg[1] * (1 - alpha),
              fg[2] * alpha + bg[2] * (1 - alpha),
            ];
          };

          const resolveEffectiveBackground = (el: Element): { rgb: [number, number, number]; css: string } => {
            const layers: Rgba[] = [];
            let cur: Element | null = el;
            let baseCss = "rgb(255, 255, 255)";
            while (cur) {
              const bgStr = window.getComputedStyle(cur).backgroundColor;
              const parsed = parseCssColor(bgStr);
              if (parsed && parsed[3] > 0) {
                layers.push(parsed);
                if (parsed[3] >= 0.999) {
                  baseCss = bgStr;
                  break;
                }
              }
              cur = cur.parentElement;
            }
            let acc: [number, number, number] = [1, 1, 1];
            for (let i = layers.length - 1; i >= 0; i -= 1) {
              const layer = layers[i];
              if (layer) {
                acc = compositeOver(layer, acc);
              }
            }
            return { rgb: acc, css: baseCss };
          };

          const luminance = (rgb: [number, number, number]): number => {
            const lin = (c: number): number =>
              c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
            return 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2]);
          };

          const ratioBetween = (
            a: [number, number, number],
            b: [number, number, number],
          ): number => {
            const l1 = luminance(a);
            const l2 = luminance(b);
            const hi = Math.max(l1, l2);
            const lo = Math.min(l1, l2);
            return (hi + 0.05) / (lo + 0.05);
          };

          const targets: ReadonlyArray<{ name: string; selector: string }> = [
            { name: "primary-button", selector: 'main [data-park-button="primary"]' },
            { name: "text-input", selector: "main input.park-input" },
            { name: "sidebar-active-item", selector: 'aside button[aria-current="page"]' },
            { name: "drawer-title", selector: '[data-scope="dialog"][data-part="title"]' },
          ];

          return targets.map((t) => {
            const el = document.querySelector(t.selector);
            if (!el) {
              return { target: t.name, fg: "missing", bg: "missing", ratio: 0 };
            }
            const fgStr = window.getComputedStyle(el).color;
            const fgParsed = parseCssColor(fgStr);
            const bgResolved = resolveEffectiveBackground(el);
            if (!fgParsed) {
              return { target: t.name, fg: fgStr, bg: bgResolved.css, ratio: 0 };
            }
            const fgRgb = compositeOver(fgParsed, bgResolved.rgb);
            return {
              target: t.name,
              fg: fgStr,
              bg: bgResolved.css,
              ratio: ratioBetween(fgRgb, bgResolved.rgb),
            };
          });
        });

        expect(samples).toHaveLength(4);
        for (const s of samples) {
          expect(
            s.ratio,
            `WCAG contrast < 4.5 in mode=${modeLabel} neutral=${combo.neutral} accent=${combo.accent} on ${s.target} (fg=${s.fg}, bg=${s.bg})`,
          ).toBeGreaterThanOrEqual(4.5);
        }
      }
    }

    await drawerContent.locator("button", { hasText: "重置默认" }).click();
    await page.keyboard.press("Escape");
    await expect(drawerContent).toBeHidden();
  });

  test("Gate 5: End-to-end journey (login -> DNS hijack insecure switch -> containerd template -> PKI cert -> live SSE job succeeded)", async ({
    page,
  }) => {
    const token = getAdminToken();

    // 1. Login via admin.token
    await loginToConsole(page, token);
    await expect(page.locator("main", { hasText: "写锁空闲" })).toBeVisible();

    // 2. Enable public_registries_insecure & set sandbox CNI=none in /inventory, save & verify persistence
    await page.locator('aside button[data-nav-item="inventory"]').click();
    await waitPageSettled(page);

    await page.getByRole("tab", { name: "镜像私库" }).click();
    const insecureSwitch = page.locator(
      '[data-testid="public-registries-insecure-box"] [data-scope="switch"][data-part="control"]',
    );
    await expect(insecureSwitch).toBeVisible();
    const initialState = await insecureSwitch.getAttribute("data-state");
    if (initialState !== "checked") {
      await insecureSwitch.click();
    }
    await expect(insecureSwitch).toHaveAttribute("data-state", "checked");

    await page.getByRole("tab", { name: "集群网络" }).click();
    const cniTrigger = page.locator(
      '[data-scope="select"][data-part="trigger"][aria-label="CNI 网络插件"]',
    );
    await cniTrigger.click();
    await page
      .locator('[data-scope="select"][data-part="content"]:not([hidden]) [data-scope="select"][data-part="item"]', {
        hasText: "不安装 (none)",
      })
      .click();
    await expect(cniTrigger).toContainText("不安装 (none)");

    await page.locator("main button", { hasText: "保存" }).click();
    await expect(page.getByText("已保存")).toBeVisible();

    await page.reload();
    await waitPageSettled(page);
    await page.getByRole("tab", { name: "镜像私库" }).click();
    await expect(
      page.locator(
        '[data-testid="public-registries-insecure-box"] [data-scope="switch"][data-part="control"]',
      ),
    ).toHaveAttribute("data-state", "checked");

    // 3. Preview rendered containerd-config.toml in /templates
    await page.locator('aside button[data-nav-item="templates"]').click();
    await waitPageSettled(page);

    const containerdBtn = page.locator("main button", { hasText: "containerd-config.toml" });
    await expect(containerdBtn).toBeVisible();
    await containerdBtn.click();

    await expect(page.locator("main .park-badge", { hasText: "语法合法" })).toBeVisible();
    const previewCode = page.locator("main pre code");
    await expect(previewCode).toContainText('/etc/containerd/certs.d');
    await expect(previewCode).toContainText("SystemdCgroup = true");

    // 4. Issue and verify X.509 certificate in /pki without leaking private key PEM
    await page.locator('aside button[data-nav-item="pki"]').click();
    await waitPageSettled(page);

    await page.locator("main button", { hasText: "签发证书" }).first().click();
    const genDialog = page.locator('[data-scope="dialog"][data-part="content"]');
    await expect(genDialog).toBeVisible();

    await genDialog.locator('input[name="name"]').fill("e2e-harbor-tls");
    await genDialog.locator('input[name="cn"]').fill("10.0.0.10");
    await genDialog.locator('textarea[name="sans"]').fill("10.0.0.10, harbor.e2e.internal");
    await genDialog.locator("button", { hasText: "签发证书" }).click();
    await expect(genDialog).toBeHidden();

    const certRow = page.locator("main tr", { hasText: "custom/e2e-harbor-tls.crt" });
    await expect(certRow).toBeVisible();
    await expect(certRow).toContainText("harbor.e2e.internal");
    await expect(certRow).toContainText("0600 已匹配");
    const pkiBodyText = await page.locator("main").innerText();
    expect(pkiBodyText).not.toContain("PRIVATE KEY");

    await certRow.locator("button", { hasText: "校验" }).click();
    const verifyDialog = page.locator('[data-scope="dialog"][data-part="content"]');
    await expect(verifyDialog).toBeVisible();
    await expect(verifyDialog.locator(".park-badge", { hasText: "核验通过" })).toBeVisible();
    await verifyDialog.locator("button", { hasText: "关闭" }).click();
    await expect(verifyDialog).toBeHidden();

    // 5. Trigger local sandbox async orchestration in /operations and watch SSE converge to succeeded
    await page.locator('aside button[data-nav-item="operations"]').click();
    await waitPageSettled(page);

    await page.locator("main button", { hasText: "编排参数" }).click();
    const opsDrawer = page.locator('[data-scope="dialog"][data-part="content"]');
    await expect(opsDrawer).toBeVisible();

    const playbookTrigger = opsDrawer.locator(
      '[data-scope="select"][data-part="trigger"][aria-label="目标剧本"]',
    );
    await playbookTrigger.focus();
    await expect(playbookTrigger).toBeFocused();
    await selectOptionByKeyboard(page, playbookTrigger, "download");
    await expect(playbookTrigger).toContainText("离线下载 (download.yaml)");

    const chartsTagCheckbox = opsDrawer.locator(
      '[data-scope="checkbox"][data-part="control"][aria-label="charts"]',
    );
    await expect(chartsTagCheckbox).toBeVisible();
    await chartsTagCheckbox.click();

    await opsDrawer.locator("button", { hasText: "执行编排" }).click();
    await expect(opsDrawer).toBeHidden();

    const statusBadge = page.getByTestId("active-job-status");
    await expect(statusBadge).toHaveText("状态 · succeeded", { timeout: 15_000 });
    await expect(page.locator("main")).toContainText("Download Cilium Helm chart archive");
    await expect(page.locator("main")).toContainText("localhost");
  });
});
