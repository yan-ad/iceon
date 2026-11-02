import { installNamespaceSnapshotMock } from "./helpers/namespaceSnapshotMock";
import { expect, test, type Page } from "@playwright/test";
import { resourceCreation, resourceViews } from "../src/app/cluster/resourceViews";

type Call = { command: string; request: Record<string, string | number | null> };
type Controls = { failure?: string; malformed?: string; empty?: boolean; hold?: boolean; failContinuation?: boolean; manyRows?: boolean; writeAccess?: "denied" | "unknown" };
type MockWindow = Window & {
  __resourceCalls: Call[];
  __resourceControls: Controls;
  __releaseResources: () => void;
};
const route = (section: string) => `/#/cluster/one?section=${section}`;
const table = (page: Page, name = "Pods") => page.getByRole("table", { name, exact: true });

test("resource ages advance locally without a new inventory read", async ({ page }) => {
  await page.clock.install({ time: new Date("2026-01-18T02:04:00Z") });
  await fixture(page, { section: "resource-quotas" });
  const grid = table(page, "Resource Quotas");
  const cell = grid.getByRole("row").nth(1).getByRole("cell").filter({ hasText: /^2w 3d 2h 4m$/ });
  await expect(cell).toHaveCount(1);
  await page.getByRole("textbox", { name: "Search Resource Quotas", exact: true }).focus();
  await page.clock.pauseAt(await page.evaluate(() => Date.now() + 100));
  const before = await calls(page);
  await page.clock.fastForward(60_000);
  await expect(grid.getByRole("row").nth(1)).toContainText("2w 3d 2h 5m");
  expect(await calls(page)).toEqual(before);
});

test("Cluster overview reads Nodes then warning Events in its existing lease", async ({ page }) => {
  await fixture(page, { section: "overview" });
  await expect(page.getByRole("heading", { name: "Cluster overview", exact: true })).toBeVisible();
  await expect(page.getByText("Nodes loaded: 1", { exact: true })).toBeVisible();
  await expect(page.getByText("No warning events returned.", { exact: true })).toBeVisible();
  const reads = await calls(page);
  expect(reads.map(call => call.request.kind).filter((kind, index, all) => index === 0 || kind !== all[index - 1])).toEqual(["nodes", "events"]);
  const nodeRead = reads.find(call => call.request.kind === "nodes")!;
  const eventRead = reads.find(call => call.request.kind === "events")!;
  expect(nodeRead.request).toEqual({ clusterId: "one", sessionId: expect.stringMatching(/^lease-/), kind: "nodes", readId: expect.any(Number) });
  expect(eventRead.request).toEqual({ clusterId: "one", sessionId: nodeRead.request.sessionId, kind: "events", readId: expect.any(Number), status: "Warning" });
  expect(reads.filter(call => call.request.kind === "nodes").length).toBeGreaterThanOrEqual(1);
  const cancelled = await calls(page, "cluster_resources_cancel");
  for (const read of reads.filter(call => call.request.kind === "nodes").slice(0, -1)) expect(cancelled).toContainEqual({ command: "cluster_resources_cancel", request: read.request });
  const connects = await calls(page, "cluster_connect");
  expect(connects).toHaveLength(1);
  expect(await calls(page, "cluster_custom_resource_definitions")).toEqual([]);
  await navigate(page, "pods"); await ready(page);
  expect(await calls(page, "cluster_connect")).toEqual(connects);
});

test("table headers stay below the fixed filter toolbar while rows scroll", async ({ page }) => {
  await fixture(page, { section: "resource-quotas", manyRows: true });
  const grid = table(page, "Resource Quotas");
  await expect(grid.getByRole("row")).toHaveCount(81);
  const filters = page.getByRole("region", { name: "Resource Quotas filters", exact: true });
  const header = grid.getByRole("columnheader").first();
  const scroll = page.getByRole("group", { name: "Table", exact: true });
  const toolbarBefore = (await filters.boundingBox())!;
  const headerBefore = (await header.boundingBox())!;
  expect(await scroll.evaluate(element => element.scrollHeight > element.clientHeight)).toBe(true);
  await scroll.evaluate(element => { element.scrollTop = 400; });
  await expect.poll(() => scroll.evaluate(element => element.scrollTop)).toBeGreaterThan(0);
  const toolbarAfter = (await filters.boundingBox())!;
  const headerAfter = (await header.boundingBox())!;
  expect(Math.abs(toolbarAfter.y - toolbarBefore.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(headerAfter.y - headerBefore.y)).toBeLessThanOrEqual(1);
  expect(headerAfter.y).toBeGreaterThanOrEqual(toolbarAfter.y + toolbarAfter.height - 1);
  await expect(page.getByRole("textbox", { name: "Search Resource Quotas", exact: true })).toBeVisible();
});

for (const width of [1440, 390]) for (const colorScheme of ["light", "dark"] as const) test(`floating actions cover rows without intercepting selection at ${width} in ${colorScheme}`, async ({ page }) => {
  await page.emulateMedia({ colorScheme });
  await fixture(page, { section: "resource-quotas", manyRows: true });
  await page.setViewportSize({ width, height: 900 });
  if (width === 390) await page.getByRole("table", { name: "Resource Quotas", exact: true }).evaluate(element => { element.style.zoom = "1.25"; });
  const overlay = page.getByRole("group", { name: "Resource actions", exact: true });
  const viewport = page.locator("[data-iceon-table-viewport]");
  const create = overlay.getByRole("button", { name: /Create/, exact: false });
  const remove = overlay.getByRole("button", { name: "Delete selected resources", exact: true });
  await expect(create).toBeVisible(); await expect(remove).toHaveCount(0);
  await expect(page.getByText("Changes require verified access to their target resources.", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Access unverified", { exact: true })).toHaveCount(0);
  const frame = (await viewport.boundingBox())!, fade = (await overlay.boundingBox())!;
  expect(Math.abs(frame.y + frame.height - fade.y - fade.height)).toBeLessThan(2);
  expect(fade.y).toBeLessThan(frame.y + frame.height - 40);
  const styles = await overlay.evaluate(element => {
    const style = getComputedStyle(element);
    return { events: style.pointerEvents, background: style.backgroundImage, surface: style.getPropertyValue("--color-background-surface").trim() };
  });
  expect(styles.events).toBe("none"); expect(styles.background).toContain("linear-gradient(to top");
  expect(styles.background).toContain("rgba(0, 0, 0, 0)"); expect(styles.surface).not.toBe("");
  const behind = await viewport.getByRole("checkbox").evaluateAll((elements, fade) => elements.findIndex(element => {
    const rect = element.getBoundingClientRect();
    return rect.y > fade.y && rect.bottom < fade.y + fade.height;
  }), fade);
  expect(behind).toBeGreaterThan(0);
  const checkbox = viewport.getByRole("checkbox").nth(behind);
  const hit = (await checkbox.locator("..").boundingBox())!;
  await page.mouse.click(hit.x + hit.width / 2, hit.y + hit.height / 2); await expect(checkbox).toBeChecked(); await expect(remove).toBeEnabled();
  await page.getByRole("button", { name: "Clear selection", exact: true }).click(); await expect(remove).toHaveCount(0);
  await viewport.evaluate(element => { element.scrollTop = element.scrollHeight; });
  const last = viewport.getByRole("checkbox", { name: "Select team-a/resource-77", exact: true });
  const lastBox = (await last.boundingBox())!;
  expect(lastBox.y + lastBox.height).toBeLessThanOrEqual((await overlay.boundingBox())!.y);
  await last.click(); await expect(last).toBeChecked(); await expect(remove).toBeEnabled();
  await expect(create).toHaveCSS("pointer-events", "auto"); await expect(remove).toHaveCSS("pointer-events", "auto");
  await create.click(); await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape"); await remove.click(); await expect(page.getByRole("dialog")).toContainText("team-a/resource-77");
  await page.keyboard.press("Escape");
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
});

test("changing filters on a continuation page resets pagination and cancels the exact read", async ({ page }) => {
  await fixture(page); await ready(page);
  await clusterAction(page, "Next page");
  await expect(table(page)).toContainText("second-page-hit");
  const continued = (await calls(page)).at(-1)!.request;
  await page.getByRole("textbox", { name: "Search Pods", exact: true }).fill("shared-name");
  await expect(table(page).getByRole("row")).toHaveCount(3);
  await expect(table(page)).not.toContainText("second-page-hit");
  expect((await calls(page)).at(-1)!.request).toMatchObject({ query: "shared-name", continueToken: null });
  expect((await calls(page, "cluster_resources_cancel"))).toContainEqual({ command: "cluster_resources_cancel", request: continued });
  await page.getByRole("button", { name: "Cluster actions", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Previous page", exact: true })).toBeDisabled();
  await expect(page.getByRole("menuitem", { name: "Next page", exact: true })).toBeDisabled();
  await expect(page.getByRole("menuitem", { name: "Enable live sync", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.mouse.move(0, 0);
  await page.getByRole("button", { name: "Clear filters", exact: true }).click();
  await ready(page);
  await page.getByRole("button", { name: "Cluster actions", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Previous page", exact: true })).toBeDisabled();
  await expect(page.getByRole("menuitem", { name: "Next page", exact: true })).toBeEnabled();
  await expect(page.getByRole("menuitem", { name: "Enable live sync", exact: true })).toBeEnabled();
});

for (const section of ["nodes", "resource-quotas"]) for (const writeAccess of ["denied", "unknown"] as const) for (const width of [1440, 390]) test(`${section} ${writeAccess} access feedback stays discoverable at ${width}`, async ({ page }) => {
  await fixture(page, { section, manyRows: true, writeAccess });
  await page.setViewportSize({ width, height: 900 });
  if (section === "resource-quotas") await selectNamespace(page, "team-a");
  const message = writeAccess === "denied" ? "Read-only access." : "Access unverified.";
  const viewport = page.locator("[data-iceon-table-viewport]");
  await expect(viewport).toBeVisible();
  if (section === "resource-quotas") {
    await expect(page.getByRole("status").filter({ hasText: message })).toHaveCount(0);
    const options = page.getByRole("button", { name: "Namespace options", exact: true });
    await options.hover();
    await expect(page.getByRole("tooltip")).toHaveText("Namespace options");
    const before = (await viewport.boundingBox())!;
    await options.click();
    await expect(page.getByRole("dialog", { name: "Namespace options", exact: true })).toContainText(writeAccess === "denied" ? message : "Permissions could not be verified. Editing is unavailable.");
    expect((await viewport.boundingBox())!.y).toBe(before.y);
    await page.keyboard.press("Escape");
  } else {
    await expect(page.getByRole("status").filter({ hasText: message })).toBeVisible();
  }
  await expect.poll(() => page.evaluate(() => {
    const frame = document.querySelector("[data-iceon-table-viewport]")?.getBoundingClientRect();
    const fade = document.querySelector('[role="group"][aria-label="Resource actions"]')?.getBoundingClientRect();
    return !!frame && !!fade && Math.abs(frame.bottom - fade.bottom) < 2;
  })).toBe(true);
  await expect(page.getByRole("button", { name: /^Create / })).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
});

async function clusterAction(page: Page, name: string) {
  if (name.startsWith("Refresh ")) {
    await page.getByRole("button", { name, exact: true }).click();
    return;
  }
  await page.getByRole("button", { name: "Cluster actions", exact: true }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}

test("failed continuation keeps the last page and allows returning to it", async ({ page }) => {
  await fixture(page, { failContinuation: true }); await ready(page);
  await clusterAction(page, "Next page");
  await expect(page.getByRole("alert")).toContainText("resource is unavailable");
  await expect(table(page)).toContainText("shared-name");
  await page.getByRole("button", { name: "Cluster actions", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Previous page", exact: true })).toBeEnabled();
  await page.getByRole("menuitem", { name: "Previous page", exact: true }).click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(table(page)).toContainText("shared-name");
});
const calls = (page: Page, command = "cluster_resources", kind?: string) => page.evaluate(
  ({ command, kind }) => (window as MockWindow).__resourceCalls.filter((call) => call.command === command &&
    (command !== "cluster_resources" || (kind ? call.request.kind === kind : call.request.kind !== "namespaces"))), { command, kind },
);
async function selectNamespace(page: Page, namespace: string) {
  const selector = page.getByRole("button", { name: /^Namespace(?! options)(?:\s|$)/ });
  await expect(selector).toBeVisible();
  await expect(page.getByRole("status", { name: "Loading namespaces…", exact: true })).toHaveCount(0);
  await selector.click();
  await page.getByRole("option", { name: namespace, exact: true }).click();
}
async function controls(page: Page, value: Controls) {
  await page.evaluate((value) => Object.assign((window as MockWindow).__resourceControls, value), value);
}

test("parent menus expose complete destination cards without resource fan-out", async ({ page }) => {
  await fixture(page, { section: "workloads" });
  const workloads = page.getByRole("navigation", { name: "Workloads destinations", exact: true });
  await expect(workloads).toBeVisible();
  await expect(workloads.getByRole("link")).toHaveCount(8);
  expect(await workloads.getByRole("link").evaluateAll(links => links.map(link => link.getAttribute("href")))).toEqual(
    ["pods", "deployments", "daemon-sets", "stateful-sets", "replica-sets", "replication-controllers", "jobs", "cron-jobs"].map(id => `#/cluster/one?section=${id}`),
  );
  expect(await calls(page)).toHaveLength(0);
  await workloads.getByRole("link", { name: "Pods", exact: true }).click();
  await ready(page);
  const connects = await calls(page, "cluster_connect");
  const before = await calls(page);
  for (const [section, label, ids] of [
    ["config", "Config", ["config-maps", "secrets", "resource-quotas", "limit-ranges", "horizontal-pod-autoscalers", "vertical-pod-autoscalers", "pod-disruption-budgets", "priority-classes", "runtime-classes", "leases", "mutating-webhook-configs", "validating-webhook-configs", "validating-admission-policies", "validating-admission-policy-bindings"]],
    ["network", "Network", ["services", "endpoint-slices", "endpoints", "ingresses", "ingress-classes", "network-policies", "port-forwarding"]],
    ["storage", "Storage", ["persistent-volume-claims", "persistent-volumes", "storage-classes"]],
    ["helm", "Helm", ["helm-charts", "helm-releases"]],
    ["access-control", "Access Control", ["users", "service-accounts", "cluster-roles", "roles", "cluster-role-bindings", "role-bindings", "pod-security-policies"]],
    ["custom-resources", "Custom Resources", ["custom-resource-definitions"]],
  ] as const) {
    await navigate(page, section);
    const destinations = page.getByRole("navigation", { name: `${label} destinations`, exact: true });
    await expect(destinations).toBeVisible();
    await expect(destinations.getByRole("link")).toHaveCount(ids.length);
    expect(await destinations.getByRole("link").evaluateAll(links => links.map(link => link.getAttribute("href")))).toEqual(ids.map(id => `#/cluster/one?section=${id}`));
    expect(await calls(page)).toEqual(before);
    if (section === "custom-resources") {
      await expect.poll(async () => (await calls(page, "cluster_custom_resource_definitions")).length).toBeGreaterThan(0);
      const discoveries = await calls(page, "cluster_custom_resource_definitions");
      for (const { request } of discoveries) expect(request).toEqual({ clusterId: "one", sessionId: connects.at(-1)!.request.sessionId,
        readId: expect.any(Number), continueToken: null });
    } else expect(await calls(page, "cluster_custom_resource_definitions")).toEqual([]);
  }
  expect(await calls(page, "cluster_connect")).toEqual(connects);
  const discoveries = await calls(page, "cluster_custom_resource_definitions");
  await navigate(page, "storage");
  await expect(page.getByRole("navigation", { name: "Storage destinations", exact: true })).toBeVisible();
  await expect.poll(async () => (await calls(page, "cluster_custom_resource_definitions_cancel")).map(call => call.request)).toEqual(discoveries.map(call => call.request));
});

test("Nodes retains its own table and lease across resource navigation", async ({ page }) => {
  await fixture(page, { section: "nodes" });
  await expect(table(page, "Nodes")).toContainText("node-a");
  await expect(table(page, "Nodes").getByRole("columnheader", { name: "CPU", exact: true })).toBeVisible();
  const connects = await calls(page, "cluster_connect");
  await navigate(page, "pods"); await ready(page);
  await navigate(page, "nodes");
  await expect(table(page, "Nodes")).toContainText("node-a");
  expect(await calls(page, "cluster_connect")).toEqual(connects);
});
async function navigate(page: Page, section: string) {
  await page.evaluate((section) => { location.hash = `/cluster/one?section=${section}`; }, section);
}
async function fixture(page: Page, options: Controls & { legacyPreference?: boolean; section?: string } = {}) {
  page.on("pageerror", (error) => { throw error; });
  await page.addInitScript(({ options, views }) => {
    if (options.legacyPreference !== undefined) localStorage.setItem("iceon.resource-tables.v1", String(options.legacyPreference));
    let session = 0;
    const calls: Call[] = [];
    const control = { ...options };
    const pending: (() => void)[] = [];
    Object.assign(window, { isTauri: true, __resourceCalls: calls, __resourceControls: control,
      __releaseResources: () => pending.splice(0).forEach((resolve) => resolve()),
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args?: { request: Call["request"] }) => {
          const request = { ...args?.request };
          if (command === "cluster_permissions_cancel") return null;
          if (command === "cluster_permissions_review") {
            const review = args!.request as unknown as import("../src/app/cluster/permissionsApi").PermissionReviewRequest;
            return { clusterId: review.clusterId, sessionId: review.sessionId, readId: review.readId, authorizationEpoch: `fixture-epoch-${review.sessionId}`, revision: review.readId, checkedAt: Date.now(), expiresAt: Date.now() + 30000, items: review.checks.map(check => ({ check, decision: ["create", "update", "patch", "delete"].includes(check.action) ? control.writeAccess ?? "allowed" : "allowed", reason: null })) };
          }
          calls.push({ command, request });
          if (command === "catalog_list") return { revision: 1, discoveryError: null, webLinks: [], clusters: [
            { id: "one", name: "Development", contextName: "dev", source: "kubeconfig", labels: [],
              color: "#476BA1", pinned: false, status: "disconnected" },
          ] };
          if (command === "cluster_session_open" || command === "cluster_connect") return {
            clusterId: request.clusterId, sessionId: request.sessionId ?? `lease-${++session}`, contextName: "dev",
            status: command === "cluster_connect" ? "connected" : "disconnected",
            apiVersion: command === "cluster_connect" ? "v1.35.8" : null, error: null,
          };
          if (command === "cluster_disconnect" || command === "cluster_resources_cancel" || command === "helm_cancel" || command === "cluster_custom_resource_definitions_cancel") return null;
          if (command === "cluster_custom_resource_definitions") return { ...request, kind: "customResourceDefinitions", namespace: null,
            items: [],
            continueToken: null, truncated: false, fetchedAt: Date.now(), metrics: null };
          if (command === "helm_table") return { clusterId: request.clusterId, sessionId: request.sessionId,
            view: request.view, namespace: request.namespace ?? null, readId: request.readId, truncated: false, items: [{ id: "helm-row", name: "sample-chart", namespace: request.view === "releases" ? String(request.namespace ?? "team-a") : null,
              version: "1.2.3", appVersion: "4.5.6", description: request.view === "releases" ? null : "Cached chart", status: request.view === "releases" ? "deployed" : null,
              revision: request.view === "releases" ? 1 : null, updated: request.view === "releases" ? "2026-10-26T00:00:00Z" : null }] };
          if (command === "cluster_pod_forward_list") return [{ ...request, id: "forward-one", namespace: "team-a", name: "sample-pod",
            remotePort: 8080, localPort: 49152, status: "running", error: null }];
          if (command !== "cluster_resources") throw `Unexpected mock command: ${command}`;
          if (request.kind === "namespaces" && !location.hash.includes("section=namespaces")) return {
            ...request, namespace: null, continueToken: null, truncated: false, fetchedAt: Date.now(), metrics: null,
            items: ["team-a", "team-b", "production", "default"].map((name) => ({
              name, namespace: null, status: "Active", createdAt: null, columns: {}, node: null,
            })),
          };
          if (request.kind === "nodes") return { ...request, namespace: null, continueToken: null, truncated: false,
            fetchedAt: Date.now(), metrics: null, items: [{ name: "node-a", status: "Ready", namespace: null, createdAt: null, columns: {}, node: null }] };
          const view = views.find((view) => view.kind === request.kind);
          if (!view) throw "invalidInput";
          if (control.hold) await new Promise<void>((resolve) => pending.push(resolve));
          if (control.failContinuation && request.continueToken) throw "resourceUnavailable";
          if (control.failure) throw control.failure;
          const columns = Object.fromEntries(view.columns.map((column) => [column.key, `fact-${column.key}`]));
          const item = (name: string, namespace: string | null, status: string, group: string) => ({
            name, namespace, uid: `uid-${namespace ?? "cluster"}-${name}`, resourceVersion: "123", status, createdAt: "2026-01-01T00:00:00Z", node: null,
            columns: request.kind === "customResourceDefinitions" ? { ...columns, group } : columns,
            data: { "private-key-name": "PRIVATE_VALUE_MUST_NOT_ESCAPE" },
            stringData: { "another-private-key": "ANOTHER_PRIVATE_VALUE" },
          });
          const ns = view.namespaced ? String(request.namespace ?? "team-a") : null;
          const firstPage = [item("shared-name", ns, "Active", "apps.example.test"),
            item(view.namespaced && request.namespace == null ? "shared-name" : "other-name",
              view.namespaced && request.namespace == null ? "team-b" : ns, "Pending", "ops.example.test")];
          if (control.manyRows) firstPage.push(...Array.from({ length: 78 }, (_, index) =>
            item(`resource-${index}`, ns, "Active", "apps.example.test")));
          const secondPage = [item("second-page-hit", ns, "Active", "apps.example.test")];
          const filterQuery = typeof request.query === "string" ? request.query.trim().toLocaleLowerCase() : "";
          const filterStatus = typeof request.status === "string" ? request.status : "";
          const filterGroup = typeof request.group === "string" ? request.group : "";
          const filteredRequest = !!(filterQuery || filterStatus || filterGroup);
          const matches = (candidate: ReturnType<typeof item>) => {
            const searchable = [candidate.name, candidate.namespace, candidate.status, ...Object.values(candidate.columns)]
              .filter(Boolean).join(" ").toLocaleLowerCase();
            return (!filterQuery || searchable.includes(filterQuery)) &&
              (!filterStatus || candidate.status === filterStatus) &&
              (!filterGroup || candidate.columns.group === filterGroup);
          };
          // Filtered reads model Rust scanning continuation pages: the hit may originate on
          // page two, but the renderer receives one complete filtered response.
          const items = filteredRequest ? [...firstPage, ...secondPage].filter(matches)
            : request.continueToken ? secondPage : firstPage;
          const response = { ...request, namespace: request.namespace ?? null, items: control.empty ? [] : items,
            fetchedAt: Date.now(), truncated: !control.empty && !filteredRequest && !request.continueToken,
            continueToken: control.empty || filteredRequest || request.continueToken ? null : "next-page-token", metrics: null };
          switch (control.malformed) {
            case "namespace": response.namespace = "wrong-namespace"; break;
            case "session": response.sessionId = "wrong-session"; break;
            case "readId": response.readId = -1; break;
            case "columns": items[0].columns = { bad: 42 } as unknown as typeof columns; break;
            case "duplicate": response.items = [items[0], items[0]]; break;
            case "timestamp": items[0].createdAt = "not-a-time"; break;
            case "token": response.continueToken = "bad\ntoken"; break;
          }
          return response;
        },
      },
    });
  }, { options, views: resourceViews });
  await page.setViewportSize({ width: 1440, height: 900 });
  await installNamespaceSnapshotMock(page);
  await page.goto(route(options.section ?? "pods"));
  await expect(page.getByRole("switch", { name: "Resource tables", exact: true })).toHaveCount(0);
}
async function ready(page: Page, name = "Pods") {
  await expect(table(page, name)).toBeVisible();
  await expect(table(page, name).getByRole("row")).toHaveCount(3);
  await expect(page.getByRole("status", { name: "Loading namespaces…", exact: true })).toHaveCount(0);
}

for (const view of resourceViews) test(`registry ${view.section}: direct route, columns and explicit creation contract`, async ({ page }) => {
  expect(resourceViews).toHaveLength(43);
  await fixture(page, { section: view.section });
    await test.step(view.section, async () => {
      await navigate(page, view.section);
      const helm = view.section === "helm-charts" || view.section === "helm-releases";
      const forward = view.section === "port-forwarding";
      const grid = table(page, helm ? `Helm ${view.label}` : forward ? "Port forwards" : view.label);
      await expect(grid).toBeVisible();
      await expect(grid.getByRole("columnheader")).toHaveText(helm ? (view.section === "helm-charts"
        ? ["Name", "Version", "App version", "Description", "Actions"]
        : ["Name", "Namespace", "Chart", "App version", "Status", "Revision", "Updated", "Actions"])
        : forward ? ["Name", "Namespace", "Pod", "Port", "Local Port", "Address", "Status", "Actions"]
        : ["Select all rows", "Name", ...(view.kind === "pods" ? ["Health"] : []), ...(view.namespaced ? ["Namespace"] : []), ...view.columns.map((column) => column.label), "Age", "Status", "Actions"]);
      await expect(page.getByText(/service is not implemented|unavailable because|Live .* data is not implemented yet/)).toHaveCount(0);
      if (view.kind) {
        await expect(grid.getByRole("row")).toHaveCount(3);
        const read = (await calls(page, "cluster_resources", view.kind)).at(-1)!.request;
        expect(read).toMatchObject({ clusterId: "one", kind: view.kind, namespace: null, continueToken: null });
        expect(read.sessionId).toMatch(/^lease-/);
        expect(read.readId).toEqual(expect.any(Number));
        for (const column of view.columns) await expect(grid).toContainText(
          view.kind === "customResourceDefinitions" && column.key === "group" ? "apps.example.test" : `fact-${column.key}`,
        );
        await expect(page.getByRole("button", { name: /^Namespace(?! options)(?:\s|$)/ })).toHaveCount(view.namespaced ? 1 : 0);
      } else {
        expect(helm || forward).toBe(true);
        await expect(grid.getByRole("row")).toHaveCount(2);
        await expect(page.getByRole("textbox", { name: forward ? "Search forwards" : `Search ${view.label}`, exact: true })).toBeEnabled();
        const serviceCalls = await calls(page, helm ? "helm_table" : "cluster_pod_forward_list");
        expect(serviceCalls.at(-1)!.request).toMatchObject({ clusterId: "one", sessionId: expect.stringMatching(/^lease-/) });
        if (helm) expect(serviceCalls.at(-1)!.request).toEqual({ clusterId: "one", sessionId: expect.stringMatching(/^lease-/),
          view: view.section === "helm-charts" ? "charts" : "releases", namespace: null, readId: expect.any(Number) });
      }
      if (view.kind || view.createLabel || helm) {
        const createLabel = helm ? "Install Helm chart" : view.createLabel ?? `Create ${resourceCreation[view.kind!].kind}`;
        const create = page.getByRole("button", { name: createLabel, exact: true });
        const beforeAction = await calls(page, "cluster_connect");
        if (view.kind) {
          await expect(create).toBeEnabled();
          await create.click();
          const editor = page.getByRole("dialog", { name: createLabel, exact: true });
          await expect(editor).toBeVisible();
          await expect(editor.getByRole("textbox", { name: "Resource YAML", exact: true })).toHaveText(
            `apiVersion: ${resourceCreation[view.kind].apiVersion}\nkind: ${resourceCreation[view.kind].kind}\nmetadata:\n  name: ""\n`,
          );
          await expect(editor.getByRole("button", { name: "Validate YAML", exact: true })).toBeEnabled();
          expect(await calls(page, "cluster_resource_create")).toHaveLength(0);
          await editor.getByRole("button", { name: "Cancel", exact: true }).click();
        } else if (helm) {
          await expect(create).toBeEnabled();
          await create.click();
          const editor = page.getByRole("dialog", { name: createLabel, exact: true });
          await expect(editor).toBeVisible();
          await expect(editor.getByRole("button", { name: "Run server dry-run", exact: true })).toBeDisabled();
          expect(await calls(page, "helm_install")).toHaveLength(0);
          await editor.getByRole("button", { name: "Cancel", exact: true }).click();
        } else {
          await expect(create).toBeDisabled();
          await create.dispatchEvent("click");
        }
        await expect(page.getByRole("dialog")).toHaveCount(0);
        expect(await calls(page, "cluster_resource_create")).toHaveLength(0);
        expect(await calls(page, "cluster_connect")).toEqual(beforeAction);
        expect((await page.evaluate(() => (window as MockWindow).__resourceCalls)).every(
          (call) => ["catalog_list", "cluster_session_open", "cluster_connect", "cluster_disconnect", "cluster_resources", "cluster_resources_cancel", "cluster_custom_resource_definitions", "cluster_custom_resource_definitions_cancel", "helm_table", "helm_cancel", "cluster_pod_forward_list"].includes(call.command),
        )).toBe(true);
      }
    });
});

test("namespace selector loads known namespaces and applies immediately", async ({ page }) => {
  await fixture(page); await ready(page);
  await expect(table(page).getByRole("cell", { name: "shared-name", exact: true })).toHaveCount(2);
  await expect(table(page)).toContainText("team-a"); await expect(table(page)).toContainText("team-b");
  const before = (await calls(page)).length;
  await selectNamespace(page, "production");
  await expect(table(page)).toContainText("production");
  expect((await calls(page)).at(-1)!.request).toMatchObject({ namespace: "production", continueToken: null });
  await selectNamespace(page, "team-a");
  await expect(table(page)).toContainText("team-a");
  await selectNamespace(page, "All namespaces");
  await expect(table(page)).toContainText("team-b");
  expect((await calls(page)).at(-1)!.request.namespace).toBeNull();
  expect((await calls(page)).length).toBeGreaterThan(before);
});

test("search and Status filter are sent to Rust and can find continuation-page rows", async ({ page }) => {
  await fixture(page); await ready(page);
  const search = page.getByRole("textbox", { name: "Search Pods", exact: true });
  await search.fill("second-page-hit");
  await expect(table(page).getByRole("row")).toHaveCount(2);
  await expect(table(page)).toContainText("second-page-hit");
  expect((await calls(page)).at(-1)!.request).toMatchObject({ query: "second-page-hit", continueToken: null });
  await search.fill(" TEAM-B ");
  await expect(table(page).getByRole("row")).toHaveCount(2);
  await expect(table(page)).toContainText("team-b");
  expect((await calls(page)).at(-1)!.request.query).toBe("TEAM-B");
  await search.fill("FACT-QOS");
  await expect(table(page).getByRole("row")).toHaveCount(4);
  await search.fill("");
  await ready(page);
  await page.getByRole("combobox", { name: "Status", exact: true }).click();
  await page.getByRole("option", { name: "Pending", exact: true }).click();
  await expect(table(page).getByRole("row")).toHaveCount(2);
  await expect(table(page)).toContainText("team-b");
  expect((await calls(page)).at(-1)!.request).toMatchObject({ status: "Pending", continueToken: null });
  await search.fill("absent");
  await expect(table(page)).toContainText("No pods match your filters.");
  await expect(table(page)).not.toContainText("No pods found");
  await table(page).getByRole("button", { name: "Clear filters", exact: true }).click();
  await ready(page); await expect(search).toHaveValue("");
  expect((await calls(page)).at(-1)!.request).not.toHaveProperty("query");
});

test("CRD Group filter combines with Status and clears with a fresh unfiltered read", async ({ page }) => {
  await fixture(page, { section: "custom-resource-definitions" }); await ready(page, "Definitions");
  await page.getByRole("button", { name: "Group", exact: true }).click();
  await page.getByRole("option", { name: "apps.example.test", exact: true }).click();
  await expect(table(page, "Definitions").getByRole("row")).toHaveCount(3);
  await expect(table(page, "Definitions")).not.toContainText("other-name");
  await page.getByRole("combobox", { name: "Status", exact: true }).click();
  await page.getByRole("option", { name: "Active", exact: true }).click();
  await expect(table(page, "Definitions").getByRole("row")).toHaveCount(3);
  expect((await calls(page)).at(-1)!.request).toMatchObject({ status: "Active", group: "apps.example.test", continueToken: null });
  await page.getByRole("textbox", { name: "Search Definitions", exact: true }).fill("absent");
  await expect(table(page, "Definitions")).toContainText("No definitions match your filters.");
  expect((await calls(page)).at(-1)!.request).toMatchObject({ query: "absent", status: "Active", group: "apps.example.test", continueToken: null });
  await table(page, "Definitions").getByRole("button", { name: "Clear filters", exact: true }).click();
  await ready(page, "Definitions"); expect((await calls(page)).at(-1)!.request).not.toHaveProperty("group");
});

test("pagination and namespace changes cancel scoped reads and reset continuation", async ({ page }) => {
  await fixture(page); await ready(page);
  const first = (await calls(page)).at(-1)!.request;
  await page.getByRole("button", { name: "Cluster actions", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Previous page", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("status").filter({ hasText: /^2\+ items$/ })).toBeVisible();
  await clusterAction(page, "Next page");
  await expect(table(page)).toContainText("second-page-hit");
  expect((await calls(page)).at(-1)!.request).toMatchObject({ namespace: null, continueToken: "next-page-token" });
  expect((await calls(page, "cluster_resources_cancel")).some((call) => JSON.stringify(call.request) === JSON.stringify(first))).toBe(true);
  await page.getByRole("button", { name: "Cluster actions", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Next page", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
  await clusterAction(page, "Previous page"); await ready(page);
  expect((await calls(page)).at(-1)!.request.continueToken).toBeNull();
  await clusterAction(page, "Next page");
  await expect(table(page)).toContainText("second-page-hit");
  const previous = (await calls(page)).at(-1)!.request;
  await selectNamespace(page, "production"); await ready(page);
  expect((await calls(page)).at(-1)!.request).toMatchObject({ namespace: "production", continueToken: null });
  await expect(table(page)).toContainText("production");
  expect((await calls(page, "cluster_resources_cancel")).some((call) => call.request.readId === previous.readId)).toBe(true);
  const scoped = (await calls(page)).at(-1)!.request;
  await navigate(page, "services"); await ready(page, "Services");
  expect((await calls(page, "cluster_resources_cancel")).some((call) => call.request.readId === scoped.readId)).toBe(true);
  expect((await calls(page)).at(-1)!.request).toMatchObject({ kind: "services", namespace: "production", continueToken: null });
});

test("late results from a cancelled namespace read cannot replace the current scope", async ({ page }) => {
  await fixture(page); await ready(page);
  await controls(page, { hold: true });
  await clusterAction(page, "Next page");
  await expect(page.getByRole("progressbar", { name: "Loading pods…", exact: true })).toBeVisible();
  const pending = (await calls(page)).at(-1)!.request;
  await controls(page, { hold: false });
  await selectNamespace(page, "production");
  await ready(page);
  await expect.poll(async () => (await calls(page, "cluster_resources_cancel")).some(
    (call) => JSON.stringify(call.request) === JSON.stringify(pending),
  )).toBe(true);
  await page.evaluate(() => (window as MockWindow).__releaseResources());
  await expect(table(page)).not.toContainText("second-page-hit");
  await expect(table(page)).toContainText("production");
  await expect(table(page)).toContainText("production");
  await page.getByRole("button", { name: "Cluster actions", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Previous page", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
});

test("resource tables default on and navigation preserves the lease", async ({ page }) => {
  await fixture(page); await ready(page);
  const connections = await calls(page, "cluster_connect");
  await navigate(page, "services"); await ready(page, "Services");
  await navigate(page, "pods"); await ready(page);
  expect(await calls(page, "cluster_connect")).toEqual(connections);
  expect(await page.evaluate(() => localStorage.getItem("iceon.resource-tables.v1"))).toBeNull();
});

test("resource tables ignore a false legacy preference through navigation and reload", async ({ page }) => {
  await fixture(page, { legacyPreference: false });
  await ready(page);
  const connections = await calls(page, "cluster_connect");
  await navigate(page, "services"); await ready(page, "Services");
  expect(await calls(page, "cluster_connect")).toEqual(connections);
  await page.reload(); await ready(page, "Services");
  await expect(page.getByRole("switch", { name: "Resource tables", exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("iceon.resource-tables.v1"))).toBe("false");
});

for (const [failure, message, section] of [
  ["permissionDenied", "Kubernetes permissions", "pods"],
  ["resourceUnavailable", "resource is unavailable", "pod-security-policies"],
] as const) {
  test(`${failure} (including removed API/404) is an error, never an empty inventory`, async ({ page }) => {
    await fixture(page, { failure, section });
    await expect(page.getByRole("alert")).toContainText(message);
    await expect(page.getByRole("table")).toHaveCount(0);
    await expect(page.locator(".astryx-empty-state")).toHaveCount(0);
    expect((await calls(page)).at(-1)!.request.kind).toBe(section === "pods" ? "pods" : "podSecurityPolicies");
  });
}
for (const malformed of ["namespace", "session", "readId", "columns", "duplicate", "timestamp", "token"]) {
  test(`malformed ${malformed} DTO fails closed without a fake empty table`, async ({ page }) => {
    await fixture(page, { malformed });
    await expect(page.getByRole("alert")).toContainText("unsupported response");
    await expect(table(page)).toHaveCount(0);
    await expect(page.locator(".astryx-empty-state")).toHaveCount(0);
  });
}

test("automatic Pod LIST updates are quiet, do not overlap, and leave continuation pages alone", async ({ page }) => {
  await page.clock.install();
  await fixture(page); await ready(page);
  const initial = (await calls(page)).length;
  await controls(page, { hold: true });
  await page.clock.fastForward(10_000);
  await expect.poll(async () => (await calls(page)).length).toBe(initial + 1);
  await expect(page.getByRole("status", { name: "Loading pods…", exact: true })).toHaveCount(0);
  await expect(page.getByText(/Previously fetched|Fetched \d/)).toHaveCount(0);
  await page.clock.fastForward(60_000);
  expect((await calls(page)).length).toBe(initial + 1);
  await page.evaluate(() => { const dialog = document.createElement("section"); dialog.setAttribute("role", "dialog"); dialog.id = "held-confirmation"; dialog.textContent = "Confirmation"; document.body.append(dialog); });
  await controls(page, { hold: false, empty: true });
  await page.evaluate(() => (window as MockWindow).__releaseResources());
  await page.clock.fastForward(1_000);
  await expect(table(page)).toContainText("shared-name");
  await page.evaluate(() => document.querySelector("#held-confirmation")!.remove());
  await page.clock.fastForward(250);
  await expect(table(page).getByRole("heading", { name: "No pods found", exact: true })).toBeVisible();
  await controls(page, { empty: false });
  await page.clock.fastForward(10_000);
  await expect(table(page)).toContainText("shared-name");
  await clusterAction(page, "Next page");
  await expect(table(page)).toContainText("second-page-hit");
  const continued = (await calls(page)).length;
  await page.clock.fastForward(60_000);
  expect((await calls(page)).length).toBe(continued);
});

test("failed refresh preserves rows and reports stale data without reconnecting", async ({ page }) => {
  await fixture(page); await ready(page);
  const connections = await calls(page, "cluster_connect");
  await controls(page, { failure: "network" });
  await clusterAction(page, "Refresh pods");
  await expect(page.getByRole("alert")).toContainText("could not be reached");
  await ready(page);
  await expect(page.getByRole("status").filter({ hasText: /Updates unavailable.*stale/ })).toBeVisible();
  expect(await calls(page, "cluster_connect")).toEqual(connections);
  await controls(page, { failure: "" });
  await clusterAction(page, "Refresh pods");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText(/Updates unavailable.*stale/)).toHaveCount(0);
});

test("successful empty response is distinct from loading and failures", async ({ page }) => {
  await fixture(page, { hold: true, empty: true });
  await expect(page.getByRole("status", { name: "Loading pods…", exact: true })).toBeVisible();
  await expect(table(page)).toHaveCount(0);
  await page.evaluate(() => (window as MockWindow).__releaseResources());
  await expect(table(page).getByRole("heading", { name: "No pods found", exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

for (const width of [1440, 390]) for (const colorScheme of ["light", "dark"] as const) {
  test(`StatefulSets use the Astryx empty state at ${width} in ${colorScheme}`, async ({ page }, testInfo) => {
    await page.emulateMedia({ colorScheme });
    await fixture(page, { section: "stateful-sets", empty: true });
    await page.setViewportSize({ width, height: 900 });
    const empty = table(page, "Stateful Sets").locator(".astryx-empty-state");
    await expect(empty.getByRole("heading", { name: "No stateful sets found", level: 2, exact: true })).toBeVisible();
    await expect(empty).toHaveAttribute("role", "status");
    await expect(empty).toContainText("No stateful sets were found across all namespaces.");
    await expect(page.getByText("No stateful sets were returned.", { exact: true })).toHaveCount(0);
    await expect(empty.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();
    const box = (await empty.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(width);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    const viewport = page.locator("[data-iceon-table-viewport]");
    await viewport.evaluate(element => { element.scrollLeft = element.scrollWidth; });
    const scrolled = (await empty.boundingBox())!;
    expect(scrolled.x).toBeGreaterThanOrEqual(0);
    expect(scrolled.x + scrolled.width).toBeLessThanOrEqual(width);
    await page.screenshot({ path: testInfo.outputPath("statefulsets-empty.png") });
    await controls(page, { empty: false });
    await empty.getByRole("button", { name: "Refresh", exact: true }).focus();
    await page.keyboard.press("Enter");
    await ready(page, "Stateful Sets");
    await expect(empty).toHaveCount(0);
  });
}

test("StatefulSets filtered empty state clears filters with the keyboard and keeps namespace scope", async ({ page }) => {
  await fixture(page, { section: "stateful-sets" });
  await ready(page, "Stateful Sets");
  await selectNamespace(page, "team-a");
  const search = page.getByRole("textbox", { name: "Search Stateful Sets", exact: true });
  await search.fill("absent");
  const empty = table(page, "Stateful Sets").locator(".astryx-empty-state");
  await expect(empty.getByRole("heading", { name: "No stateful sets match your filters.", exact: true })).toBeVisible();
  await expect(empty).toContainText("Try changing your search or clearing a filter.");
  await empty.getByRole("button", { name: "Clear filters", exact: true }).focus();
  await page.keyboard.press("Enter");
  await ready(page, "Stateful Sets");
  await expect(search).toHaveValue("");
  expect((await calls(page)).at(-1)!.request).toMatchObject({ namespace: "team-a", continueToken: null });
  expect((await calls(page)).at(-1)!.request).not.toHaveProperty("query");
});

test("an empty snapshot does not mask a failed refresh with a successful empty state", async ({ page }) => {
  await fixture(page, { section: "stateful-sets", empty: true });
  const empty = table(page, "Stateful Sets").locator(".astryx-empty-state");
  await expect(empty).toBeVisible();
  await selectNamespace(page, "team-a");
  await expect(empty).toContainText("No stateful sets were found in namespace team-a.");
  await controls(page, { failure: "network" });
  await empty.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Could not load stateful sets");
  await expect(page.locator(".astryx-empty-state")).toHaveCount(0);
  await expect(table(page, "Stateful Sets")).toHaveCount(0);
  await controls(page, { failure: "" });
  await page.getByRole("alert").getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(empty).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

for (const section of ["secrets", "config-maps"]) {
  test(`${section} renders metadata only and the IPC parser discards data fields`, async ({ page }) => {
    const view = resourceViews.find((view) => view.section === section)!;
    await fixture(page, { section }); await ready(page, view.label);
    await expect(page.getByText("Metadata only. Data values and key names are not loaded.", { exact: true })).toHaveCount(0);
    await expect(table(page, view.label).getByRole("columnheader")).toHaveText(["Select all rows", "Name", "Namespace", "Age", "Status", "Actions"]);
    await expect(page.locator("body")).not.toContainText("PRIVATE_VALUE");
    await expect(page.locator("body")).not.toContainText("private-key-name");
    const parsed = await page.evaluate(async () => {
      const modulePath = "/src/app/cluster/api.ts";
      const api = await import(/* @vite-ignore */ modulePath);
      const request = (window as MockWindow).__resourceCalls.filter((call) => call.command === "cluster_resources" && call.request.kind !== "namespaces").at(-1)!.request;
      return (await api.loadResources({ ...request, readId: api.nextReadId() })).items;
    });
    expect(parsed).toHaveLength(2);
    for (const item of parsed) {
      expect(Object.keys(item).sort()).toEqual(["columns", "createdAt", "name", "namespace", "node", "resourceVersion", "status", "uid"]);
      expect(item.uid).toBe(`uid-${item.namespace}-${item.name}`);
      expect(item.resourceVersion).toBe("123");
      expect(item.columns).toEqual({}); expect(item.node).toBeNull();
      expect(JSON.stringify(item)).not.toMatch(/private-key|PRIVATE_VALUE|data|stringData/);
    }
  });
}

test("narrow tables scroll internally and search remains labelled and keyboard accessible", async ({ page }) => {
  await fixture(page, { section: "resource-quotas" }); await ready(page, "Resource Quotas");
  for (const width of [360, 640]) {
    await page.setViewportSize({ width, height: 740 });
    const search = page.getByRole("textbox", { name: "Search Resource Quotas", exact: true });
    await expect(search).toBeVisible(); await search.focus(); await expect(search).toBeFocused();
    await page.keyboard.type("TEAM-B");
    await expect(table(page, "Resource Quotas").getByRole("row")).toHaveCount(2);
    await search.fill("");
    const region = page.getByRole("group", { name: "Table", exact: true });
    await expect(region).toHaveAttribute("tabindex", "0");
    await region.focus(); await expect(region).toBeFocused(); await page.keyboard.press("End");
    expect(await region.evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true);
    const header = table(page, "Resource Quotas").getByRole("columnheader").first();
    await expect(header).toHaveCSS("position", "sticky");
    expect(await header.evaluate((element) => {
      const wrapper = element.closest('[role="group"]');
      return wrapper ? getComputedStyle(wrapper).overflow : "visible";
    })).toMatch(/auto|scroll/);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
  }
});
