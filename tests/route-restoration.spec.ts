import { expect, test, type Page } from "@playwright/test";
import { defaultPreferences } from "../src/app/preferences/api";

async function fixture(page: Page, route: unknown, enabled = true, restoreRoute = true, delay = 0) {
  await page.addInitScript(({ route, enabled, restoreRoute, delay, defaults }) => {
    const calls: { command: string; route?: unknown }[] = [];
    Object.assign(window, {
      isTauri: true,
      __ICEON_WINDOW_LAYOUT__: { seamless: false, platform: "linux", restoreRoute },
      __routeHistory: calls,
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        transformCallback: () => 1,
        invoke: async (command: string, args?: { route?: unknown }) => {
          calls.push({ command, route: args?.route });
          if (command === "plugin:event|listen") return 1;
          if (command === "preferences_get") return { revision: 1, preferences: { ...defaults, app: { ...defaults.app, restoreLastPage: enabled } } };
          if (command === "route_history_get") {
            await new Promise(resolve => setTimeout(resolve, delay));
            return route;
          }
          if (command === "catalog_list") return { revision: 1, clusters: [], discoveryError: null };
          if (command === "catalog_connection_states") return [];
          if (command === "update_state") return { phase: "idle", version: null, error: null };
          return null;
        },
      },
    });
  }, { route, enabled, restoreRoute, delay, defaults: defaultPreferences });
}
const calls = (page: Page) => page.evaluate(() => (window as unknown as { __routeHistory: { command: string; route?: unknown }[] }).__routeHistory);

test("route restoration is opt-in without history IPC by default", async ({ page }) => {
  await fixture(page, { path: "/catalog/clusters" }, false);
  await page.goto("/");
  await expect(page).toHaveURL(/#\/catalog$/);
  await page.waitForTimeout(350);
  expect((await calls(page)).filter(c => c.command.startsWith("route_history"))).toEqual([]);
});

test("restores a saved Catalog destination and remembers Clusters navigation", async ({ page }) => {
  await fixture(page, { path: "/catalog" });
  await page.goto("/");
  await expect(page).toHaveURL(/#\/catalog$/);
  await page.getByRole("link", { name: "Clusters", exact: true }).click();
  await expect(page).toHaveURL(/#\/catalog\/clusters$/);
  await expect.poll(async () => (await calls(page)).filter(c => c.command === "route_history_save").at(-1)?.route).toEqual({ path: "/catalog/clusters" });
});

for (const route of [{ path: "https://evil.test/" }, { path: "/catalog?token=secret" }, { path: "/cluster/missing", section: "pods" }, { path: "/catalog/web-links", secret: "untrusted" }]) {
  test(`ignores an invalid or missing destination ${JSON.stringify(route)}`, async ({ page }) => {
    await fixture(page, route);
    await page.goto("/");
    await page.waitForTimeout(350);
    await expect(page).toHaveURL(/#\/catalog$/);
    expect((await calls(page)).some(c => c.command === "cluster_session_open")).toBe(false);
  });
}

test("explicit deep link wins over saved route", async ({ page }) => {
  await fixture(page, { path: "/catalog" });
  await page.goto("/#/catalog/clusters");
  await expect(page).toHaveURL(/#\/catalog\/clusters$/);
  await page.waitForTimeout(350);
  expect((await calls(page)).some(c => c.command === "route_history_get")).toBe(false);
});

test("new Catalog window does not restore", async ({ page }) => {
  await fixture(page, { path: "/catalog/clusters" }, true, false);
  await page.goto("/");
  await page.waitForTimeout(350);
  await expect(page).toHaveURL(/#\/catalog$/);
  expect((await calls(page)).some(c => c.command === "route_history_get")).toBe(false);
});

test("restores a saved Secrets destination", async ({ page }) => {
  await fixture(page, { path: "/secrets" });
  await page.goto("/");
  await expect(page).toHaveURL(/#\/secrets$/);
  await expect(page.getByRole("heading", { name: "Secrets", exact: true })).toBeAttached();
});

test("navigation while history is loading is not overwritten", async ({ page }) => {
  await fixture(page, { path: "/catalog" }, true, true, 600);
  await page.goto("/");
  await expect.poll(async () => (await calls(page)).some(c => c.command === "route_history_get")).toBe(true);
  await page.getByRole("link", { name: "Clusters", exact: true }).click();
  await page.waitForTimeout(700);
  await expect(page).toHaveURL(/#\/catalog\/clusters$/);
});
