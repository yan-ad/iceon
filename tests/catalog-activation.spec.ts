import { expect, test, type Page } from "@playwright/test";

async function fixture(page: Page, options: { empty?: boolean; slow?: boolean } = {}) {
  await page.addInitScript((options) => {
    const calls: { command: string; request?: Record<string, unknown> }[] = [];
    const state = {
      revision: 1, discoveryError: null,
      clusters: options.empty ? [] : [
        { id: "cluster-one", name: "Development", contextName: "dev-context", source: "kubeconfig", labels: ["file=/fixtures/config"], color: "#476BA1", status: "disconnected", pinned: true },
        { id: "cluster-two", name: "Staging", contextName: "stage-context", source: "kubeconfig", labels: ["file=/fixtures/config"], color: "#659040", status: "disconnected", pinned: true },
      ],
    };
    Object.assign(window, {
      isTauri: true, __activationCalls: calls,
      __TAURI_INTERNALS__: { invoke: async (command: string, args?: { request: Record<string, unknown> }) => {
        calls.push({ command, request: args?.request });
        if (options.slow) await new Promise((resolve) => setTimeout(resolve, 250));
        if (command === "cluster_permissions_review") {
          const review = args?.request as unknown as { clusterId: string; sessionId: string; readId: number; checks: unknown[] };
          const checkedAt = Date.now();
          return { clusterId: review.clusterId, sessionId: review.sessionId, readId: review.readId,
            authorizationEpoch: `fixture-${review.sessionId}`, revision: review.readId, checkedAt, expiresAt: checkedAt + 30_000,
            items: review.checks.map(check => ({ check, decision: "allowed", reason: null })) };
        }
        if (command === "cluster_permissions_cancel" || command === "cluster_permissions_refresh") return null;
        if (command === "cluster_session_open") return { clusterId: args?.request.clusterId, sessionId: crypto.randomUUID(), contextName: "dev-context", status: "disconnected", apiVersion: null, error: null };
        if (command === "cluster_connect") return { clusterId: args?.request.clusterId, sessionId: args?.request.sessionId, contextName: "dev-context", status: "connected", apiVersion: "v1.34.1", error: null };
        if (command === "cluster_resources") return { ...args?.request, namespace: args?.request.namespace ?? null, continueToken: null, metrics: null, items: [], truncated: false, fetchedAt: Date.now() };
        if (command === "cluster_disconnect" || command === "cluster_resources_cancel") return;
        if (command === "catalog_set_pin") {
          const item = state.clusters.find((item) => item.id === args?.request.id);
          if (item) item.pinned = !!args?.request.pinned;
          state.revision++;
        }
        return structuredClone(state);
      } },
    });
  }, options);
  await page.setViewportSize({ width: 1200, height: 800 });
}

async function calls(page: Page) {
  return page.evaluate(() => (window as unknown as { __activationCalls: { command: string; request?: Record<string, unknown> }[] }).__activationCalls);
}

async function expectCollapsedCatalog(page: Page) {
  const sidebar = page.getByRole("navigation", { name: "Catalog navigation", exact: true });
  await expect(sidebar).toBeVisible();
  await expect(sidebar).toHaveAttribute("data-collapsed", "true");
  await expect(sidebar.getByRole("link", { name: "Clusters", exact: true })).toHaveAttribute("aria-label", "Clusters");
  await expect(page.getByRole("button", { name: /^(Collapse|Expand) sidebar$/ })).toHaveCount(0);
  await expect(page.getByRole("navigation", { name: "App toolbar", exact: true }).getByRole("button", { name: "Open sidebar", exact: true })).toHaveCount(0);
}

test("cluster row opens inset workspace while Catalog stays collapsed", async ({ page }, testInfo) => {
  await fixture(page);
  await page.goto("/");
  await expectCollapsedCatalog(page);
  await page.getByRole("row", { name: "Open cluster Development", exact: true }).getByRole("cell", { name: "kubeconfig", exact: true }).click();
  await expect(page).toHaveURL(/#\/cluster\/cluster-one$/);
  await expect(page.getByRole("heading", { level: 1, name: /^Development · / })).toBeAttached();
  await expect(page.getByRole("button", { name: "Switch cluster", exact: true })).toContainText("Development");
  await expectCollapsedCatalog(page);
  const inset = page.getByRole("navigation", { name: "Cluster navigation", exact: true });
  await expect(inset).toBeVisible();
  await expect(inset.getByRole("link", { name: "Nodes", exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Cluster overview", exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("cluster-workspace.png") });
  await inset.getByRole("link", { name: "Nodes", exact: true }).click();
  await expect(page).toHaveURL(/section=nodes$/);
  await expect(inset.getByRole("link", { name: "Nodes", exact: true })).toHaveAttribute("aria-current", "page");
  for (const name of ["Workloads", "Network", "Storage", "Helm"]) {
    await inset.getByRole("button", { name: `Expand ${name}`, exact: true }).click();
    await expect(inset.getByRole("button", { name: `Collapse ${name}`, exact: true })).toHaveAttribute("aria-expanded", "true");
    await expect(page).toHaveURL(/section=nodes$/);
    await expect(page.getByRole("main")).toContainText("No nodes were returned");
    await expect(page.getByRole("navigation", { name: `${name} destinations`, exact: true })).toHaveCount(0);
  }
  expect((await calls(page)).filter((call) => !["catalog_list", "cluster_session_open", "cluster_connect", "cluster_disconnect", "cluster_resources", "cluster_resources_cancel", "cluster_permissions_review", "cluster_permissions_cancel", "cluster_permissions_refresh", "catalog_connection_states", "cluster_namespace_cache_get"].includes(call.command))).toEqual([]);
  await page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: "Clusters", exact: true }).click();
  await expect(page.getByRole("table", { name: "Clusters", exact: true })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Cluster navigation", exact: true })).toHaveCount(0);
  await expectCollapsedCatalog(page);
});

test("row Enter/Space and pinned cluster activation work; pin/remove buttons do not activate rows", async ({ page }) => {
  await fixture(page);
  await page.goto("/");
  await page.getByRole("row", { name: "Open cluster Development", exact: true }).getByRole("button", { name: "Unpin Development", exact: true }).click();
  await expect(page).toHaveURL(/#\/catalog$/);
  await page.getByRole("button", { name: "Remove Development", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page).toHaveURL(/#\/catalog$/);
  await page.getByRole("dialog").getByRole("button", { name: "Cancel", exact: true }).click();
  const row = page.getByRole("row", { name: "Open cluster Development", exact: true });
  await row.focus(); await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/cluster\/cluster-one$/);
  await page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: "Clusters", exact: true }).click();
  await row.focus(); await page.keyboard.press("Space");
  await expect(page).toHaveURL(/cluster\/cluster-one$/);
  await page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: "Staging", exact: true }).click();
  await expect(page).toHaveURL(/cluster\/cluster-two$/);
  await expect(page.getByRole("heading", { level: 1, name: /^Staging · / })).toBeAttached();
  await expect(page.getByRole("button", { name: "Switch cluster", exact: true })).toContainText("Staging");
});

test("direct missing cluster route does not select another cluster", async ({ page }) => {
  await fixture(page);
  await page.goto("/#/cluster/missing");
  await expect(page.getByRole("main").getByRole("heading", { name: "Cluster not found", exact: true })).toBeVisible();
  await expect(page.getByRole("main").getByRole("heading", { name: "Development", exact: true })).toHaveCount(0);
  await expectCollapsedCatalog(page);
});

test("cluster navigation is available in the narrow-window drawer", async ({ page }) => {
  await fixture(page);
  await page.setViewportSize({ width: 360, height: 700 });
  await page.goto("/#/cluster/cluster-one");
  await expect(page.getByRole("heading", { level: 1, name: /^Development · / })).toBeAttached();
  const rail = page.locator("#catalog-sidebar-narrow");
  await expect(rail).toBeVisible();
  await expect(rail).toHaveAttribute("data-collapsed", "true");
  await expect(page.getByRole("navigation", { name: "App toolbar", exact: true }).getByRole("button", { name: "Open sidebar", exact: true })).toHaveCount(0);
  await rail.getByRole("button", { name: "Open sidebar", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("navigation", { name: "Cluster navigation", exact: true })).toBeVisible();
  await dialog.getByRole("navigation", { name: "Cluster navigation", exact: true }).getByRole("link", { name: "Events", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("table", { name: "Events", exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(360);
});

for (const preference of ["false", "true"]) {
  test(`Catalog stays collapsed through workspace, history and reload with stored preference ${preference}`, async ({ page }) => {
    await fixture(page);
    await page.addInitScript(value => {
      if (localStorage.getItem("iceon.sidebar-collapsed.v1") === null) localStorage.setItem("iceon.sidebar-collapsed.v1", value);
    }, preference);
    await page.goto("/");
    await expectCollapsedCatalog(page);
    await page.getByRole("row", { name: "Open cluster Development", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expectCollapsedCatalog(page);
    await page.getByRole("navigation", { name: "Cluster navigation", exact: true }).getByRole("link", { name: "Nodes", exact: true }).click();
    await page.getByRole("button", { name: "Go back", exact: true }).click();
    await expect(page).toHaveURL(/cluster\/cluster-one$/);
    await expectCollapsedCatalog(page);
    await page.getByRole("button", { name: "Go forward", exact: true }).click();
    await expect(page).toHaveURL(/section=nodes$/);
    await expectCollapsedCatalog(page);
    await page.reload();
    await expect(page).toHaveURL(/section=nodes$/);
    await expectCollapsedCatalog(page);
    await page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: "Staging", exact: true }).click();
    await expectCollapsedCatalog(page);
    await page.getByRole("navigation", { name: "Catalog navigation", exact: true }).getByRole("link", { name: "Clusters", exact: true }).click();
    await expectCollapsedCatalog(page);
    await page.reload();
    await expectCollapsedCatalog(page);
    expect(await page.evaluate(() => localStorage.getItem("iceon.sidebar-collapsed.v1"))).toBe(preference);
  });
}

test("cluster-only catalog exposes no link activation or creation actions", async ({ page }) => {
  await fixture(page);
  await page.goto("/");
  await expect(page.getByRole("row", { name: "Open cluster Development", exact: true })).toBeVisible();
  await expect(page.getByRole("row", { name: /^Open web link / })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Add web link", exact: true })).toHaveCount(0);
  const sidebar = page.getByRole("navigation", { name: "Catalog navigation", exact: true });
  await expect(sidebar.getByRole("link")).toHaveCount(4);
  for (const name of ["Clusters", "Secrets", "Development", "Staging"]) {
    await expect(sidebar.getByRole("link", { name, exact: true })).toBeVisible();
  }
  await expect(sidebar.getByRole("link", { name: "Secrets", exact: true })).toHaveAttribute("href", "/#/secrets");
  expect((await calls(page)).some(call => /web_link/.test(call.command))).toBe(false);
});
