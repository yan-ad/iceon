import { expect, test, type Page } from "@playwright/test";

for (const platform of ["browser", "macos", "windows", "linux"]) {
  test(`${platform}: frame follows the zoomed root and reserves insets once`, async ({ page }) => {
    if (platform !== "browser") await mockWindow(page, platform);
    await page.setViewportSize({ width: 390, height: 640 });
    await page.goto("/");
    const main = page.getByRole("main");
    await expect(main).toBeVisible();
    const frame = page.locator(".astryx-stack").filter({ has: page.locator(".astryx-app-shell") }).first();

    for (const zoom of [1, 1.25, 1.5, 1]) {
      await page.evaluate(zoom => { document.documentElement.style.zoom = String(zoom); }, zoom);
      for (const height of [640, 520]) {
        await page.setViewportSize({ width: 390, height });
        await expect.poll(async () => (await frame.boundingBox())!.height).toBeCloseTo(height, 0);
        const bounds = (await main.boundingBox())!;
        expect(bounds.y + bounds.height).toBeLessThanOrEqual(height + 1);
        const sidebar = main.getByRole("navigation", { name: "Catalog navigation", exact: true });
        const navigation = (await sidebar.boundingBox())!;
        expect(navigation.y + navigation.height).toBeLessThanOrEqual(height + 1);
        expect(await page.evaluate(() => ({
          width: document.documentElement.scrollWidth <= window.innerWidth,
          height: document.documentElement.scrollHeight <= window.innerHeight,
        }))).toEqual({ width: true, height: true });
      }
    }

    // Synthetic device insets exercise the frame's border-box budget without OS APIs.
    await frame.evaluate(element => {
      element.style.paddingTop = "var(--spacing-3)";
      element.style.paddingBottom = "var(--spacing-4)";
    });
    await page.evaluate(() => { document.documentElement.style.zoom = "1.25"; });
    const inset = await frame.evaluate(element => {
      const style = getComputedStyle(element);
      return { top: Number.parseFloat(style.paddingTop) * 1.25, bottom: Number.parseFloat(style.paddingBottom) * 1.25 };
    });
    const toolbar = (await page.getByRole("navigation", { name: "App toolbar", exact: true }).boundingBox())!;
    const content = (await main.boundingBox())!;
    expect((await frame.boundingBox())!.height).toBeCloseTo(520, 0);
    expect(toolbar.y).toBeCloseTo(inset.top, 0);
    expect(content.y + content.height).toBeCloseTo(520 - inset.bottom, 0);
  });
}

async function mockWindow(page: Page, platform: string, seamless = true, failActions = false) {
  await page.addInitScript(({ platform, seamless, failActions }) => {
    Object.defineProperty(window, "__ICEON_WINDOW_LAYOUT__", {
      value: Object.freeze({ platform, seamless }),
    });
    const calls: string[] = [];
    Object.assign(window, {
      __windowCalls: calls,
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" } },
        invoke: async (command: string) => {
          calls.push(command);
          if (failActions) throw new Error("Window command rejected");
        },
      },
    });
  }, { platform, seamless, failActions });
}

for (const platform of ["macos", "windows", "linux"]) {
  test(`${platform}: one toolbar row clears native controls at desktop and narrow widths`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await mockWindow(page, platform);
    await page.goto("/");
    await page.setViewportSize({ width: 1100, height: 700 });
    await expect(page).toHaveURL(/#\/catalog$/);
    const toolbar = page.getByTestId("window-safe-area");
    await expect(toolbar).toBeVisible();
    await expect(page.getByRole("navigation", { name: "App toolbar", exact: true })).toHaveCount(1);
    await expect(toolbar.getByText("Iceon", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Close window", exact: true }))
      .toHaveCount(platform === "macos" ? 0 : 1);
    await expect(toolbar.locator("[data-tauri-drag-region]").first()).toBeVisible();
    for (const width of [1100, 800, 360]) {
      await page.setViewportSize({ width, height: 600 });
      await expect(toolbar.getByRole("button", { name: /^(?:Open|Collapse|Expand) sidebar$/ })).toHaveCount(0);
      await expect(page.getByRole("button", { name: /^(?:Collapse|Expand) sidebar$/ })).toHaveCount(0);
      if (width < 768) {
        const rail = page.getByRole("main").getByRole("navigation", { name: "Catalog navigation", exact: true });
        await expect(rail.getByRole("button", { name: "Open sidebar", exact: true })).toBeVisible();
      }
      await expect(toolbar).toBeVisible();
      const chrome = await toolbar.boundingBox();
      const main = await page.getByRole("main").boundingBox();
      expect(chrome!.height).toBeGreaterThanOrEqual(44);
      expect(chrome!.y).toBe(0);
      expect(chrome!.height).toBeLessThanOrEqual(48);
      for (const name of ["Go back", "Go forward", "Refresh catalog", ...(width < 768 ? ["Toolbar actions"] : ["Preferences", "Search"])]) {
        const action = page.getByRole("button", { name, exact: true });
        await expect(action).toBeVisible();
        const bounds = await action.boundingBox();
        expect(bounds!.y).toBeGreaterThanOrEqual(chrome!.y);
        expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(chrome!.y + chrome!.height);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
        expect(await action.evaluate((element) => element.closest("[data-tauri-drag-region]") === null)).toBe(true);
      }
      if (width < 768) {
        await toolbar.getByRole("button", { name: "Toolbar actions", exact: true }).click();
        for (const name of ["Preferences", "Search"]) await expect(page.getByRole("menuitem", { name, exact: true })).toBeVisible();
        await page.keyboard.press("Escape");
      }
      const forward = (await toolbar.getByRole("button", { name: "Go forward", exact: true }).boundingBox())!;
      const refresh = (await toolbar.getByRole("button", { name: "Refresh catalog", exact: true }).boundingBox())!;
      expect(refresh.x).toBeGreaterThanOrEqual(forward.x + forward.width);
      const firstControl = toolbar.getByRole("button").first();
      await expect(firstControl).toHaveAccessibleName("Go back");
      const back = await firstControl.boundingBox();
      if (platform === "macos") {
        expect(back!.x).toBeGreaterThanOrEqual(80);
        const lastAction = (await toolbar.getByRole("button", { name: width < 768 ? "Toolbar actions" : "Open Iceon on GitHub", exact: true }).boundingBox())!;
        const clearance = await toolbar.evaluate(element => Number.parseFloat(getComputedStyle(element).paddingRight));
        expect(clearance).toBeGreaterThan(0);
        expect(chrome!.x + chrome!.width - lastAction.x - lastAction.width).toBeCloseTo(clearance, 1);
      }
      expect(main!.y).toBeGreaterThanOrEqual(chrome!.y + chrome!.height);
      expect(main!.y).toBeLessThanOrEqual(chrome!.y + chrome!.height + 1);
      expect(main!.y + main!.height).toBeLessThanOrEqual(601);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    }
    expect(errors).toEqual([]);
  });

  test(`${platform}: disabling seamless keeps Catalog without native chrome`, async ({ page }) => {
    await mockWindow(page, platform, false);
    await page.goto("/");
    await expect(page.getByTestId("window-safe-area")).toHaveCount(0);
  await expect(page.locator("[data-seamless], [data-tauri-drag-region]")).toHaveCount(0);
  for (const name of ["Minimize window", "Maximize or restore window", "Close window"]) {
    await expect(page.getByRole("button", { name, exact: true })).toHaveCount(0);
  }
  const toolbar = page.getByRole("navigation", { name: "App toolbar", exact: true });
  expect(await toolbar.evaluate(element => getComputedStyle(element).paddingRight)).toBe("0px");
    await expect(page.getByRole("heading", { level: 1, name: "Clusters", exact: true })).toBeAttached();
    await expect(page.getByRole("button", { name: "Close window", exact: true })).toHaveCount(0);
  });
}

test("browser preview opens Catalog and preserves history and search without native APIs", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  await expect(page).toHaveURL(/#\/catalog$/);
  await expect(page.getByRole("heading", { level: 1, name: "Clusters", exact: true })).toBeAttached();
  await expect(page.getByTestId("window-safe-area")).toHaveCount(0);
  await page.getByRole("link", { name: "Clusters", exact: true }).click();
  await expect(page).toHaveURL(/#\/catalog\/clusters$/);
  await page.getByRole("button", { name: "Go back", exact: true }).click();
  await expect(page).toHaveURL(/#\/catalog$/);
  await page.getByRole("button", { name: "Go forward", exact: true }).click();
  await expect(page).toHaveURL(/#\/catalog\/clusters$/);
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByRole("dialog").getByText("Welcome Page", { exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("window controls invoke native commands and do not become drag regions", async ({ page }) => {
  await mockWindow(page, "windows");
  await page.goto("/");
  for (const name of ["Minimize window", "Maximize or restore window", "Close window"]) {
    const control = page.getByRole("button", { name, exact: true });
    await expect(control).not.toHaveAttribute("data-tauri-drag-region");
    expect(await control.evaluate((element) => element.closest("[data-tauri-drag-region]") === null)).toBe(true);
    await control.focus();
    await page.keyboard.press("Enter");
  }
  expect(await page.evaluate(() => (window as unknown as { __windowCalls: string[] }).__windowCalls)).toEqual([
    "plugin:window|minimize", "plugin:window|toggle_maximize", "plugin:window|close",
  ]);
});

test("failed native actions show an accessible error", async ({ page }) => {
  await mockWindow(page, "linux", true, true);
  await page.goto("/");
  await page.getByRole("button", { name: "Minimize window", exact: true }).click();
  await expect(page.getByRole("alert").filter({ hasText: "Window action failed" })).toBeVisible();
});

test("sidebar-local control keeps narrow-window navigation accessible", async ({ page }) => {
  await mockWindow(page, "macos");
  await page.setViewportSize({ width: 360, height: 600 });
  await page.goto("/");
  await expect(page).toHaveURL(/#\/catalog$/);
  await expect(page.getByRole("navigation", { name: "App toolbar", exact: true }).getByRole("button", { name: "Open sidebar", exact: true })).toHaveCount(0);
  const toggle = page.getByRole("main").getByRole("button", { name: "Open sidebar", exact: true });
  await toggle.click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(toggle).toBeFocused();
});

test("categories cannot be pinned and there is only one sidebar", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("iceon.pinned-pages.v1", JSON.stringify([
    "/catalog/general", "/catalog/clusters", "/catalog/web-links",
  ])));
  await page.setViewportSize({ width: 1100, height: 700 });
  await page.goto("/");
  const sidebar = page.getByRole("navigation", { name: "Catalog navigation", exact: true });
  await expect(sidebar).toHaveCount(1);
  await expect(sidebar.getByRole("link")).toHaveCount(2);
  await expect(sidebar.getByRole("link", { name: "Clusters", exact: true })).toBeVisible();
  await expect(sidebar.getByRole("link", { name: "Secrets", exact: true })).toHaveAttribute("href", "/#/secrets");
  await expect(page.getByRole("navigation", { name: "Apps", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Home", exact: true })).toHaveCount(0);
  for (const category of ["Clusters"]) {
    await page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: category, exact: true }).click();
    await expect(page.getByRole("heading", { level: 1, name: category, exact: true })).toBeAttached();
    await expect(page.getByRole("button", { name: /\b(?:Pin|Unpin)\b/ })).toHaveCount(0);
  }
  await expect(page.getByRole("heading", { name: "Desktop engine required", exact: true })).toBeVisible();
  await expect(page.getByRole("table", { name: "Clusters", exact: true })).toHaveCount(0);
  await page.reload();
  await expect(sidebar.getByRole("link")).toHaveCount(2);
  await sidebar.getByRole("link", { name: "Secrets", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Secrets", exact: true })).toBeAttached();
  await expect(page.getByRole("button", { name: /\b(?:Pin|Unpin)\b/ })).toHaveCount(0);
  await page.keyboard.press("Control+k");
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("Catalog navigation stays fixed collapsed across reload without preference access", async ({ page }, testInfo) => {
  await page.addInitScript(() => {
    localStorage.setItem("iceon.sidebar-collapsed.v1", "false");
    const accesses: string[] = [];
    Object.assign(window, { __sidebarPreferenceAccesses: accesses });
    for (const method of ["getItem", "setItem", "removeItem"] as const) {
      const original = Storage.prototype[method];
      Storage.prototype[method] = function (...args: string[]) {
        if (args[0] === "iceon.sidebar-collapsed.v1") accesses.push(method);
        return Reflect.apply(original, this, args);
      };
    }
  });
  await page.setViewportSize({ width: 1100, height: 700 });
  await page.goto("/");
  const sidebar = page.getByRole("navigation", { name: "Catalog navigation", exact: true });
  await expect(sidebar).toHaveCount(1);
  await expect(sidebar).toHaveAttribute("data-collapsed", "true");
  const width = (await sidebar.boundingBox())!.width;
  expect(width).toBeGreaterThan(0);
  expect(width).toBeLessThanOrEqual(64);
  for (const reload of [false, true]) {
    if (reload) await page.reload();
    await expect(sidebar).toHaveAttribute("data-collapsed", "true");
    await expect(page.getByRole("button", { name: /^(?:Open|Collapse|Expand) sidebar$/ })).toHaveCount(0);
    const categoryLabel = sidebar.getByText("Navigation", { exact: true });
    expect(await categoryLabel.evaluate(element => {
      for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
        const style = getComputedStyle(ancestor);
        const bounds = ancestor.getBoundingClientRect();
        if (style.position === "absolute" && style.overflow === "hidden" && bounds.width <= 1 && bounds.height <= 1) return true;
      }
      return false;
    })).toBe(true);
    await expect(sidebar.getByRole("group", { name: "Navigation", exact: true })).toBeAttached();
    await expect(sidebar.getByRole("link", { name: "Clusters", exact: true })).toBeVisible();
    await sidebar.getByRole("link", { name: "Clusters", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Desktop engine required", exact: true })).toBeVisible();
    await expect(page.getByRole("table", { name: "Clusters", exact: true })).toHaveCount(0);
    expect((await sidebar.boundingBox())!.width).toBeCloseTo(width, 1);
    expect(await page.evaluate(() => (window as unknown as { __sidebarPreferenceAccesses: string[] }).__sidebarPreferenceAccesses)).toEqual([]);
  }
  await page.screenshot({ path: testInfo.outputPath("sidebar-fixed-collapsed.png") });
});

test("narrow sidebar-local drawer closes after navigation and returns to the fixed desktop rail", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 700 });
  await page.goto("/");
  await expect(page.getByRole("navigation", { name: "App toolbar", exact: true }).getByRole("button", { name: "Open sidebar", exact: true })).toHaveCount(0);
  const toggle = page.getByRole("main").getByRole("button", { name: "Open sidebar", exact: true });
  await toggle.click();
  const drawer = page.getByRole("dialog");
  await expect(drawer.getByRole("navigation", { name: "Catalog navigation", exact: true })).toHaveCount(1);
  await expect(drawer.getByRole("link", { name: "Clusters", exact: true })).toBeVisible();
  await drawer.getByRole("link", { name: "Clusters", exact: true }).click();
  await expect(drawer).toBeHidden();
  await expect(page.getByRole("heading", { name: "Desktop engine required", exact: true })).toBeVisible();
  await expect(page.getByRole("table", { name: "Clusters", exact: true })).toHaveCount(0);
  await page.setViewportSize({ width: 1100, height: 700 });
  await expect(page.getByRole("button", { name: /^(?:Open|Collapse|Expand) sidebar$/ })).toHaveCount(0);
  await expect(page.getByRole("navigation", { name: "Catalog navigation", exact: true })).toHaveAttribute("data-collapsed", "true");
});
