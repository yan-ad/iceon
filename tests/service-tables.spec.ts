import { installNamespaceSnapshotMock } from "./helpers/namespaceSnapshotMock";
import { expect, test, type Page } from "@playwright/test";

type Request = Record<string, string | number | null>;
type Call = { command: string; request: Request };
type Controls = { failure?: string; malformed?: string; empty?: boolean; hold?: boolean; holdCancel?: boolean; detailFailure?: string; stopFailure?: string; forwardStatus?: string; holdDetail?: boolean; holdStart?: boolean; startFailure?: string };
type MockWindow = Window & { __serviceCalls: Call[]; __serviceControls: Controls; __releaseServices: () => void; __releaseHelmCancels: () => void; __externalOpens: string[] };
const grid = (page: Page, name: string) => page.getByRole("table", { name, exact: true });

for (const width of [1440, 390]) for (const colorScheme of ["light", "dark"] as const) {
  test(`port forwards use the Astryx empty state at ${width} in ${colorScheme}`, async ({ page }, testInfo) => {
    await page.emulateMedia({ colorScheme });
    await fixture(page, "port-forwarding");
    await page.setViewportSize({ width, height: 900 });
    const empty = grid(page, "Port forwards").locator(".astryx-empty-state");
    await expect(empty).toHaveAttribute("role", "status");
    await expect(empty.getByRole("heading", { name: "No port forwards", level: 2, exact: true })).toBeVisible();
    await expect(empty).toContainText("Create a port forward to connect a local port to a Pod in this session.");
    await expect(page.getByText("No active port forwards were returned.", { exact: true })).toHaveCount(0);
    const viewport = page.locator("[data-iceon-table-viewport]");
    for (const scroll of [false, true]) {
      if (scroll) await viewport.evaluate(element => { element.scrollLeft = element.scrollWidth; });
      const box = (await empty.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    await page.screenshot({ path: testInfo.outputPath("port-forwards-empty.png") });
    await empty.getByRole("button", { name: "Create forward", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("dialog", { name: "Create port forward", exact: true })).toBeVisible();
    expect(await calls(page, "cluster_pod_forward_start")).toEqual([]);
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(empty).toBeVisible();
  });
}

test("port forward loading and failed empty refreshes do not show a successful empty state", async ({ page }) => {
  await fixture(page, "port-forwarding", { hold: true });
  const empty = page.locator(".astryx-empty-state");
  await expect(page.getByRole("status", { name: "Loading port forwards…", exact: true })).toBeVisible();
  await expect(empty).toHaveCount(0);
  await controls(page, { hold: false });
  await page.evaluate(() => (window as MockWindow).__releaseServices());
  await expect(empty).toBeVisible();
  await controls(page, { failure: "network" });
  await refreshForwards(page);
  await expect(page.getByRole("alert")).toContainText("Port forwards unavailable");
  await expect(empty).toHaveCount(0);
  await expect(grid(page, "Port forwards")).toHaveCount(0);
  await controls(page, { failure: "" });
  await page.getByRole("alert").getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(empty.getByRole("heading", { name: "No port forwards", exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("delayed Helm cleanup targets only the old read while the new read is pending", async ({ page }) => {
  await fixture(page);
  await expect(grid(page, "Helm Releases")).toContainText("edge");
  const previous = (await calls(page, "helm_table")).at(-1)!.request;
  const previousCancels = await calls(page, "helm_cancel");
  await controls(page, { hold: true, holdCancel: true });
  await refreshHelm(page);
  await expect(page.getByRole("progressbar", { name: "Loading Helm releases…", exact: true })).toBeVisible();
  const pending = (await calls(page, "helm_table")).at(-1)!.request;
  expect(pending).toEqual({ ...previous, readId: expect.any(Number) });
  expect(pending.readId).toBeGreaterThan(previous.readId as number);
  await expect.poll(async () => (await calls(page, "helm_cancel")).slice(previousCancels.length)).toEqual([
    { command: "helm_cancel", request: helmCancelScope(previous) },
  ]);
  await page.evaluate(() => (window as MockWindow).__releaseHelmCancels());
  expect((await calls(page, "helm_cancel")).some((call) => call.request.readId === pending.readId)).toBe(false);
  await expect(page.getByRole("progressbar", { name: "Loading Helm releases…", exact: true })).toBeVisible();
  await page.evaluate(() => (window as MockWindow).__releaseServices());
  await expect(page.getByRole("progressbar", { name: "Loading Helm releases…", exact: true })).toHaveCount(0);
  await expect(grid(page, "Helm Releases")).toContainText("edge");
  expect((await page.evaluate(() => (window as MockWindow).__serviceCalls)).every(
    (call) => ["catalog_list", "cluster_session_open", "cluster_connect", "cluster_disconnect", "cluster_permissions_review", "cluster_permissions_cancel", "cluster_permissions_refresh", "cluster_resources", "cluster_resources_cancel", "helm_table", "helm_cancel", "cluster_pod_forward_list", "cluster_pod_detail", "cluster_pod_forward_start", "cluster_pod_forward_stop"].includes(call.command),
  )).toBe(true);
});

for (const forwardStatus of ["stopped", "error"]) test(`${forwardStatus} forwards use Remove and remove the actual session record`, async ({ page }) => {
  await fixture(page, "port-forwarding"); await startForward(page);
  const table = grid(page, "Port forwards");
  await expect(table).toContainText("api-pod");
  await controls(page, { forwardStatus });
  await refreshForwards(page);
  await openForwardActions(page);
  await expect(page.getByRole("menuitem", { name: "Stop", exact: true })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Remove", exact: true })).toBeEnabled();
  await page.getByRole("menuitem", { name: "Remove", exact: true }).click();
  await expect(table).toContainText("No port forwards");
  expect(await calls(page, "cluster_pod_forward_stop")).toEqual([{ command: "cluster_pod_forward_stop", request: {
    clusterId: "one", sessionId: (await calls(page, "cluster_pod_forward_start"))[0].request.sessionId, id: "forward-one",
  } }]);
});
async function openForwardActions(page: Page) {
  // Switch only for the synchronous menu trigger; background polling must retain native IPC.
  await grid(page, "Port forwards").getByRole("button", { name: "Actions for forward-one", exact: true }).evaluate(element => {
    Object.assign(window, { isTauri: false });
    try { (element as HTMLButtonElement).click(); }
    finally { Object.assign(window, { isTauri: true }); }
  });
  await expect(page.getByRole("menuitem", { name: "Details", exact: true })).toBeEnabled();
}
const calls = (page: Page, command: string) => page.evaluate((command) => (window as MockWindow).__serviceCalls.filter((call) => call.command === command), command);
const navigate = (page: Page, section: string) => page.evaluate((section) => { location.hash = `/cluster/one?section=${section}`; }, section);
const controls = (page: Page, value: Controls) => page.evaluate((value) => Object.assign((window as MockWindow).__serviceControls, value), value);
const helmCancelScope = ({ clusterId, sessionId, view, readId }: Request) => ({ clusterId, sessionId, view, readId });
async function refreshHelm(page: Page) {
  await page.getByRole("button", { name: "Refresh helm-releases", exact: true }).click();
  await page.keyboard.press("Tab");
  await page.mouse.move(0, 0);
}
async function refreshForwards(page: Page) {
  await page.getByRole("button", { name: "Refresh port-forwarding", exact: true }).click();
  await page.keyboard.press("Tab");
  await page.mouse.move(0, 0);
}
async function selectNamespace(page: Page, namespace: string) {
  const selector = page.getByRole("button", { name: /^(?:Pod namespace|Namespace)(?! options)(?:\s|$)/ });
  await expect(selector).toBeVisible();
  await expect(page.getByRole("status", { name: "Loading namespaces…", exact: true })).toHaveCount(0);
  await selector.click();
  await page.getByRole("option", { name: namespace, exact: true }).click();
}

async function fixture(page: Page, section = "helm-releases", options: Controls = {}) {
  page.on("pageerror", (error) => { throw error; });
  await page.addInitScript((options) => {
    const calls: Call[] = [];
    const control = { ...options };
    const pending: (() => void)[] = [];
    const pendingCancels: (() => void)[] = [];
    const externalOpens: string[] = [];
    let session = 0;
    let forwards: (Request & { error: string | null })[] = [];
    window.open = (url) => { externalOpens.push(String(url)); return null; };
    Object.assign(window, { isTauri: true, __serviceCalls: calls, __serviceControls: control, __externalOpens: externalOpens,
      __releaseServices: () => pending.splice(0).forEach((resolve) => resolve()),
      __releaseHelmCancels: () => pendingCancels.splice(0).forEach((resolve) => resolve()),
      __TAURI_INTERNALS__: { invoke: async (command: string, args?: { request: Request }) => {
        const request = { ...args?.request };
        calls.push({ command, request });
        if (command === "cluster_permissions_review") return { clusterId: request.clusterId, sessionId: request.sessionId, readId: request.readId,
          authorizationEpoch: String(request.sessionId), revision: 0, checkedAt: Date.now(), expiresAt: Date.now() + 30_000,
          items: (request.checks as unknown[]).map(check => ({ check, decision: "allowed", reason: null })) };
        if (command === "cluster_permissions_cancel" || command === "cluster_permissions_refresh") return null;
        if (command === "catalog_list") return { revision: 1, discoveryError: null, webLinks: [], clusters: [
          { id: "one", name: "Development", contextName: "dev", source: "kubeconfig", labels: [], color: "#476BA1", pinned: false, status: "disconnected" },
        ] };
        if (command === "cluster_session_open" || command === "cluster_connect") return {
          clusterId: request.clusterId, sessionId: request.sessionId ?? `lease-${++session}`, contextName: "dev",
          status: command === "cluster_connect" ? "connected" : "disconnected", apiVersion: command === "cluster_connect" ? "v1.35.8" : null, error: null,
        };
        if (command === "cluster_disconnect") { forwards = []; return; }
        if (command === "cluster_resources_cancel") return;
        if (command === "cluster_resources" && request.kind === "namespaces") return {
          ...request, namespace: null, continueToken: null, truncated: false, fetchedAt: Date.now(), metrics: null,
          items: ["team-a", "team-b", "production", "default"].map((name) => ({
            name, namespace: null, status: "Active", createdAt: null, columns: {}, node: null,
          })),
        };
        if (command === "helm_cancel") {
          if (control.holdCancel) await new Promise<void>((resolve) => pendingCancels.push(resolve));
          return;
        }
        if (command === "helm_table") {
          const failure = control.failure;
          const malformed = control.malformed;
          if (control.hold) await new Promise<void>((resolve) => pending.push(resolve));
          if (failure) throw failure;
          const release = request.view === "releases";
          const item = (name: string, namespace: string | null, status: string | null) => ({
            id: `${namespace ?? "repository"}/${name}`, name, namespace, version: release ? "nginx-1.2.3" : "1.2.3", appVersion: "4.5.6",
            description: release ? null : "Cached HTTP chart", status, revision: release ? 2 : null, updated: release ? "2026-01-01T00:00:00Z" : null,
          });
          const items = control.empty ? [] : [item("edge", release ? String(request.namespace ?? "team-a") : null, release ? "deployed" : null),
            item("worker", release ? String(request.namespace ?? "team-b") : null, release ? "failed" : null)];
          const response = { clusterId: request.clusterId, sessionId: request.sessionId, view: request.view,
            namespace: request.namespace ?? null, readId: request.readId, items, truncated: true };
          if (malformed === "session") response.sessionId = "old-session";
          if (malformed === "cluster") response.clusterId = "other-cluster";
          if (malformed === "view") response.view = release ? "charts" : "releases";
          if (malformed === "namespace") response.namespace = "wrong-namespace";
          if (malformed === "readId") response.readId = -1;
          if (malformed === "itemNamespace" && items[0]) items[0].namespace = "wrong-namespace";
          if (malformed === "missingNamespace") return { ...response, namespace: undefined };
          if (malformed === "missingReadId") return { ...response, readId: undefined };
          return response;
        }
        if (command === "cluster_pod_forward_list") {
          if (control.hold) await new Promise<void>((resolve) => pending.push(resolve));
          if (control.failure) throw control.failure;
          return forwards.map((item) => ({ ...item, ...(control.forwardStatus ? { status: control.forwardStatus } : {}),
            ...(control.malformed === "session" ? { sessionId: "old-session" } : {}) }));
        }
        if (command === "cluster_pod_detail") {
          if (control.holdDetail) await new Promise<void>((resolve) => pending.push(resolve));
          if (control.detailFailure) throw control.detailFailure;
          return { ...request, uid: "resolved-pod-uid", resourceVersion: "123", phase: "Running", node: null, podIps: [], hostIps: [], serviceAccount: null,
            qos: null, createdAt: null, deletionTimestamp: null, containers: [], conditions: [], yaml: "", events: [] };
        }
        if (command === "cluster_pod_forward_start") {
          if (control.holdStart) await new Promise<void>((resolve) => pending.push(resolve));
          if (control.startFailure) throw control.startFailure;
          const item = { clusterId: request.clusterId, sessionId: request.sessionId, id: "forward-one", namespace: request.namespace, name: request.name,
            remotePort: request.remotePort, localPort: request.localPort || 49152, status: "running", error: null };
          forwards.push(item); return item;
        }
        if (command === "cluster_pod_forward_stop") {
          if (control.stopFailure) throw control.stopFailure;
          forwards = forwards.filter((item) => item.id !== request.id); return;
        }
        throw `Unexpected mock command: ${command}`;
      } },
    });
  }, options);
  await page.setViewportSize({ width: 1440, height: 900 });
  await installNamespaceSnapshotMock(page);
  await page.goto(`/#/cluster/one?section=${section}`);
  await expect(page.getByRole("switch", { name: "Resource tables", exact: true })).toHaveCount(0);
}

async function startForward(page: Page) {
  if (!await page.getByRole("dialog").count()) await page.getByRole("button", { name: "Create port forward", exact: true }).click();
  await selectNamespace(page, "team-a");
  await page.getByRole("textbox", { name: "Pod", exact: true }).fill("api-pod");
  await page.getByRole("textbox", { name: "Port", exact: true }).fill("8080");
  await page.getByRole("button", { name: "Start forward", exact: true }).click();
}

test("Helm charts search cached inventory locally and open explicit installation without sending a request", async ({ page }) => {
  await fixture(page, "helm-charts");
  const table = grid(page, "Helm Charts");
  await expect(table).toContainText("Cached HTTP chart");
  await expect(table.getByRole("columnheader")).toHaveText(["Name", "Version", "App version", "Description", "Actions"]);
  await expect(page.getByRole("button", { name: /^Namespace(?! options)(?:\s|$)/ })).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Charts filters", exact: true })).toBeVisible();
  const before = await calls(page, "helm_table");
  expect(before.at(-1)!.request).toEqual({ clusterId: "one", sessionId: expect.stringMatching(/^lease-/), view: "charts", namespace: null, readId: expect.any(Number) });
  await page.getByRole("textbox", { name: "Search Charts", exact: true }).fill(" EDGE ");
  await expect(table.getByRole("row")).toHaveCount(2);
  await page.getByRole("textbox", { name: "Search Charts", exact: true }).fill("absent");
  await expect(table).toContainText("No charts match your filters.");
  expect(await calls(page, "helm_table")).toEqual(before);
  const install = page.getByRole("button", { name: "Install Helm chart", exact: true });
  await expect(install).toBeEnabled(); await install.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("Cluster: Development");
  await expect(dialog).toContainText("Context: dev");
  await expect(dialog.getByRole("textbox", { name: "Release name", exact: true })).toHaveValue("");
  await expect(dialog.getByRole("textbox", { name: "Chart repository/name", exact: true })).toHaveValue("");
  await expect(dialog.getByRole("textbox", { name: "Values YAML", exact: true })).toHaveText("{}\n");
  await expect(dialog.getByRole("button", { name: "Run server dry-run", exact: true })).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "Install release", exact: true })).toHaveCount(0);
  expect(await calls(page, "helm_install")).toEqual([]);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await calls(page, "helm_install")).toEqual([]);
  expect(await calls(page, "helm_install_cancel")).toEqual([]);
  expect(await calls(page, "helm_table")).toEqual(before);
});

test("Helm releases combine local search and status with validated namespace reads", async ({ page }) => {
  await fixture(page);
  const table = grid(page, "Helm Releases");
  await expect(table.getByRole("row")).toHaveCount(3);
  await expect(page.getByRole("status").filter({ hasText: "inventory may be partial" })).toBeVisible();
  const before = await calls(page, "helm_table");
  await page.getByRole("combobox", { name: "Status", exact: true }).click();
  await page.getByRole("option", { name: "failed", exact: true }).click();
  await expect(table).toContainText("worker"); await expect(table).not.toContainText("edge");
  await page.getByRole("textbox", { name: "Search Releases", exact: true }).fill("edge");
  await expect(table).toContainText("No releases match your filters.");
  expect(await calls(page, "helm_table")).toEqual(before);
  await selectNamespace(page, "production");
  await expect(page.getByRole("button", { name: /^Namespace(?! options)(?:\s|$)/ })).toContainText("production");
  const productionRead = (await calls(page, "helm_table")).at(-1)!.request;
  expect(productionRead).toEqual({ clusterId: "one", sessionId: expect.stringMatching(/^lease-/), view: "releases", namespace: "production", readId: expect.any(Number) });
  await selectNamespace(page, "All namespaces");
  await expect(page.getByRole("button", { name: /^Namespace(?! options)(?:\s|$)/ })).toContainText("All namespaces");
  const allNamespacesRead = (await calls(page, "helm_table")).at(-1)!.request;
  expect(allNamespacesRead).toEqual({ clusterId: "one", sessionId: productionRead.sessionId, view: "releases", namespace: null, readId: expect.any(Number) });
});

for (const [failure, message] of [["helmUnavailable", "Helm is not installed"], ["conflict", "diagnostic output is withheld"], ["permissionDenied", "Kubernetes permissions"]]) {
  test(`Helm ${failure} is an actionable error, not a returned-empty inventory`, async ({ page }) => {
    await fixture(page, "helm-releases", { failure });
    await expect(page.getByRole("alert")).toContainText(message);
    await expect(grid(page, "Helm Releases")).toHaveCount(0);
    await expect(page.getByText(/were returned/)).toHaveCount(0);
    await controls(page, { failure: "" });
    await refreshHelm(page);
    await expect(grid(page, "Helm Releases")).toContainText("edge");
  });
}

for (const malformed of ["session", "cluster", "view", "namespace", "readId", "itemNamespace", "missingNamespace", "missingReadId"]) {
  test(`Helm rejects mismatched ${malformed} scope`, async ({ page }) => {
    await fixture(page);
    await expect(grid(page, "Helm Releases")).toContainText("edge");
    await controls(page, { malformed });
    await selectNamespace(page, "production");
    await expect(page.getByRole("alert")).toContainText("unsupported response");
    await expect(grid(page, "Helm Releases")).toHaveCount(0);
    await expect(page.getByText(/were returned/)).toHaveCount(0);
  });
}

test("Helm loading and legitimate empty inventory remain distinct", async ({ page }) => {
  await fixture(page, "helm-charts", { empty: true, hold: true });
  await expect(page.getByRole("status", { name: "Loading Helm charts…", exact: true })).toBeVisible();
  await expect(grid(page, "Helm Charts")).toHaveCount(0);
  await page.evaluate(() => (window as MockWindow).__releaseServices());
  await expect(grid(page, "Helm Charts")).toContainText("No charts were returned.");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("late Helm results cannot cross namespace scopes", async ({ page }) => {
  await fixture(page);
  await expect(grid(page, "Helm Releases")).toContainText("edge");
  await controls(page, { hold: true });
  await refreshHelm(page);
  await expect(page.getByRole("progressbar", { name: "Loading Helm releases…", exact: true })).toBeVisible();
  const canceled = (await calls(page, "helm_table")).at(-1)!.request;
  await controls(page, { hold: false });
  await selectNamespace(page, "production");
  await expect(grid(page, "Helm Releases")).toContainText("production");
  await expect.poll(async () => (await calls(page, "helm_cancel")).at(-1)?.request).toEqual(helmCancelScope(canceled));
  await page.evaluate(() => (window as MockWindow).__releaseServices());
  await expect(grid(page, "Helm Releases")).not.toContainText("team-a");
  await expect(page.getByRole("button", { name: /^Namespace(?! options)(?:\s|$)/ })).toContainText("production");
});

test("port table resolves Pod UID before loopback auto-port start, survives navigation, and stops through IPC", async ({ page }) => {
  await fixture(page, "port-forwarding");
  const table = grid(page, "Port forwards");
  await expect(table).toContainText("No port forwards");
  await page.getByRole("button", { name: "Create port forward", exact: true }).click();
  await expect(page.getByRole("button", { name: "Start forward", exact: true })).toBeDisabled();
  await expect(page.getByRole("textbox", { name: "Local port", exact: true })).toHaveValue("0");
  await startForward(page);
  await expect(table).toContainText("api-pod"); await expect(table).toContainText("127.0.0.1");
  await expect(table).toContainText("49152");
  const detail = (await calls(page, "cluster_pod_detail"))[0];
  const start = (await calls(page, "cluster_pod_forward_start"))[0];
  expect(detail.request).toEqual({ clusterId: "one", sessionId: expect.stringMatching(/^lease-/), namespace: "team-a", name: "api-pod" });
  expect(start.request).toEqual({ ...detail.request, uid: "resolved-pod-uid", remotePort: 8080, localPort: 0 });
  const all = await page.evaluate(() => (window as MockWindow).__serviceCalls);
  expect(all.indexOf(all.find((call) => call.command === "cluster_pod_detail")!)).toBeLessThan(all.indexOf(all.find((call) => call.command === "cluster_pod_forward_start")!));
  const connections = await calls(page, "cluster_connect");
  await navigate(page, "helm-charts"); await expect(grid(page, "Helm Charts")).toBeVisible();
  expect(await calls(page, "cluster_pod_forward_stop")).toHaveLength(0);
  await navigate(page, "port-forwarding"); await expect(table).toContainText("api-pod");
  expect(await calls(page, "cluster_connect")).toEqual(connections);
  expect((await calls(page, "cluster_pod_forward_list")).every((call) => call.request.sessionId === start.request.sessionId)).toBe(true);
  await openForwardActions(page);
  await expect(page.getByRole("menuitem", { name: "Remove", exact: true })).toHaveCount(0);
  await page.getByRole("menuitem", { name: "Stop", exact: true }).click();
  await expect(table).toContainText("No port forwards");
  expect((await calls(page, "cluster_pod_forward_stop"))[0].request).toEqual({ clusterId: "one", sessionId: start.request.sessionId, id: "forward-one" });
  expect(await page.evaluate(() => (window as MockWindow).__externalOpens)).toEqual([]);
  expect(await page.evaluate(() => (window as MockWindow).__serviceCalls.some((call) => /open_url|opener|catalog_open_web_link/.test(call.command)))).toBe(false);
});

test("forward filters are local and failed refresh retains honest stale rows", async ({ page }) => {
  await fixture(page, "port-forwarding"); await expect(grid(page, "Port forwards")).toBeVisible();
  await startForward(page); await expect(grid(page, "Port forwards")).toContainText("api-pod");
  await page.getByRole("textbox", { name: "Search forwards", exact: true }).fill("absent");
  await expect(grid(page, "Port forwards")).toContainText("No forwards match the current filters.");
  await page.getByRole("textbox", { name: "Search forwards", exact: true }).fill("");
  await page.getByRole("combobox", { name: "Namespace filter", exact: true }).click();
  await page.getByRole("option", { name: "team-a", exact: true }).click();
  await page.getByRole("combobox", { name: "Status", exact: true }).click();
  await page.getByRole("option", { name: "running", exact: true }).click();
  await expect(grid(page, "Port forwards")).toContainText("api-pod");
  const search = page.getByRole("textbox", { name: "Search forwards", exact: true });
  await search.fill("absent");
  const empty = grid(page, "Port forwards").locator(".astryx-empty-state");
  await expect(empty.getByRole("heading", { name: "No forwards match the current filters.", exact: true })).toBeVisible();
  await expect(empty).toContainText("Try changing your search or clearing a filter.");
  await empty.getByRole("button", { name: "Clear filters", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(search).toHaveValue("");
  await expect(page.getByRole("combobox", { name: "Namespace filter", exact: true })).toContainText("All namespaces");
  await expect(page.getByRole("combobox", { name: "Status", exact: true })).toContainText("All statuses");
  await expect(grid(page, "Port forwards")).toContainText("api-pod");
  await controls(page, { failure: "network" });
  await refreshForwards(page);
  await expect(page.getByRole("alert")).toContainText("Retained forward states may be stale");
  await expect(grid(page, "Port forwards")).toContainText("api-pod");
  await openForwardActions(page);
  await expect(page.getByRole("menuitem", { name: "Stop", exact: true })).toBeDisabled();
  expect(await calls(page, "cluster_pod_forward_stop")).toEqual([]);
});

test("failed forward Stop retains its row, reports the error, and retries the same session record", async ({ page }) => {
  await fixture(page, "port-forwarding", { stopFailure: "permissionDenied" });
  await startForward(page);
  const start = (await calls(page, "cluster_pod_forward_start"))[0].request;
  const stop = { command: "cluster_pod_forward_stop", request: { clusterId: "one", sessionId: start.sessionId, id: "forward-one" } };
  await openForwardActions(page);
  await page.getByRole("menuitem", { name: "Stop", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Kubernetes permissions");
  await expect(grid(page, "Port forwards")).toContainText("api-pod");
  expect(await calls(page, "cluster_pod_forward_stop")).toEqual([stop]);
  await controls(page, { stopFailure: "" });
  await openForwardActions(page);
  await page.getByRole("menuitem", { name: "Stop", exact: true }).click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(grid(page, "Port forwards")).toContainText("No port forwards");
  expect(await calls(page, "cluster_pod_forward_stop")).toEqual([stop, stop]);
});

test("forward errors never impersonate empty lists and failed UID resolution cannot start a forward", async ({ page }) => {
  await fixture(page, "port-forwarding", { failure: "permissionDenied", detailFailure: "permissionDenied" });
  await expect(page.getByRole("alert")).toContainText("Kubernetes permissions");
  await expect(grid(page, "Port forwards")).toHaveCount(0);
  await expect(page.getByText(/were returned/)).toHaveCount(0);
  await controls(page, { failure: "" });
  await refreshForwards(page);
  await expect(grid(page, "Port forwards")).toContainText("No port forwards");
  await startForward(page);
  await expect(page.getByRole("alert")).toContainText("Kubernetes permissions");
  expect(await calls(page, "cluster_pod_forward_start")).toHaveLength(0);
});

test("forward response rejects another session rather than replacing retained rows", async ({ page }) => {
  await fixture(page, "port-forwarding"); await expect(grid(page, "Port forwards")).toBeVisible();
  await startForward(page); await expect(grid(page, "Port forwards")).toContainText("api-pod");
  await controls(page, { malformed: "session" });
  await refreshForwards(page);
  await expect(page.getByRole("alert")).toContainText("unsupported response");
  await expect(grid(page, "Port forwards")).toContainText("api-pod");
});


test("forward creation FAB opens a bounded accessible form, validates ports, and cancels without IPC", async ({ page }) => {
  await fixture(page, "port-forwarding");
  await expect(grid(page, "Port forwards")).toBeVisible();
  const fab = page.getByRole("button", { name: "Create port forward", exact: true });
  await expect(fab).toBeVisible();
  await expect(fab).toHaveText("");
  await expect(page.getByRole("button", { name: "Refresh", exact: true })).toHaveCount(0);
  await expect(page.getByRole("textbox", { name: "Pod", exact: true })).toHaveCount(0);
  await page.setViewportSize({ width: 390, height: 600 });
  const bounds = await fab.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(600);
  await fab.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Create port forward", exact: true });
  await expect(dialog).toBeVisible();
  const dialogBounds = await dialog.boundingBox();
  expect(dialogBounds!.x).toBeGreaterThanOrEqual(0);
  expect(dialogBounds!.x + dialogBounds!.width).toBeLessThanOrEqual(390);
  await selectNamespace(page, "team-a");
  await dialog.getByRole("textbox", { name: "Pod", exact: true }).fill("Bad Pod");
  await dialog.getByRole("textbox", { name: "Port", exact: true }).fill("65536");
  await dialog.getByRole("textbox", { name: "Local port", exact: true }).fill("-1");
  await expect(dialog.getByRole("button", { name: "Start forward", exact: true })).toBeDisabled();
  await expect(dialog).toContainText("Enter a valid Pod name.");
  await expect(dialog).toContainText("Use a port from 1 to 65535.");
  await expect(dialog).toContainText("Use 0 or a port from 1 to 65535.");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(fab).toBeFocused();
  expect(await calls(page, "cluster_pod_detail")).toEqual([]);
  expect(await calls(page, "cluster_pod_forward_start")).toEqual([]);
});

test("forward start errors stay in the dialog and allow retry", async ({ page }) => {
  await fixture(page, "port-forwarding", { startFailure: "permissionDenied" });
  await startForward(page);
  const dialog = page.getByRole("dialog", { name: "Create port forward", exact: true });
  await expect(dialog.getByRole("alert")).toContainText("Kubernetes permissions");
  await expect(dialog.getByRole("textbox", { name: "Pod", exact: true })).toHaveValue("api-pod");
  await controls(page, { startFailure: "" });
  await dialog.getByRole("button", { name: "Start forward", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(grid(page, "Port forwards")).toContainText("api-pod");
});

test("late forward UID resolution cannot start after its view is disposed", async ({ page }) => {
  await fixture(page, "port-forwarding", { holdDetail: true });
  await startForward(page);
  await expect.poll(async () => (await calls(page, "cluster_pod_detail")).length).toBe(1);
  await navigate(page, "helm-charts");
  await expect(grid(page, "Helm Charts")).toBeVisible();
  await page.evaluate(() => (window as MockWindow).__releaseServices());
  await navigate(page, "port-forwarding");
  await expect(grid(page, "Port forwards")).toBeVisible();
  expect(await calls(page, "cluster_pod_forward_start")).toEqual([]);
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("a late started forward stays owned by the workspace session", async ({ page }) => {
  await fixture(page, "port-forwarding", { holdStart: true });
  await startForward(page);
  await expect.poll(async () => (await calls(page, "cluster_pod_forward_start")).length).toBe(1);
  await expect(page.getByRole("alertdialog").getByRole("button", { name: "Cancel", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await navigate(page, "helm-charts");
  await expect(grid(page, "Helm Charts")).toBeVisible();
  await page.evaluate(() => (window as MockWindow).__releaseServices());
  await navigate(page, "port-forwarding");
  await expect(grid(page, "Port forwards")).toContainText("api-pod");
  expect(await calls(page, "cluster_pod_forward_stop")).toEqual([]);
});
