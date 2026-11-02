import { expect, test, type Page } from "@playwright/test";

async function fixture(page: Page, options: { delayReveal?: boolean; delayLogin?: boolean; delayOpen?: boolean; unavailable?: boolean; sessionDuration?: number; emptyFolder?: boolean; emptyProfiles?: boolean } = {}) {
  await page.addInitScript(options => {
    const profile = { id: "a".repeat(32), provider: "vault", name: "Fixture Vault", address: "https://vault.invalid", namespace: "", authMount: "userpass", kvMount: "secret", basePath: "" };
    const session = { profileId: profile.id, sessionId: "fixture-session", status: "signedOut", expiresAt: null, credentialStore: options.unavailable ? "unavailable" : "macosKeychain" };
    const calls: { command: string; args: any }[] = [];
    let resolveReveal: (() => void) | undefined;
    let resolveLogin: (() => void) | undefined;
    const openResolvers: (() => void)[] = [];
    Object.assign(window, {
      isTauri: true,
      __secretCalls: calls,
      __finishReveal: () => resolveReveal?.(),
      __finishLogin: () => resolveLogin?.(),
      __finishOpen: () => openResolvers.splice(0).forEach(resolve => resolve()),
      __TAURI_INTERNALS__: {
        transformCallback: () => 1,
        invoke: async (command: string, args: any) => {
          calls.push({ command, args: structuredClone(args) });

          if (command === "catalog_list") return { revision: 1, clusters: [], discoveryError: null };
          if (command === "secrets_profiles") return options.emptyProfiles ? [] : [profile];
          if (command === "secrets_profile_save") return { ...profile, ...args.request };
          if (command === "secrets_session_open") {
            if (options.delayOpen) await new Promise<void>(resolve => { openResolvers.push(resolve); });
            return session;
          }
          if (command === "secrets_login" || command === "secrets_resume") {
            if (options.delayLogin) await new Promise<void>(resolve => { resolveLogin = resolve; });
            return { ...session, status: "connected", expiresAt: Date.now() + (options.sessionDuration ?? 60_000) };
          }
          if (command === "secrets_list") return { ...args.request, entries: options.emptyFolder ? [] : args.request.path ? [{ name: "nested", directory: false }] : [{ name: "folder", directory: true }, { name: "sample", directory: false }], partial: false };
          if (command === "secrets_reveal") {
            if (options.delayReveal) await new Promise<void>(resolve => { resolveReveal = resolve; });
            return { ...args.request, fields: [{ name: "example", value: "SYNTHETIC_SECRET_VALUE" }], version: 1, expiresAt: Date.now() + 30_000 };
          }
          return null;
        },
      },
    });
  }, options);
}

async function openConnection(page: Page) {
  await page.goto("/#/secrets");
  await page.getByRole("button", { name: "Open Fixture Vault", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Username", exact: true })).toBeVisible();
}
async function login(page: Page) {
  await page.getByRole("textbox", { name: "Username", exact: true }).fill("fixture-user");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: "Log in", exact: true }).click();
}
async function calls(page: Page, command: string) {
  return page.evaluate(command => (window as any).__secretCalls.filter((call: any) => call.command === command), command);
}

test("Secrets is standalone sidebar navigation; browser fallback cannot invoke a provider", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("link", { name: "Secrets", exact: true }).click();
  await expect(page).toHaveURL(/#\/secrets$/);
  await expect(page.getByRole("heading", { name: "Secrets", exact: true })).toBeAttached();
  await expect(page.getByText("Browser preview cannot connect", { exact: false })).toBeVisible();
  await expect(page.getByRole("button", { name: "Add secret connection", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Preferences", exact: true }).click();
  await expect(page.getByRole("dialog").getByRole("button", { name: "Secrets management", exact: true })).toHaveCount(0);
});

test("plus opens a provider dialog and saves metadata without authentication", async ({ page }) => {
  await fixture(page);
  await page.goto("/#/secrets");
  const add = page.getByRole("button", { name: "Add secret connection", exact: true });
  await expect(add).toBeVisible();
  await add.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("textbox", { name: "Connection name Required", exact: true }).fill("New fixture");
  await dialog.getByRole("textbox", { name: "HTTPS endpoint Required", exact: true }).fill("https://another.invalid");
  await dialog.getByRole("button", { name: "Save connection", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Open New fixture", exact: true })).toBeVisible();
  expect(await calls(page, "secrets_login")).toHaveLength(0);
  expect((await calls(page, "secrets_profile_save"))[0].args.request.provider).toBe("vault");
  await expect(add).toBeVisible();
});

test("saved-connection empty state offers the same Add connection action", async ({ page }) => {
  await fixture(page, { emptyProfiles: true });
  await page.goto("/#/secrets");
  await expect(page.getByRole("heading", { name: "No saved connections", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Add connection", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Add secret connection", exact: true })).toBeVisible();
});

test("explicit login, browse, reveal, hide and disconnect use scoped IPC", async ({ page }) => {
  await fixture(page);
  await openConnection(page);
  await page.getByRole("textbox", { name: "Username", exact: true }).fill("fixture-user");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.getByRole("textbox", { name: "Username", exact: true }).focus();
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("fixture-password");
  expect(await calls(page, "secrets_reveal")).toHaveLength(0);
  await page.getByRole("button", { name: "Log in", exact: true }).click();
  await page.getByRole("button", { name: "Open folder", exact: true }).click();
  await expect(page.getByText("Current folder: /folder", { exact: true })).toBeVisible();
  expect((await calls(page, "secrets_list")).at(-1).args.request.path).toBe("folder");
  await page.getByRole("button", { name: "Reveal nested", exact: true }).click();
  await expect(page.getByText("SYNTHETIC_SECRET_VALUE", { exact: true })).toBeVisible();
  expect((await calls(page, "secrets_reveal")).at(-1).args.request.path).toBe("folder/nested");
  await page.getByRole("button", { name: "Hide values", exact: true }).click();
  await expect(page.getByText("SYNTHETIC_SECRET_VALUE", { exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Reveal nested", exact: true }).click();
  await expect(page.getByText("SYNTHETIC_SECRET_VALUE", { exact: true })).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(page.getByText("SYNTHETIC_SECRET_VALUE", { exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("");
  expect((await calls(page, "secrets_login"))[0].args.request.password).toBe("fixture-password");
  expect((await calls(page, "secrets_disconnect"))[0].args.request.forget).toBe(false);
});

test("late reveal cannot revive values after blur", async ({ page }) => {
  await fixture(page, { delayReveal: true });
  await openConnection(page); await login(page);
  await page.getByRole("button", { name: "Reveal sample", exact: true }).click();
  await expect.poll(async () => (await calls(page, "secrets_reveal")).length).toBe(1);
  await page.evaluate(() => { window.dispatchEvent(new Event("blur")); (window as any).__finishReveal(); });
  await expect(page.getByText("Reading provider…", { exact: true })).toHaveCount(0);
  await expect(page.getByText("SYNTHETIC_SECRET_VALUE", { exact: true })).toHaveCount(0);
  expect(await calls(page, "secrets_cancel")).toHaveLength(1);
});

test("disposal closes the owned session before a pending login resolves", async ({ page }) => {
  await fixture(page, { delayLogin: true });
  await openConnection(page);
  await page.getByRole("checkbox", { name: "Remember login", exact: true }).check();
  await login(page);
  await expect.poll(async () => (await calls(page, "secrets_login")).length).toBe(1);
  expect((await calls(page, "secrets_login"))[0].args.request.remember).toBe(true);
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("");
  const closedBefore = (await calls(page, "secrets_close")).length;
  await page.getByRole("button", { name: "Close connection", exact: true }).click();
  expect(await calls(page, "secrets_close")).toHaveLength(closedBefore + 1);
  await page.evaluate(() => (window as any).__finishLogin());
  await expect.poll(async () => (await calls(page, "secrets_close")).length).toBe(closedBefore + 1);
  expect(await calls(page, "secrets_list")).toHaveLength(0);
  await expect(page.getByRole("textbox", { name: "Username", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Open Fixture Vault", exact: true }).click();
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("");
  await expect(page.getByRole("checkbox", { name: "Remember login", exact: true })).not.toBeChecked();
  await expect(page.getByRole("button", { name: "Reveal sample", exact: true })).toHaveCount(0);
  expect(await calls(page, "secrets_list")).toHaveLength(0);
});

test("disposal closes the owned session before a pending resume resolves", async ({ page }) => {
  await fixture(page, { delayLogin: true });
  await openConnection(page);
  await page.getByRole("button", { name: "Resume saved login", exact: true }).click();
  await expect.poll(async () => (await calls(page, "secrets_resume")).length).toBe(1);
  const closedBefore = (await calls(page, "secrets_close")).length;
  await page.getByRole("button", { name: "Close connection", exact: true }).click();
  expect(await calls(page, "secrets_close")).toHaveLength(closedBefore + 1);
  await page.evaluate(() => (window as any).__finishLogin());
  await expect.poll(async () => (await calls(page, "secrets_close")).length).toBe(closedBefore + 1);
  expect(await calls(page, "secrets_list")).toHaveLength(0);
  await expect(page.getByRole("textbox", { name: "Username", exact: true })).toHaveCount(0);
});

test("late session open is closed after leaving the page", async ({ page }) => {
  await fixture(page, { delayOpen: true });
  await page.goto("/#/secrets");
  await page.getByRole("button", { name: "Open Fixture Vault", exact: true }).click();
  await expect.poll(async () => (await calls(page, "secrets_session_open")).length).toBeGreaterThan(0);
  const opened = (await calls(page, "secrets_session_open")).length;
  await page.getByRole("link", { name: "Clusters", exact: true }).click();
  await page.evaluate(() => (window as any).__finishOpen());
  await expect.poll(async () => (await calls(page, "secrets_close")).length).toBe(opened);
});

test("unavailable credential store disables remember and resume, not login", async ({ page }) => {
  await fixture(page, { unavailable: true });
  await openConnection(page);
  await expect(page.getByRole("checkbox", { name: "Remember login", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Resume saved login", exact: true })).toBeDisabled();
  await login(page);
  await expect(page.getByRole("button", { name: "Reveal sample", exact: true })).toBeVisible();
});

test("resume, forget, expiry and connection removal clear sensitive state", async ({ page }) => {
  await fixture(page);
  await openConnection(page);
  await page.clock.install();
  await page.getByRole("button", { name: "Resume saved login", exact: true }).click();
  await page.getByRole("button", { name: "Reveal sample", exact: true }).click();
  await expect(page.getByText("SYNTHETIC_SECRET_VALUE", { exact: true })).toBeVisible();
  await page.clock.fastForward(31_000);
  await expect(page.getByText("SYNTHETIC_SECRET_VALUE", { exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Forget saved login", exact: true }).click();
  expect((await calls(page, "secrets_disconnect"))[0].args.request.forget).toBe(true);
  await page.getByRole("button", { name: "Close connection", exact: true }).click();
  await page.getByRole("button", { name: "Remove Fixture Vault", exact: true }).click();
  await page.getByRole("button", { name: "Remove connection", exact: true }).click();
  await expect(page.getByRole("button", { name: "Open Fixture Vault", exact: true })).toHaveCount(0);
});

test("bounded narrow page keeps plus visible and dialog keyboard accessible", async ({ page }) => {
  await fixture(page);
  await page.setViewportSize({ width: 390, height: 500 });
  await page.goto("/#/secrets");
  const add = page.getByRole("button", { name: "Add secret connection", exact: true });
  await expect(add).toBeInViewport();
  await add.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeInViewport();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(add).toBeFocused();
});

test("password clears on window blur and does not survive reopening", async ({ page }) => {
  await fixture(page);
  await openConnection(page);
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: "Close connection", exact: true }).click();
  await page.getByRole("button", { name: "Open Fixture Vault", exact: true }).click();
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("");
});

test("pending reveal is cancelled on unmount and cannot revive values", async ({ page }) => {
  await fixture(page, { delayReveal: true });
  await openConnection(page); await login(page);
  await page.getByRole("button", { name: "Reveal sample", exact: true }).click();
  await expect.poll(async () => (await calls(page, "secrets_reveal")).length).toBe(1);
  const closedBefore = (await calls(page, "secrets_close")).length;
  await page.getByRole("link", { name: "Clusters", exact: true }).click();
  expect(await calls(page, "secrets_close")).toHaveLength(closedBefore + 1);
  expect(await calls(page, "secrets_cancel")).toHaveLength(1);
  await page.evaluate(() => (window as any).__finishReveal());
  await page.getByRole("link", { name: "Secrets", exact: true }).click();
  await expect(page.getByText("SYNTHETIC_SECRET_VALUE", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Close connection", exact: true })).toHaveCount(0);
});

test("long sessions rearm the browser timer until their actual expiry", async ({ page }) => {
  const duration = 3_000_000_000;
  await fixture(page, { sessionDuration: duration });
  await openConnection(page);
  await page.clock.install();
  await login(page);
  await expect(page.getByRole("button", { name: "Reveal sample", exact: true })).toBeVisible();
  await page.clock.fastForward(2_147_483_647);
  await expect(page.getByRole("button", { name: "Reveal sample", exact: true })).toBeVisible();
  await page.clock.fastForward(duration - 2_147_483_647);
  await expect(page.getByRole("button", { name: "Reveal sample", exact: true })).toHaveCount(0);
  await expect(page.getByText("Session: Expired.", { exact: false })).toBeVisible();
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("");
});

test("empty folders use the table empty state with a working refresh", async ({ page }) => {
  await fixture(page, { emptyFolder: true });
  await openConnection(page); await login(page);
  const empty = page.getByRole("heading", { name: "This folder is empty", exact: true });
  await expect(empty).toBeVisible();
  const reads = (await calls(page, "secrets_list")).length;
  await page.getByRole("button", { name: "Refresh folder", exact: true }).last().click();
  await expect.poll(async () => (await calls(page, "secrets_list")).length).toBe(reads + 1);
  await expect(empty).toBeVisible();
});
