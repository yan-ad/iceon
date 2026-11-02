import { expect, test } from "@playwright/test";

test("Catalog refresh follows history and pauses while an import dialog is open", async ({ page }) => {
  await page.addInitScript(() => {
    Object.assign(window, { isTauri: true, __catalogReads: 0, __TAURI_INTERNALS__: {
      invoke: async (command: string) => {
        if (command === "catalog_list") {
          (window as any).__catalogReads += 1;
          return { revision: 1, discoveryError: null, clusters: [], webLinks: [] };
        }
        return null;
      },
    } });
  });
  await page.goto("/#/catalog");
  const refresh = page.getByRole("button", { name: "Refresh catalog", exact: true });
  await expect(refresh).toBeEnabled();
  await expect(page.locator('.toolbar-history button').nth(1)).toHaveAccessibleName("Go forward");
  await expect(page.locator('.toolbar-history button').nth(2)).toHaveAccessibleName("Refresh catalog");
  const reads = () => page.evaluate(() => (window as any).__catalogReads as number);
  const before = await reads();
  await refresh.click();
  await expect.poll(reads).toBe(before + 1);
  await page.getByRole("button", { name: "Add config", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "r", ctrlKey: true, bubbles: true, cancelable: true })));
  expect(await reads()).toBe(before + 1);
  await page.keyboard.press("Escape");
  await page.evaluate(() => {
    for (const overrides of [{ repeat: true }, { shiftKey: true }, { altKey: true }, { isComposing: true }]) {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "r", ctrlKey: true, bubbles: true, cancelable: true, ...overrides }));
    }
    const handled = new KeyboardEvent("keydown", { key: "r", ctrlKey: true, bubbles: true, cancelable: true });
    handled.preventDefault();
    window.dispatchEvent(handled);
  });
  expect(await reads()).toBe(before + 1);
  await page.evaluate(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "r", ctrlKey: true, bubbles: true, cancelable: true })));
  await expect.poll(reads).toBe(before + 2);
});

test("page refresh registrations retain independent owners and AddButton forwards its ref", async ({ page }) => {
  await page.route("**/universal-action-fixture", route => route.fulfill({ contentType: "text/html", body: `<!doctype html><html><body><main id="fixture"></main><script type="module">
    import RefreshRuntime from '/@react-refresh';
    RefreshRuntime.injectIntoGlobalHook(window);
    window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => type => type;
    window.__vite_plugin_react_preamble_installed__ = true;
    const { React, createRoot, Theme } = await import('/tests/helpers/updateFixtureModules.ts');
    const { WorkspaceRefreshProvider, useRegisterPageRefresh, useWorkspaceRefresh } = await import('/src/app/cluster/WorkspaceRefresh.tsx');
    const { AddButton } = await import('/src/app/components/AddButton.tsx');
    const { brandTheme } = await import('/src/themes/brand.ts');
    import '/node_modules/@astryxdesign/core/src/reset.css';
    import '/node_modules/@astryxdesign/core/dist/astryx.css';
    const e = React.createElement;
    function Register({ label }) {
      useRegisterPageRefresh({ scope: '/secrets', label, enabled: true, refresh: () => { window.__refreshed = label; } });
      return null;
    }
    function Fixture() {
      const [child, setChild] = React.useState(true);
      const { pageRefresh } = useWorkspaceRefresh();
      return e(React.Fragment, null,
        child && e(Register, { label: 'Refresh secrets' }),
        e(Register, { label: 'Refresh connections' }),
        e('button', { onClick: () => setChild(false) }, 'Remove child'),
        e('button', { onClick: () => pageRefresh?.refresh() }, pageRefresh?.label ?? 'No refresh'),
        e(AddButton, { label: 'Add fixture', style: { position: 'fixed', right: 'var(--spacing-4)', bottom: 'var(--spacing-4)' }, ref: node => { window.__addRef = node; } }));
    }
    createRoot(document.getElementById('fixture')).render(e(React.StrictMode, null, e(Theme, { theme: brandTheme }, e(WorkspaceRefreshProvider, null, e(Fixture)))));
  </script></body></html>` }));
  await page.goto("/universal-action-fixture");
  await expect(page.getByRole("button", { name: "Refresh connections" })).toBeVisible();
  await page.getByRole("button", { name: "Remove child" }).click();
  await page.getByRole("button", { name: "Refresh connections" }).click();
  expect(await page.evaluate(() => (window as any).__refreshed)).toBe("Refresh connections");
  const add = page.getByRole("button", { name: "Add fixture" });
  expect(await add.evaluate(node => node === (window as any).__addRef)).toBe(true);
  const bounds = (await add.boundingBox())!;
  expect(bounds.width).toBe(bounds.height);
  expect(bounds.width).toBeGreaterThanOrEqual(44);
  expect(await add.evaluate(node => getComputedStyle(node).position)).toBe("fixed");
});
