import { expect, test, type Page } from "@playwright/test";

async function engineFixture(page: Page, options: { empty?: boolean; failList?: boolean; slow?: boolean; holdList?: boolean; discoveryFailure?: boolean; defaultConfig?: boolean } = {}) {
  await page.addInitScript((options) => {
    const initial = { revision: 1, discoveryError: null, clusters: options.empty ? [] : [{ id: "cluster-fixture", name: "Development", contextName: "Development", source: "kubeconfig", labels: ["file=/fixtures/fixture.yaml"], color: "#476BA1", status: "disconnected", pinned: false }] };
    const read = () => JSON.parse(sessionStorage.getItem("test.catalog") ?? JSON.stringify(initial));
    const calls: { command: string; request: unknown }[] = [];
    const pending: (() => void)[] = [];
    Object.assign(window, {
      isTauri: true,
      __catalogCalls: calls,
      __releaseCatalogList: () => { options.holdList = false; pending.splice(0).forEach(resolve => resolve()); },
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args?: { request: Record<string, unknown> }) => {
          const request = args?.request ?? {};
          calls.push({ command, request });

          if (options.holdList && command === "catalog_list") await new Promise<void>(resolve => pending.push(resolve));
          if (options.slow) await new Promise((resolve) => setTimeout(resolve, 300));
          if (options.failList && command === "catalog_list") throw "storeCorrupt";
          const state = read();
          if (command === "catalog_import_kubeconfig") {
            if (request.path === "/invalid.yaml") throw "invalidKubeconfig";
            if (request.path === "/fixtures/broken-link") throw "inputUnavailable";
            state.clusters.push({ id: "cluster-import", name: "Imported context", contextName: "Imported context", source: "kubeconfig", labels: [`file=${request.path}`], color: "#228877", status: "disconnected", pinned: false });
          } else if (command === "catalog_set_pin") {
            const item = state.clusters.find((item: { id: string }) => item.id === request.id);
            if (!item) throw "notFound";
            item.pinned = request.pinned;
          } else if (command === "catalog_remove") {
            state.clusters = state.clusters.filter((item: { id: string }) => item.id !== request.id);
          } else if (command !== "catalog_list") throw "unsupported";
          state.discoveryError = options.discoveryFailure ? "inputUnavailable" : null;
          if (command === "catalog_list" && options.defaultConfig && !options.discoveryFailure && !state.clusters.some((item: { id: string }) => item.id === "cluster-discovered")) {
            state.clusters.push({ id: "cluster-discovered", name: "Default context", contextName: "Default context", source: "kubeconfig", labels: ["file=~/.kube/config"], color: "#659040", status: "disconnected", pinned: false });
          }
          if (command !== "catalog_list") state.revision++;
          sessionStorage.setItem("test.catalog", JSON.stringify(state));
          return state;
        },
      },
    });
  }, options);
  await page.setViewportSize({ width: 1100, height: 700 });
}

test("Catalog home shows only clusters with unchanged columns and standalone Secrets navigation", async ({ page }, testInfo) => {
  await engineFixture(page);
  await page.goto("/");
  await expect(page).toHaveURL(/#\/catalog$/);
  const table = page.getByRole("table", { name: "Clusters", exact: true });
  await expect(table.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
  await expect(table.getByRole("columnheader")).toHaveText(["Name", "Kind", "Source", "Labels", "Status", "Actions"]);
  await expect(table.getByRole("cell", { name: "file=/fixtures/fixture.yaml", exact: true })).toBeVisible();
  await expect(table.getByRole("cell", { name: "Clusters", exact: true }).locator("svg")).toHaveCount(1);
  await expect(table.getByRole("cell", { name: "Disconnected", exact: true }).locator(".astryx-badge")).toHaveCount(1);
  const navigation = page.getByRole("navigation", { name: "Catalog navigation", exact: true });
  await expect(navigation.getByRole("link")).toHaveCount(2);
  await expect(navigation.getByRole("link", { name: "Clusters", exact: true })).toHaveAttribute("href", "/#/catalog/clusters");
  for (const name of ["All Categories", "Web Links", "General"]) await expect(navigation.getByRole("link", { name, exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Add web link", exact: true })).toHaveCount(0);
  const snapshot = await page.evaluate(() => JSON.parse(sessionStorage.getItem("test.catalog")!));
  expect(snapshot).not.toHaveProperty("webLinks");
  await page.screenshot({ path: testInfo.outputPath("catalog-desktop.png") });
});

for (const route of ["all", "general", "web-links"]) {
  test(`legacy category ${route} redirects to Clusters`, async ({ page }) => {
    await engineFixture(page);
    await page.goto(`/#/catalog/${route}`);
    await expect(page).toHaveURL(/#\/catalog\/clusters$/);
    await expect(page.getByRole("table", { name: "Clusters", exact: true })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Clusters", level: 1, exact: true })).toBeAttached();
    await expect(page.getByRole("button", { name: "Add web link", exact: true })).toHaveCount(0);
    expect(await page.evaluate(() => (window as unknown as { __catalogCalls: { command: string }[] }).__catalogCalls.some(call => /web_link/.test(call.command)))).toBe(false);
  });
}

test("individual row pins persist and sidebar reflects them; categories never have pin actions", async ({ page }) => {
  await engineFixture(page);
  await page.goto("/");
  const table = page.getByRole("table", { name: "Clusters", exact: true });
  await table.getByRole("button", { name: "Pin Development", exact: true }).click();
  await expect(page.getByRole("link", { name: "Development", exact: true })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Catalog navigation", exact: true })).toHaveAttribute("data-collapsed", "true");
  await expect(page.getByRole("button", { name: /^(Collapse|Expand) sidebar$/ })).toHaveCount(0);
  const pinnedLink = page.getByRole("link", { name: "Development", exact: true });
  await expect(pinnedLink).toBeVisible();
  await expect(pinnedLink).toHaveAttribute("aria-label", "Development");
  await expect(page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("button", { name: "Unpin Development", exact: true })).toHaveCount(0);
  await expect(pinnedLink.locator("[data-record-color]")).toHaveAttribute("data-record-color", "#476BA1");
  await pinnedLink.click();
  await expect(page).toHaveURL(/cluster\/cluster-fixture$/);
  await page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: "Clusters", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Catalog navigation", exact: true })).toHaveAttribute("data-collapsed", "true");
  await page.getByRole("link", { name: "Clusters", exact: true }).click();
  await page.reload();
  await expect(table.getByRole("button", { name: "Unpin Development", exact: true })).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("link", { name: "Development", exact: true }).click();
  await expect(page).toHaveURL(/cluster\/cluster-fixture$/);
  await expect(page.getByRole("heading", { level: 1, name: /^Development · / })).toBeAttached();
  await page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: "Clusters", exact: true }).click();
  await page.getByRole("link", { name: "Clusters", exact: true }).click();
  await table.getByRole("button", { name: "Unpin Development", exact: true }).click();
  await expect(page.getByRole("link", { name: "Development", exact: true })).toHaveCount(0);
  for (const name of ["Clusters"]) await expect(page.getByRole("button", { name: `Pin ${name}`, exact: true })).toHaveCount(0);
});

test("explicit import populates clusters; removal confirms local scope and persists", async ({ page }) => {
  await engineFixture(page, { empty: true });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Your catalog is empty", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Add config", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Import context metadata.");
  await dialog.getByRole("textbox", { name: "Kubeconfig path Required", exact: true }).fill("/fixtures/config-link");
  await dialog.getByRole("button", { name: "Import contexts", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("cell", { name: "Imported context", exact: true })).toBeVisible();
  await expect(page.getByRole("cell", { name: "file=/fixtures/config-link", exact: true })).toBeVisible();
  await page.getByRole("textbox", { name: "Filter catalog", exact: true }).fill("unmatched");
  await expect(page.getByRole("heading", { name: "No matching items", exact: true })).toBeVisible();
  await page.getByRole("textbox", { name: "Filter catalog", exact: true }).fill("");
  await page.getByRole("button", { name: "Remove Imported context", exact: true }).click();
  await expect(dialog).toContainText("The kubeconfig and cluster will not be changed.");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("cell").filter({ hasText: "Imported context" })).toBeVisible();
  await page.getByRole("button", { name: "Remove Imported context", exact: true }).click();
  await dialog.getByRole("button", { name: "Remove from catalog", exact: true }).click();
  await expect(page.getByRole("cell").filter({ hasText: "Imported context" })).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("heading", { name: "Your catalog is empty", exact: true })).toBeVisible();
});

test("invalid imports stay in dialog with safe errors and preserve records", async ({ page }) => {
  await engineFixture(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Add config", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("textbox", { name: "Kubeconfig path Required", exact: true }).fill("/invalid.yaml");
  await dialog.getByRole("button", { name: "Import contexts", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText("not a valid kubeconfig");
  await expect(dialog.getByRole("alert")).not.toContainText("/invalid.yaml");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
});

test("loading and corrupt storage are distinct from an empty catalog", async ({ page }) => {
  await engineFixture(page, { failList: true, holdList: true });
  await page.goto("/#/catalog");
  await expect(page.getByRole("status", { name: "Loading catalog…", exact: true })).toBeVisible();
  await page.evaluate(() => (window as Window & { __releaseCatalogList: () => void }).__releaseCatalogList());
  await expect(page.getByRole("alert")).toContainText("has not been overwritten");
  await expect(page.getByRole("heading", { name: "Catalog unavailable", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Add config", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Refresh catalog", exact: true })).toBeEnabled();
});

test("browser fallback disables persistence without inventing records", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 700 });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Desktop engine required", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Add config", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Add web link", exact: true })).toHaveCount(0);
  await expect(page.getByRole("alert")).toContainText("desktop app only");
});


test("failed refresh retains the last loaded records and redacts unknown errors", async ({ page }) => {
  await engineFixture(page);
  await page.goto("/");
  await expect(page.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { __TAURI_INTERNALS__: { invoke: () => Promise<never> } }).__TAURI_INTERNALS__.invoke = async () => { throw "SECRET_TOKEN fixture error"; };
  });
  await page.getByRole("button", { name: "Refresh catalog", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Previously loaded records are still shown");
  await expect(page.getByRole("alert")).not.toContainText("SECRET_TOKEN");
  await expect(page.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
});

test("populated catalog stays inside narrow and enlarged-text viewports", async ({ page }, testInfo) => {
  await engineFixture(page);
  await page.goto("/");
  await expect(page.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
  for (const width of [360, 800]) {
    await page.setViewportSize({ width, height: 700 });
    await page.evaluate(() => { document.documentElement.style.fontSize = "200%"; });
    if (width < 640) {
      await page.getByRole("button", { name: "Toolbar actions", exact: true }).click();
      await expect(page.getByRole("menuitem", { name: "Search", exact: true })).toBeVisible();
      await page.keyboard.press("Escape");
    } else await expect(page.getByRole("button", { name: "Search", exact: true })).toBeVisible();
    await expect(page.getByRole("table", { name: "Clusters", exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
  }
  await page.screenshot({ path: testInfo.outputPath("catalog-enlarged.png") });
});

test("default discovery is built in without a visible title, switch, or information block", async ({ page }) => {
  await engineFixture(page, { empty: true, defaultConfig: true });
  await page.goto("/");
  await expect(page.getByRole("cell", { name: "Default context", exact: true })).toBeVisible();
  await expect(page.getByRole("cell", { name: "file=~/.kube/config", exact: true })).toBeVisible();
  await expect(page.getByRole("switch")).toHaveCount(0);
  await expect(page.getByText("Imported clusters stay disconnected.", { exact: false })).toHaveCount(0);
  const heading = page.getByRole("heading", { level: 1, name: "Clusters", exact: true });
  expect(await heading.evaluate((element) => element.parentElement!.getBoundingClientRect().height)).toBeLessThanOrEqual(1);
  const toolbar = page.getByRole("navigation", { name: "App toolbar", exact: true });
  for (const name of ["Search", "Refresh catalog"]) await expect(toolbar.getByRole("button", { name, exact: true })).toBeVisible();
  for (const name of ["Add config"]) {
    await expect(toolbar.getByRole("button", { name, exact: true })).toHaveCount(0);
    await expect(page.getByRole("main").getByRole("button", { name, exact: true })).toBeVisible();
  }
  expect(await page.evaluate(() => (window as unknown as { __catalogCalls: { command: string }[] }).__catalogCalls.every((call) => call.command === "catalog_list"))).toBe(true);
  await page.reload();
  await expect(page.getByRole("cell", { name: "Default context", exact: true })).toBeVisible();
});

test("discovery failures preserve records and show only an actionable error", async ({ page }) => {
  await engineFixture(page, { discoveryFailure: true });
  await page.goto("/");
  await expect(page.getByRole("alert")).toContainText("Could not refresh file=~/.kube/config");
  await expect(page.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Refresh catalog", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Previously saved records are kept");
  await expect(page.getByRole("switch")).toHaveCount(0);
});

test("record icons retain colors across reloads and pins; labels are searchable", async ({ page }) => {
  await engineFixture(page);
  await page.goto("/");
  const row = page.getByRole("row").filter({ has: page.getByRole("button", { name: "Pin Development", exact: true }) });
  const icon = row.locator("[data-record-color]");
  await expect(icon).toHaveAttribute("data-record-color", "#476BA1");
  await expect(icon.locator("svg")).toHaveCount(1);
  await expect(icon).toHaveCSS("color", "rgb(74, 109, 162)");
  await expect(icon).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(icon).toHaveCSS("padding", "0px");
  await expect(icon).toHaveCSS("flex-shrink", "0");
  const displayColor = await icon.evaluate((element) => getComputedStyle(element).color);
  await expect(icon.locator("svg")).toHaveCSS("color", displayColor);
  const dimensions = await icon.locator("svg").evaluate((element) => {
    const { width, height } = element.getBoundingClientRect();
    return { width, height };
  });
  expect(dimensions.width).toBeGreaterThan(0);
  expect(dimensions.height).toBe(dimensions.width);
  await expect(row.getByRole("cell", { name: "Clusters", exact: true }).locator("svg")).toHaveCSS("width", `${dimensions.width}px`);
  await row.getByRole("button", { name: "Pin Development", exact: true }).click();
  await expect(page.getByRole("link", { name: "Development", exact: true }).locator("[data-record-color]")).toHaveAttribute("data-record-color", "#476BA1");
  const pinnedIcon = page.getByRole("link", { name: "Development", exact: true }).locator("[data-record-color]");
  await expect(pinnedIcon).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(pinnedIcon.locator("svg")).toHaveCSS("color", displayColor);
  await expect(pinnedIcon.locator("svg")).toHaveCSS("width", `${dimensions.width}px`);
  await expect(pinnedIcon.locator("svg")).toHaveCSS("height", `${dimensions.height}px`);
  await page.reload();
  const persistedIcon = page.getByRole("table").locator('[data-record-color="#476BA1"]');
  await expect(persistedIcon).toBeVisible();
  await expect(persistedIcon).toHaveCSS("color", displayColor);
  await expect(persistedIcon).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await page.getByRole("textbox", { name: "Filter catalog", exact: true }).fill("file=/fixtures");
  await expect(page.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
  await page.getByRole("link", { name: "Development", exact: true }).click();
  const sidebar = page.getByRole("navigation", { name: "Cluster navigation", exact: true });
  const selectorIcon = page.getByRole("button", { name: "Switch cluster", exact: true }).locator("[data-record-color]");
  await expect(selectorIcon).toHaveAttribute("data-record-color", "#476BA1");
  await expect(selectorIcon).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(selectorIcon).toHaveCSS("flex-shrink", "0");
  await expect(selectorIcon.locator("svg")).toHaveCSS("color", displayColor);
  await expect(selectorIcon.locator("svg")).toHaveCSS("width", `${dimensions.width}px`);
  await expect(selectorIcon.locator("svg")).toHaveCSS("height", `${dimensions.height}px`);
  await expect(sidebar.getByText("Saved context", { exact: true })).toHaveCount(0);
  await expect(sidebar.getByText("Context:", { exact: false })).toHaveCount(0);
  await page.setViewportSize({ width: 360, height: 700 });
  await page.getByRole("button", { name: "Open sidebar", exact: true }).click();
  const drawerIcon = page.getByRole("dialog").getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: "Development", exact: true }).locator("[data-record-color]");
  await expect(drawerIcon.locator("svg")).toHaveCSS("width", `${dimensions.width}px`);
  await expect(drawerIcon.locator("svg")).toHaveCSS("height", `${dimensions.height}px`);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(360);
});

test("toolbar cluster selector truncates long names and keeps the full name available", async ({ page }) => {
  await engineFixture(page);
  await page.goto("/");
  await expect(page.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
  const name = "Development-".repeat(20);
  await page.evaluate((name) => {
    const catalog = JSON.parse(sessionStorage.getItem("test.catalog")!);
    catalog.clusters[0].name = name;
    sessionStorage.setItem("test.catalog", JSON.stringify(catalog));
  }, name);
  await page.goto("/#/cluster/cluster-fixture");
  await page.reload();
  const selector = page.getByRole("button", { name: "Switch cluster", exact: true });
  const label = selector.getByText(name, { exact: true });
  await expect(label).toBeVisible();
  expect(await label.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
  await selector.hover();
  await expect(page.getByRole("tooltip", { name: `Switch cluster · ${name}`, exact: true })).toBeVisible();
  await page.setViewportSize({ width: 360, height: 700 });
  await expect(selector).toBeVisible();
  await selector.hover();
  await expect(page.getByRole("tooltip", { name: `Switch cluster · ${name}`, exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(360);
});

test("bottom catalog actions support keyboard dialogs while Refresh stays after forward", async ({ page }) => {
  await engineFixture(page);
  await page.setViewportSize({ width: 360, height: 700 });
  await page.goto("/");
  const toolbar = page.getByRole("navigation", { name: "App toolbar", exact: true });
  await expect(toolbar.getByRole("button", { name: "Catalog actions", exact: true })).toHaveCount(0);
  await toolbar.getByRole("button", { name: "Toolbar actions", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Search", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  for (const [action, title] of [["Add config", "Import kubeconfig"]]) {
    const trigger = page.getByRole("main").getByRole("button", { name: action, exact: true });
    await expect(toolbar.getByRole("button", { name: action, exact: true })).toHaveCount(0);
    await trigger.focus();
    await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: title, exact: true })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Import contexts", exact: true })).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(trigger).toBeFocused();
  }
  await toolbar.getByRole("button", { name: "Refresh catalog", exact: true }).click();
  await expect(page.getByRole("cell", { name: "Development", exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __catalogCalls: { command: string }[] }).__catalogCalls.filter((call) => call.command === "catalog_list").length)).toBeGreaterThanOrEqual(2);
});

for (const [route, category, tableName] of [
  ["clusters", "Clusters", "Clusters"],
]) {
  test(`${category}: bottom actions clear the table and viewport at desktop and narrow widths`, async ({ page }) => {
    await engineFixture(page);
    await page.goto(`/#/catalog/${route}`);
    const main = page.getByRole("main");
    await expect(main.getByRole("table", { name: tableName, exact: true })).toBeVisible();
    for (const width of [1100, 800, 360]) {
      await page.setViewportSize({ width, height: 700 });
      const toolbar = page.getByRole("navigation", { name: "App toolbar", exact: true });
      await expect(toolbar.getByRole("button", { name: "Refresh catalog", exact: true })).toBeVisible();
      await expect(toolbar.getByRole("button", { name: "Catalog actions", exact: true })).toHaveCount(0);
      const mainBounds = (await main.boundingBox())!;
      const filterBounds = (await main.getByRole("textbox", { name: "Filter catalog", exact: true }).boundingBox())!;
      const buttons = [];
      for (const name of ["Add config"]) {
        const action = main.getByRole("button", { name, exact: true });
        await expect(action).toBeVisible();
        await expect(action).toHaveAccessibleName(name);
        await expect(toolbar.getByRole("button", { name, exact: true })).toHaveCount(0);
        const bounds = (await action.boundingBox())!;
        expect(bounds.width).toBeGreaterThanOrEqual(44);
        expect(bounds.height).toBeGreaterThanOrEqual(44);
        expect(bounds.y).toBeGreaterThan(filterBounds.y + filterBounds.height);
        expect(bounds.x).toBeGreaterThanOrEqual(mainBounds.x);
        expect(bounds.x + bounds.width).toBeLessThanOrEqual(mainBounds.x + mainBounds.width);
        expect(bounds.y + bounds.height).toBeLessThanOrEqual(mainBounds.y + mainBounds.height);
        expect(bounds.y + bounds.height).toBeGreaterThan(mainBounds.y + mainBounds.height - 48);
        expect(await action.evaluate(element => {
          const bounds = element.getBoundingClientRect();
          return element.contains(document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2));
        })).toBe(true);
        buttons.push(bounds);
      }
      expect(mainBounds.x + mainBounds.width - buttons[0].x - buttons[0].width).toBeGreaterThan(0);
      await expect(page.getByRole("button", { name: "Add web link", exact: true })).toHaveCount(0);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    }
  });
}
