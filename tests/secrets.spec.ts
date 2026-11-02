import { expect, test, type Page } from "@playwright/test";
import { defaultPreferences } from "../src/app/preferences/api";

const A = "a".repeat(32);
const B = "b".repeat(32);
const VALUE = "SYNTHETIC_SECRET_VALUE";
type Options = { two?: boolean; emptyProfiles?: boolean; emptyFolder?: boolean; unavailable?: boolean; delayReveal?: boolean; delayRead?: boolean; delayLogin?: boolean; delayOpen?: boolean; profileError?: boolean; logoutError?: boolean; partial?: boolean; failLogin?: boolean; failMounts?: boolean; failUpdate?: boolean; failMetadata?: boolean; failPin?: boolean; failRemove?: boolean };
type Call = { command: string; args: any };

async function fixture(page: Page, options: Options = {}) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(({ options, preferences }) => {
    // Only synthetic, non-sensitive profile metadata survives reload in this mock Rust store.
    const storageKey = "test.secrets.synthetic-profiles";
    const first = { id: "a".repeat(32), provider: "vault", name: "Fixture Vault", address: "https://vault.synthetic.invalid", namespace: "", authMount: "userpass", kvMount: "development", basePath: "", pinned: false };
    let profiles = JSON.parse(localStorage.getItem(storageKey) || "null") ?? (options.emptyProfiles ? [] : [first, ...(options.two ? [{ ...first, id: "b".repeat(32), name: "Secondary Vault", address: "https://second.synthetic.invalid" }] : [])]);
    const persist = () => localStorage.setItem(storageKey, JSON.stringify(profiles));
    const calls: Call[] = [];
    const sessions = new Map<string, any>();
    const pending: Record<string, (() => void)[]> = {};
    let serial = 0;
    let callbackId = 0;
    const handlers = new Map<number, unknown>();
    let profileError = !!options.profileError;
    const wait = (kind: string) => new Promise<void>(resolve => (pending[kind] ??= []).push(resolve));
    Object.defineProperty(window, "isTauri", { value: true, writable: true, configurable: true });
    Object.assign(window, {
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} }, __ICEON_WINDOW_LAYOUT__: { platform: "macos", seamless: false },
      __secretCalls: calls,
      __secretSessions: sessions,
      __finishSecret: (kind: string) => pending[kind]?.splice(0).forEach(resolve => resolve()),
      __secretProfileError: (value: boolean) => { profileError = value; },
      __secretOptions: options,
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
        transformCallback: (callback: unknown) => { handlers.set(++callbackId, callback); return callbackId; }, unregisterCallback: (id: number) => { handlers.delete(id); },
        invoke: async (command: string, args: any = {}) => {
          calls.push({ command, args: structuredClone(args) });
          const request = args.request;
          if (command === "plugin:event|listen") return 1;
          if (command === "native_navigation_ready") return;
          if (command === "preferences_get") return { revision: 1, preferences };
          if (command === "update_state") return { phase: "idle", version: null, error: null };
          if (command === "catalog_list") return { revision: 1, clusters: [], pinnedItems: [], discoveryError: null };
          if (command === "catalog_connection_states") return [];
          if (command === "secrets_profiles") { if (profileError) throw "storeUnavailable"; return structuredClone(profiles); }
          if (command === "secrets_profile_save") {
            const saved = { ...request, id: (++serial + 12).toString(16).padStart(32, "0"), pinned: false };
            profiles.push(saved); persist(); return structuredClone(saved);
          }
          if (command === "secrets_profile_update") {
            if (options.failUpdate) throw "storeUnavailable";
            const previous = profiles.find((profile: any) => profile.id === request.id);
            if (!previous) throw "notFound";
            const saved = { ...previous, ...request, pinned: previous.pinned };
            profiles = profiles.map((profile: any) => profile.id === saved.id ? saved : profile);
            for (const [key, session] of sessions) if (session.profileId === saved.id) sessions.delete(key);
            persist(); return structuredClone(saved);
          }
          if (command === "secrets_profile_pin") {
            if (options.failPin) throw "storeUnavailable";
            const saved = profiles.find((profile: any) => profile.id === args.profileId);
            if (!saved) throw "notFound";
            saved.pinned = args.pinned; persist(); return structuredClone(saved);
          }
          if (command === "secrets_profile_remove") { if (options.failRemove) throw "storeUnavailable"; profiles = profiles.filter((profile: any) => profile.id !== args.profileId); persist(); return null; }
          if (command === "secrets_session_open") {
            const session = { profileId: args.profileId, sessionId: `${args.profileId}-${args.ownerId}-${++serial}`, status: "signedOut", expiresAt: null, credentialStore: options.unavailable ? "unavailable" : "macosKeychain" };
            sessions.set(session.sessionId, session);
            if (options.delayOpen) await wait("open");
            return structuredClone(session);
          }
          if (command === "secrets_close") { sessions.delete(request.sessionId); return null; }
          if (["secrets_login", "secrets_resume", "secrets_mounts", "secrets_list", "secrets_metadata", "secrets_reveal", "secrets_disconnect", "secrets_cancel"].includes(command)) {
            const current = sessions.get(request.sessionId);
            if (!current || current.profileId !== request.profileId) throw "staleSession";
            if (["secrets_mounts", "secrets_list", "secrets_metadata", "secrets_reveal"].includes(command) && current.status !== "connected") throw "sessionExpired";
          }
          if (command === "secrets_login" || command === "secrets_resume") {
            const session = sessions.get(request.sessionId);
            if (!session || session.profileId !== request.profileId) throw "staleSession";
            if (options.delayLogin) await wait("login");
            if (options.failLogin) throw "authFailed";
            const connected = { ...session, status: "connected", expiresAt: Date.now() + 60_000 };
            if (sessions.has(request.sessionId)) sessions.set(request.sessionId, connected);
            return connected;
          }
          if (command === "secrets_disconnect") { const current = sessions.get(request.sessionId); sessions.set(request.sessionId, { ...current, status: "signedOut", expiresAt: null }); if (options.logoutError) throw "network"; return null; }
          if (["secrets_mounts", "secrets_list", "secrets_metadata"].includes(command) && options.delayRead) await wait("read");
          if (command === "secrets_mounts") { if (options.failMounts) throw "permissionDenied"; return { profileId: request.profileId, sessionId: request.sessionId, readId: request.readId, mounts: [{ path: "development", type: "kv", version: 2, supported: true }, { path: "production", type: "kv", version: 2, supported: true }, { path: "pki", type: "pki", version: null, supported: false }], partial: !!options.partial }; }
          if (command === "secrets_list") return { ...request, mount: request.mount ?? "development", entries: options.emptyFolder ? [] : request.path ? [{ name: "nested", directory: false }] : [{ name: "folder", directory: true }, { name: "sample", directory: false }], partial: !!options.partial };
          if (command === "secrets_metadata") { if (options.failMetadata) throw "permissionDenied"; return { ...request, currentVersion: 11, createdAt: "2026-01-01T00:00:00Z", updatedAt: "2026-02-01T00:00:00Z", versions: [{ version: 11, createdAt: "2026-02-01T00:00:00Z", deletionTime: "", destroyed: false }, { version: 10, createdAt: "2026-01-01T00:00:00Z", deletionTime: "", destroyed: true }], partial: !!options.partial }; }
          if (command === "secrets_reveal") {
            if (options.delayReveal) await wait("reveal");
            return { ...request, fields: [{ name: "example", value: "SYNTHETIC_SECRET_VALUE" }], version: 11, expiresAt: Date.now() + 30_000 };
          }
          return null;
        },
      },
    });
  }, { options, preferences: defaultPreferences });
}
async function calls(page: Page, command: string): Promise<Call[]> {
  return page.evaluate(command => (window as any).__secretCalls.filter((call: Call) => call.command === command), command);
}
const button = (page: Page, name: string) => page.getByRole("button", { name, exact: true });
const username = (page: Page) => page.getByRole("textbox", { name: /^Username(?: Required)?$/ });
const password = (page: Page) => page.getByRole("textbox", { name: /^Password(?: Required)?$/ });
async function openConnection(page: Page) {
  await page.goto("/#/secrets");
  await button(page, "Open Fixture Vault").click();
  await expect(page).toHaveURL(new RegExp(`#/secrets\\?connection=${A}$`));
  await expect(username(page)).toBeEnabled();
}
async function login(page: Page) {
  await username(page).fill("fixture-user");
  await password(page).fill("synthetic-password");
  await button(page, "Log in").click();
}
async function inspect(page: Page, folder = false) {
  await button(page, "Open development").click();
  if (folder) await button(page, "Open folder").click();
  await button(page, folder ? "Open nested" : "Open sample").click();
  await expect(page.getByText("Current version: 11", { exact: true })).toBeVisible();
}
async function finish(page: Page, kind: string) { await page.evaluate(kind => (window as any).__finishSecret(kind), kind); }

test("browser fallback has no secrets IPC and disables Add", async ({ page }) => {
  await page.addInitScript(() => { (window as any).__secretCalls = []; (window as any).__TAURI_INTERNALS__ = { invoke: (command: string) => { (window as any).__secretCalls.push({ command }); return null; } }; });
  await page.goto("/");
  await page.getByRole("link", { name: "Secrets", exact: true }).click();
  await expect(page.getByText("Browser preview cannot connect", { exact: false })).toBeVisible();
  await expect(button(page, "Add secret connection")).toBeDisabled();
  expect(await calls(page, "secrets_profiles")).toEqual([]);
});

test("opening selects a route and desktop sign-in aside; missing fields fail inline without authentication", async ({ page }) => {
  await fixture(page); await openConnection(page);
  const aside = page.getByRole("complementary", { name: "Fixture Vault connection", exact: true });
  await expect(aside).toBeVisible();
  const asideBox = await aside.boundingBox();
  expect(asideBox!.x).toBeGreaterThan(500);
  await button(page, "Log in").click();
  await expect(username(page)).toHaveAttribute("aria-invalid", "true");
  await expect(username(page)).toBeFocused();
  await expect(page.getByText("Enter your username.", { exact: true })).toBeVisible();
  await username(page).fill("fixture-user"); await button(page, "Log in").click();
  await expect(password(page)).toHaveAttribute("aria-invalid", "true");
  await expect(password(page)).toBeFocused();
  expect(await calls(page, "secrets_login")).toEqual([]);
  expect(await calls(page, "secrets_mounts")).toEqual([]);
});

test("profile validation focuses direct errors; saving metadata never authenticates or discovers mounts", async ({ page }) => {
  await fixture(page); await page.goto("/#/secrets");
  await button(page, "Add secret connection").click();
  const dialog = page.getByRole("dialog", { name: "Add secret connection", exact: true });
  const name = dialog.getByRole("textbox", { name: "Connection name Required", exact: true });
  const endpoint = dialog.getByRole("textbox", { name: "HTTPS endpoint Required", exact: true });
  const mount = dialog.getByRole("textbox", { name: "Default KV v2 mount Required", exact: true });
  await endpoint.fill("http://vault.synthetic.invalid"); await mount.fill("/bad/");
  await dialog.getByRole("button", { name: "Save connection", exact: true }).click();
  await expect(name).toBeFocused();
  for (const field of [name, endpoint, mount]) await expect(field).toHaveAttribute("aria-invalid", "true");
  await expect(dialog.getByText("Enter a connection name of at most 128 bytes.", { exact: true })).toBeVisible();
  await expect(dialog.getByText("Enter an HTTPS origin without credentials, a path, query, or fragment.", { exact: true })).toBeVisible();
  expect(await calls(page, "secrets_profile_save")).toEqual([]);
  await expect(dialog.getByText("Authentication method path, not your username.", { exact: true })).toBeVisible();
  await expect(dialog.getByText("Engine path, not your password.", { exact: false })).toBeVisible();
  await name.fill("New fixture"); await endpoint.fill("https://new.synthetic.invalid"); await mount.fill("production");
  await dialog.getByRole("button", { name: "Save connection", exact: true }).click();
  await expect(dialog).toHaveCount(0); await expect(username(page)).toBeEnabled();
  expect(await calls(page, "secrets_login")).toEqual([]); expect(await calls(page, "secrets_mounts")).toEqual([]);
  expect((await calls(page, "secrets_profile_save"))[0].args.request).toMatchObject({ provider: "vault", kvMount: "production" });
  expect((await calls(page, "secrets_profile_save"))[0].args.request).not.toHaveProperty("password");
});

test("pin persists across reload, selects the exact connection, and editing preserves identity and pin", async ({ page }) => {
  await fixture(page, { two: true }); await openConnection(page); await login(page);
  const original = (await calls(page, "secrets_login"))[0].args.request.sessionId;
  await button(page, "Connections").click(); await button(page, "Pin Fixture Vault").click();
  const pin = page.getByRole("link", { name: "Fixture Vault · Secrets", exact: true });
  await expect(pin).toBeVisible();
  await button(page, "Edit Fixture Vault").click();
  const dialog = page.getByRole("dialog", { name: "Edit secret connection", exact: true });
  await expect(dialog.getByRole("note")).toContainText("forgets its saved login");
  await dialog.getByRole("textbox", { name: "Default KV v2 mount Required", exact: true }).fill("production");
  await dialog.getByRole("button", { name: "Save connection", exact: true }).click();
  await expect(username(page)).toBeEnabled();
  expect((await calls(page, "secrets_profile_update"))[0].args.request).toMatchObject({ id: A, kvMount: "production" });
  await expect.poll(async () => (await calls(page, "secrets_close")).some(call => call.args.request.sessionId === original)).toBe(true);
  await page.reload(); await expect(pin).toBeVisible();
  await button(page, "Connections").click(); await expect(button(page, "Unpin Fixture Vault")).toBeVisible();
  await button(page, "Open Secondary Vault").click(); await pin.click();
  await expect(page).toHaveURL(new RegExp(`connection=${A}$`));
  await button(page, "Connections").click(); await button(page, "Remove Fixture Vault").click();
  await page.getByRole("dialog").getByRole("button", { name: "Remove connection", exact: true }).click();
  await expect(pin).toHaveCount(0); await expect(button(page, "Open Fixture Vault")).toHaveCount(0);
});

test("two connection sessions survive switching while inactive passwords and reveals clear", async ({ page }) => {
  await fixture(page, { two: true }); await openConnection(page);
  await password(page).fill("synthetic-draft");
  await button(page, "Connections").click(); await button(page, "Open Secondary Vault").click();
  await expect(username(page)).toBeEnabled();
  await button(page, "Fixture Vault").click(); await expect(password(page)).toHaveValue("");
  await login(page); await inspect(page); await button(page, "Reveal values").click();
  await expect(page.getByText(VALUE, { exact: true })).toBeVisible();
  const closes = (await calls(page, "secrets_close")).length;
  await button(page, "Secondary Vault").click(); await expect(page.getByText(VALUE, { exact: true })).toHaveCount(0);
  await login(page); await expect(button(page, "Open development")).toBeEnabled();
  await button(page, "Fixture Vault").click(); await expect(button(page, "Reveal values")).toBeVisible();
  expect(await calls(page, "secrets_close")).toHaveLength(closes);
  const active = await page.evaluate(() => [...(window as any).__secretSessions.values()]);
  expect(active.map((session: any) => session.profileId).sort()).toEqual([A, B]);
  expect(new Set(active.map((session: any) => session.sessionId)).size).toBe(2);
  expect(active.every((session: any) => session.status === "connected")).toBe(true);
  await button(page, "Secondary Vault").click();
  await button(page, "Close connection").click();
  await expect(button(page, "Reveal values")).toBeVisible();
  const remaining = await page.evaluate(() => [...(window as any).__secretSessions.values()]);
  expect(remaining).toHaveLength(1); expect(remaining[0].profileId).toBe(A);
});

test("engines, folders and metadata are scoped; inspector never reveals until explicitly requested", async ({ page }) => {
  await fixture(page, { partial: true }); await openConnection(page); await login(page);
  await expect(button(page, "Open pki")).toBeDisabled(); await expect(button(page, "Open production")).toBeEnabled();
  await expect(page.getByText("The engine listing is partial.", { exact: false })).toBeVisible();
  await inspect(page, true);
  const request = (await calls(page, "secrets_metadata")).at(-1)!.args.request;
  expect(request).toMatchObject({ profileId: A, mount: "development", path: "folder/nested" });
  expect(await calls(page, "secrets_reveal")).toEqual([]);
  const details = page.getByRole("complementary", { name: "Secret details", exact: true });
  await expect(details.getByText("Secret metadata is partial.", { exact: false })).toBeVisible();
  await button(page, "Version history").click(); await expect(details.getByRole("cell", { name: "Destroyed", exact: true })).toBeVisible();
  await button(page, "Paths").click(); await expect(details.getByText("API data path: /v1/development/data/folder/nested", { exact: true })).toBeVisible();
  await expect(details.getByText("API metadata path: /v1/development/metadata/folder/nested", { exact: true })).toBeVisible();
  expect(await calls(page, "secrets_metadata")).toHaveLength(1);
  expect(await calls(page, "secrets_reveal")).toHaveLength(0);
  await button(page, "Overview").click(); await button(page, "Reveal values").click(); await expect(page.getByText(VALUE, { exact: true })).toBeVisible();
  expect((await calls(page, "secrets_reveal")).at(-1)!.args.request).toMatchObject({ ...request, readId: expect.any(Number) });
  await page.keyboard.press("Escape"); await expect(details).toHaveCount(0); await expect(button(page, "Open nested")).toBeFocused();
});

test("reveals hide manually, after 30 seconds and on window blur", async ({ page }) => {
  await fixture(page); await openConnection(page); await page.clock.install(); await login(page); await inspect(page);
  for (const mode of ["manual", "timer", "blur"]) {
    await button(page, "Reveal values").click(); await expect(page.getByText(VALUE, { exact: true })).toBeVisible();
    if (mode === "manual") await button(page, "Hide values").click();
    else if (mode === "timer") await page.clock.fastForward(30_001);
    else await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect(page.getByText(VALUE, { exact: true })).toHaveCount(0);
  }
});

for (const navigation of ["blur", "connections", "unmount"] as const) test(`late reveal is cancelled on ${navigation} and cannot revive values`, async ({ page }) => {
  await fixture(page, { delayReveal: true }); await openConnection(page); await login(page); await inspect(page);
  await button(page, "Reveal values").click(); await expect.poll(async () => (await calls(page, "secrets_reveal")).length).toBe(1);
  if (navigation === "blur") await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  else if (navigation === "connections") await button(page, "Connections").click();
  else await page.getByRole("link", { name: "Clusters", exact: true }).click();
  await expect.poll(async () => (await calls(page, "secrets_cancel")).length).toBe(1);
  const reveal = (await calls(page, "secrets_reveal"))[0].args.request;
  expect((await calls(page, "secrets_cancel"))[0].args.request).toEqual({ profileId: reveal.profileId, sessionId: reveal.sessionId, readId: reveal.readId });
  await finish(page, "reveal");
  if (navigation === "connections") await button(page, "Fixture Vault").click();
  if (navigation === "unmount") await page.getByRole("link", { name: "Secrets", exact: true }).click();
  await expect(page.getByText(VALUE, { exact: true })).toHaveCount(0);
});

for (const method of ["login", "resume"] as const) test(`close releases the owned session immediately during pending ${method}`, async ({ page }) => {
  await fixture(page, { delayLogin: true }); await openConnection(page);
  if (method === "login") { await page.getByRole("checkbox", { name: "Remember login", exact: true }).check(); await login(page); }
  else await button(page, "Resume saved login").click();
  const command = method === "login" ? "secrets_login" : "secrets_resume";
  await expect.poll(async () => (await calls(page, command)).length).toBe(1);
  const sessionId = (await calls(page, command))[0].args.request.sessionId;
  await expect(password(page)).toHaveValue(""); await button(page, "Close connection").click();
  await expect.poll(async () => (await calls(page, "secrets_close")).some(call => call.args.request.sessionId === sessionId)).toBe(true);
  await finish(page, "login"); await expect(button(page, "Open Fixture Vault")).toBeVisible();
  expect(await calls(page, "secrets_mounts")).toEqual([]);
  await button(page, "Open Fixture Vault").click(); await expect(password(page)).toHaveValue("");
  await expect(page.getByRole("checkbox", { name: "Remember login", exact: true })).not.toBeChecked();
});

test("late strict-mode opens close only their own sessions after unmount", async ({ page }) => {
  await fixture(page, { delayOpen: true }); await page.goto(`/#/secrets?connection=${A}`);
  await expect.poll(async () => (await calls(page, "secrets_session_open")).length).toBeGreaterThan(0);
  await page.getByRole("link", { name: "Clusters", exact: true }).click(); await finish(page, "open");
  await expect.poll(async () => page.evaluate(() => (window as any).__secretSessions.size)).toBe(0);
  await page.getByRole("link", { name: "Secrets", exact: true }).click(); await button(page, "Open Fixture Vault").click();
  await finish(page, "open"); await expect(username(page)).toBeEnabled();
  await expect.poll(async () => page.evaluate(() => (window as any).__secretSessions.size)).toBe(1);
});

test("unavailable credential store disables remember and resume, not explicit login", async ({ page }) => {
  await fixture(page, { unavailable: true }); await openConnection(page);
  await expect(page.getByRole("checkbox", { name: "Remember login", exact: true })).toBeDisabled();
  await expect(button(page, "Resume saved login")).toBeDisabled(); await login(page); await expect(button(page, "Open development")).toBeVisible();
});

test("failed logout still clears displayed values and password", async ({ page }) => {
  await fixture(page, { logoutError: true }); await openConnection(page); await login(page); await inspect(page);
  await button(page, "Reveal values").click(); await expect(page.getByText(VALUE, { exact: true })).toBeVisible();
  await button(page, "Disconnect").click(); await expect(page.getByText(VALUE, { exact: true })).toHaveCount(0);
  await expect(page.getByRole("alert")).toContainText("The provider could not be reached."); await expect(password(page)).toHaveValue("");
  expect((await calls(page, "secrets_disconnect"))[0].args.request.forget).toBe(false);
});

test("global Refresh follows Go forward and refreshes the active folder without inline refresh controls", async ({ page }) => {
  await fixture(page, { emptyFolder: true }); await openConnection(page); await login(page); await button(page, "Open development").click();
  await expect(page.getByRole("heading", { name: "This folder is empty", exact: true })).toBeVisible();
  const toolbar = page.getByRole("navigation", { name: "App toolbar", exact: true });
  const refresh = toolbar.getByRole("button", { name: "Refresh secrets", exact: true }); await expect(refresh).toBeEnabled();
  const controls = await toolbar.getByRole("button").evaluateAll(elements => elements.map(element => element.getAttribute("aria-label") || element.textContent?.trim()));
  expect(controls.indexOf("Refresh secrets")).toBe(controls.indexOf("Go forward") + 1);
  await expect(page.getByRole("main").getByRole("button", { name: /Refresh (folder|connections|secrets)/ })).toHaveCount(0);
  const reads = (await calls(page, "secrets_list")).length; await refresh.click();
  await expect.poll(async () => (await calls(page, "secrets_list")).length).toBe(reads + 1);
});

test("failed stored-profile reads never pretend to be an empty store; refresh recovers", async ({ page }) => {
  await fixture(page, { profileError: true, emptyProfiles: true }); await page.goto("/#/secrets");
  await expect(page.getByRole("heading", { name: "Connections could not be loaded", exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "No saved connections", exact: true })).toHaveCount(0);
  await page.evaluate(() => (window as any).__secretProfileError(false)); await button(page, "Refresh connections").click();
  await expect(page.getByRole("heading", { name: "No saved connections", exact: true })).toBeVisible();
});

for (const theme of ["light", "dark"] as const) test(`shared Add button and inspector fit 390-wide ${theme} layout at 125% zoom`, async ({ page }) => {
  await page.emulateMedia({ colorScheme: theme }); await fixture(page); await page.goto("/#/secrets");
  const add = button(page, "Add secret connection"); const desktop = await add.boundingBox();
  expect(desktop!.width).toBeGreaterThan(20);
  await page.setViewportSize({ width: 390, height: 640 }); await page.evaluate(() => { document.documentElement.style.zoom = "1.25"; });
  await expect(add).toBeInViewport(); const narrow = await add.boundingBox();
  expect(narrow!.width / desktop!.width).toBeCloseTo(1.25, 1);
  await add.click(); const dialog = page.getByRole("dialog"); await expect(dialog).toBeInViewport();
  await page.keyboard.press("Escape"); await expect(dialog).toHaveCount(0); await expect(add).toBeFocused();
  await button(page, "Open Fixture Vault").click(); await expect(username(page)).toBeEnabled(); await login(page); await inspect(page);
  await expect(page.getByRole("complementary", { name: "Secret details", exact: true })).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.keyboard.press("Escape"); await expect(button(page, "Open sample")).toBeFocused();
});

async function control(page: Page, changes: Partial<Options>) {
  await page.evaluate(changes => Object.assign((window as any).__secretOptions, changes), changes);
}

test("pin and unpin update sidebar without opening a session", async ({ page }) => {
  await fixture(page); await page.goto("/#/secrets");
  const pin = page.getByRole("link", { name: "Fixture Vault · Secrets", exact: true });
  await button(page, "Pin Fixture Vault").click(); await expect(pin).toBeVisible();
  await expect(page).toHaveURL(/#\/secrets$/);
  expect(await calls(page, "secrets_session_open")).toEqual([]);
  await button(page, "Unpin Fixture Vault").click(); await expect(pin).toHaveCount(0);
  expect(await calls(page, "secrets_session_open")).toEqual([]);
  const row = page.getByRole("row", { name: "Open connection Fixture Vault", exact: true });
  await row.focus(); await page.keyboard.press("Enter");
  await expect(username(page)).toBeEnabled();
});

test("engine loading and failure remain distinct from empty data and refresh recovers", async ({ page }) => {
  await fixture(page, { delayRead: true, failMounts: true }); await openConnection(page); await login(page);
  await expect(page.getByRole("status").filter({ hasText: "Loading engines" })).toBeVisible();
  await expect(page.getByText("No visible engines", { exact: true })).toHaveCount(0);
  await finish(page, "read");
  await expect(page.getByText("Engines could not be loaded", { exact: true })).toBeVisible();
  await expect(page.getByText("No visible engines", { exact: true })).toHaveCount(0);
  await control(page, { delayRead: false, failMounts: false });
  await button(page, "Refresh engines").click(); await expect(button(page, "Open development")).toBeEnabled();
});

test("failed login and metadata stay in their own workspace and never reveal values", async ({ page }) => {
  await fixture(page, { failLogin: true }); await openConnection(page); await login(page);
  await expect(page.getByRole("alert")).toContainText("Authentication failed");
  await expect(password(page)).toHaveValue(""); expect(await calls(page, "secrets_mounts")).toEqual([]);
  await control(page, { failLogin: false, failMetadata: true }); await login(page);
  await button(page, "Open development").click(); await button(page, "Open sample").click();
  await expect(page.getByRole("complementary", { name: "Secret details", exact: true }).getByRole("alert")).toContainText("denied");
  expect(await calls(page, "secrets_reveal")).toEqual([]);
  await control(page, { failMetadata: false }); await button(page, "Refresh secret details").click();
  await expect(button(page, "Reveal values")).toBeEnabled();
});

test("update, pin and remove failures preserve saved identity and allow retry", async ({ page }) => {
  await fixture(page, { failUpdate: true, failPin: true, failRemove: true }); await page.goto("/#/secrets");
  await button(page, "Pin Fixture Vault").click();
  await expect(page.getByRole("alert").first()).toContainText("connection store is unavailable");
  await expect(page.getByRole("link", { name: "Fixture Vault · Secrets", exact: true })).toHaveCount(0);
  await button(page, "Edit Fixture Vault").click();
  const form = page.getByRole("dialog", { name: "Edit secret connection", exact: true });
  await form.getByRole("textbox", { name: "Username/password auth mount Required", exact: true }).fill("correct-auth");
  await form.getByRole("button", { name: "Save connection", exact: true }).click();
  await expect(form.getByRole("alert")).toContainText("connection store is unavailable");
  await form.getByRole("button", { name: "Cancel connection", exact: true }).click();
  await button(page, "Remove Fixture Vault").click();
  const remove = page.getByRole("dialog", { name: "Remove secret connection", exact: true });
  await remove.getByRole("button", { name: "Remove connection", exact: true }).click();
  await expect(remove.getByRole("alert")).toContainText("connection store is unavailable");
  await remove.getByRole("button", { name: "Keep connection", exact: true }).click();
  await expect(button(page, "Open Fixture Vault")).toBeEnabled();
  expect(await calls(page, "secrets_session_open")).toEqual([]);
  await control(page, { failPin: false }); await button(page, "Pin Fixture Vault").click();
  await expect(page.getByRole("link", { name: "Fixture Vault · Secrets", exact: true })).toBeVisible();
});

test("resume, expiry and forget clear values without persisting credentials in profile metadata", async ({ page }) => {
  await fixture(page); await openConnection(page); await page.clock.install();
  await button(page, "Resume saved login").click(); await expect(button(page, "Open development")).toBeEnabled();
  expect(await calls(page, "secrets_login")).toEqual([]);
  await inspect(page); await button(page, "Reveal values").click();
  await expect(page.getByText(VALUE, { exact: true })).toBeVisible();
  await page.clock.fastForward(60_001);
  await expect(username(page)).toBeEnabled(); await expect(password(page)).toHaveValue("");
  await expect(page.getByText(VALUE, { exact: true })).toHaveCount(0);
  await button(page, "Forget saved login").click();
  await expect(page.getByRole("status").filter({ hasText: "Saved login forgotten." })).toBeVisible();
  expect((await calls(page, "secrets_disconnect")).at(-1)!.args.request.forget).toBe(true);
  const stored = await page.evaluate(() => localStorage.getItem("test.secrets.synthetic-profiles"));
  expect(stored ?? "").not.toContain(VALUE); expect(stored ?? "").not.toContain("password");
});

test("global refresh targets engines and selected metadata; form gates keyboard refresh", async ({ page }) => {
  await fixture(page); await openConnection(page); await login(page);
  const mounts = (await calls(page, "secrets_mounts")).length;
  await button(page, "Refresh engines").click();
  await expect.poll(async () => (await calls(page, "secrets_mounts")).length).toBe(mounts + 1);
  await inspect(page);
  const reads = (await calls(page, "secrets_metadata")).length;
  await button(page, "Refresh secret details").click();
  await expect.poll(async () => (await calls(page, "secrets_metadata")).length).toBe(reads + 1);
  expect(await calls(page, "secrets_reveal")).toEqual([]);
  await button(page, "Connections").click(); await button(page, "Edit Fixture Vault").click();
  const before = (await calls(page, "secrets_profiles")).length;
  await page.keyboard.press("Control+r");
  await expect(page.getByRole("dialog", { name: "Edit secret connection", exact: true })).toBeVisible();
  expect(await calls(page, "secrets_profiles")).toHaveLength(before);
});

for (const theme of ["light", "dark"] as const) test(`inspector and connection sidebar stay within desktop and narrow ${theme} viewport`, async ({ page }) => {
  await page.emulateMedia({ colorScheme: theme }); await fixture(page); await openConnection(page);
  const aside = page.getByRole("complementary", { name: "Fixture Vault connection", exact: true });
  const desktop = await aside.boundingBox(); expect(desktop!.x).toBeGreaterThan(500);
  await login(page); await inspect(page);
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 800 });
    const inspector = page.getByRole("complementary", { name: "Secret details", exact: true });
    await expect(inspector).toBeInViewport();
    await expect.poll(async () => {
      const bounds = await inspector.boundingBox();
      return !!bounds && bounds.x >= 0 && bounds.x + bounds.width <= width + 1;
    }).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  }
  await page.keyboard.press("Escape"); await expect(button(page, "Open sample")).toBeFocused();
  await button(page, "Open sample").click();
  await button(page, "Close secret details").click(); await expect(button(page, "Open sample")).toBeFocused();
});

test("saving a new profile stores only metadata and opens a signed-out workspace", async ({ page }) => {
  await fixture(page); await page.goto("/#/secrets"); await button(page, "Add secret connection").click();
  const dialog = page.getByRole("dialog", { name: "Add secret connection", exact: true });
  await dialog.getByRole("textbox", { name: "Connection name Required", exact: true }).fill("Synthetic added vault");
  await dialog.getByRole("textbox", { name: "HTTPS endpoint Required", exact: true }).fill("https://added.synthetic.invalid");
  await dialog.getByRole("textbox", { name: "Default KV v2 mount Required", exact: true }).fill("");
  await dialog.getByRole("button", { name: "Save connection", exact: true }).click();
  await expect(dialog.getByRole("textbox", { name: "Default KV v2 mount Required", exact: true })).toHaveAttribute("aria-invalid", "true");
  expect(await calls(page, "secrets_profile_save")).toEqual([]);
  await dialog.getByRole("textbox", { name: "Default KV v2 mount Required", exact: true }).fill("development");
  await dialog.getByRole("button", { name: "Save connection", exact: true }).click();
  await expect(dialog).toHaveCount(0); await expect(username(page)).toBeEnabled();
  const saved = (await calls(page, "secrets_profile_save"))[0].args.request;
  expect(saved).toEqual({ provider: "vault", name: "Synthetic added vault", address: "https://added.synthetic.invalid", authMount: "userpass", kvMount: "development", namespace: "", basePath: "" });
  expect(await calls(page, "secrets_login")).toEqual([]); expect(await calls(page, "secrets_mounts")).toEqual([]);
  await expect(page).toHaveURL(/connection=[a-f0-9]{32}$/);
});
