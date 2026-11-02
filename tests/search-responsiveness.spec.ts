import { expect, test, type Page } from "@playwright/test";

type Request = Record<string, any>;
type Call = { command: string; request: Request };
type SearchWindow = Window & { __searchCalls: Call[]; __releaseSearch: () => void };

async function fixture(page: Page, section: string, controlledClock = false) {
  page.on("pageerror", error => { throw error; });
  // Helm loads its own 500-row inventory; 500 extra clusters also mount hidden
  // cluster-switcher menus unrelated to the table under test.
  await page.addInitScript(({ catalogCount }) => {
    const calls: Call[] = [];
    const held: (() => void)[] = [];
    let lease = 0;
    const catalog = Array.from({ length: catalogCount }, (_, index) => ({ id: index ? `cluster-${index}` : "one", name: `cluster-${index}`, contextName: "fixture",
      source: "kubeconfig", labels: ["file=/fixtures/search.yaml"], color: "#476BA1", pinned: false, status: "disconnected" }));
    Object.assign(window, { isTauri: true, __searchCalls: calls, __releaseSearch: () => held.splice(0).forEach(resolve => resolve()),
      __TAURI_INTERNALS__: { invoke: async (command: string, args?: { request: Request }) => {
        const request = args?.request ?? {};
        calls.push({ command, request });
        if (command === "cluster_permissions_review") return { clusterId: request.clusterId, sessionId: request.sessionId, readId: request.readId,
          authorizationEpoch: String(request.sessionId), revision: 0, checkedAt: Date.now(), expiresAt: Date.now() + 30_000,
          items: (request.checks as unknown[]).map(check => ({ check, decision: "allowed", reason: null })) };
        if (command === "cluster_permissions_cancel" || command === "cluster_permissions_refresh") return null;
        if (command === "cluster_namespace_cache_get") return null;
        if (command === "cluster_namespace_cache_refresh") return { clusterId: request.clusterId, sessionId: request.sessionId,
          names: ["team-a", "team-b"], fetchedAt: Date.now(), expiresAt: Date.now() + 60_000, partial: false };
        if (command === "catalog_list") return { revision: 1, discoveryError: null, clusters: catalog };
        if (command === "cluster_session_open" || command === "cluster_connect") return { clusterId: request.clusterId,
          sessionId: request.sessionId ?? `lease-${++lease}`, contextName: "fixture", status: command === "cluster_connect" ? "connected" : "disconnected", apiVersion: "v1.35.8", error: null };
        if (command === "cluster_resources") {
          if (request.query === "held") await new Promise<void>(resolve => held.push(resolve));
          const usage = { used: null, allocatable: null, capacity: null };
          return { ...request, namespace: request.namespace ?? null, continueToken: request.query || request.kind === "namespaces" ? null : "next-page", truncated: false,
            metrics: null, fetchedAt: Date.now(), items: request.kind === "namespaces" ? ["team-a", "team-b"].map(name => ({ name, uid: name, namespace: null, resourceVersion: "1", createdAt: null, columns: {}, status: "Active", node: null })) :
              Array.from({ length: request.query ? 1 : 100 }, (_, index) => ({ name: request.query || `resource-${index}`, uid: `uid-${index}`,
                namespace: request.kind === "nodes" ? null : request.namespace || "team-a", resourceVersion: "1", createdAt: null, columns: {}, status: "Running",
                node: request.kind === "nodes" ? { roles: [], taints: 0, version: null, internalIps: [], schedulable: true, diskAllocatable: null,
                  metricsTimestamp: null, cpu: usage, memory: usage, conditions: [] } : null })) };
        }
        if (command === "helm_table") return { ...request, truncated: false, items: Array.from({ length: 500 }, (_, index) => ({
          id: `chart-${index}`, name: `repo/chart-${index}`, namespace: null, version: "1.0.0", appVersion: "1", description: "Cached chart", status: null, revision: null, updated: null })) };
        if (command === "cluster_pod_forward_list") return Array.from({ length: 8 }, (_, index) => ({ clusterId: request.clusterId, sessionId: request.sessionId, id: `forward-${index}`, namespace: "team-a", name: `pod-${index}`,
          uid: `uid-${index}`, remotePort: 8080, localPort: 20000 + index, status: "running", error: null }));
        if (["cluster_disconnect", "cluster_resources_cancel", "helm_cancel"].includes(command)) return null;
        throw `Unexpected fixture command: ${command}`;
      } },
    });
  }, { catalogCount: section === "helm-charts" ? 1 : 500 });
  await page.setViewportSize({ width: 1500, height: 900 });
  // React captures timer/performance APIs at module load. Install before it
  // loads so it cannot mix native callbacks with a subsequently paused clock.
  if (controlledClock) await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  await page.goto(section === "catalog" ? "/#/catalog/clusters" : `/#/cluster/one?section=${section}`);
  await expect(page.getByRole("button", { name: section === "catalog" ? "Refresh catalog" : `Refresh ${section}`, exact: true })).toBeEnabled({ timeout: 20_000 });
}
const calls = (page: Page, command: string, kind?: string) => page.evaluate(({ command, kind }) =>
  (window as SearchWindow).__searchCalls.filter(call => call.command === command && (!kind || call.request.kind === kind)), { command, kind });

test.describe("production input latency", () => {
  test.skip(process.env.ICEON_TEST_PRODUCTION !== "1", "Run against bun run build with ICEON_TEST_PRODUCTION=1.");

for (const [section, label, inputLabel, text] of [
  ["catalog", "Clusters", "Filter catalog", "cluster-49"],
  ["helm-charts", "Helm Charts", "Search Charts", "repo/chart-49"],
  ["pods", "Pods", "Search Pods", "resource-49"],
  ["nodes", "Nodes", "Search nodes", "resource-49"],
] as const) test(`${label}: real-clock typing paints each character promptly and retains caret/focus`, async ({ page }) => {
  test.setTimeout(90_000);
  await fixture(page, section);
  const grid = page.getByRole("table", { name: label, exact: true });
  await expect(grid).toBeVisible();
  const input = page.getByRole("textbox", { name: inputLabel, exact: true });
  await input.focus();
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await page.evaluate(() => {
    const delays: number[] = [];
    Object.assign(window, { __typingPaintDelays: delays });
    document.activeElement!.addEventListener("beforeinput", () => {
      const started = performance.now();
      requestAnimationFrame(() => delays.push(performance.now() - started));
    });
  });
  // Focus is already established; don't rescan a dense accessibility tree while timing input.
  await page.keyboard.type(text, { delay: 20 });
  // Exercise edits with both the dense retained inventory and narrowing results.
  for (let edit = 0; edit < 5; edit++) {
    await page.keyboard.press("Backspace");
    await page.keyboard.type(text.slice(-1), { delay: 20 });
  }
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const delays = await page.evaluate(() => (window as any).__typingPaintDelays as number[]);
  await expect(input).toHaveValue(text);
  await expect(input).toBeFocused();
  expect(delays.length).toBeGreaterThanOrEqual(text.length + 5);
  delays.sort((a, b) => a - b);
  const p95 = delays[Math.ceil(delays.length * .95) - 1];
  console.log(`${label} input-to-frame: p95=${p95.toFixed(1)}ms, max=${Math.max(...delays).toFixed(1)}ms`);
  expect(p95).toBeLessThan(100);
  expect(Math.max(...delays)).toBeLessThan(250);
  await input.evaluate(element => (element as HTMLInputElement).setSelectionRange(0, 0));
  await input.pressSequentially("z");
  await expect(input).toHaveValue(`z${text}`);
  expect(await input.evaluate(element => (element as HTMLInputElement).selectionStart)).toBe(1);
});

});

for (const [section, label, inputLabel, command, needle, count] of [
  ["catalog", "Clusters", "Filter catalog", "catalog_list", "cluster-499", 500],
  ["helm-charts", "Helm Charts", "Search Charts", "helm_table", "chart-499", 500],
  ["port-forwarding", "Port forwards", "Search forwards", "cluster_pod_forward_list", "forward-7", 8],
] as const) test(`${label}: keyboard search preserves text and filters ${count} loaded rows locally`, async ({ page }) => {
  await fixture(page, section);
  const grid = page.getByRole("table", { name: label, exact: true });
  await expect(grid.getByRole("row")).toHaveCount(count + 1, { timeout: 20_000 });
  const before = (await calls(page, command)).length;
  const input = page.getByRole("textbox", { name: inputLabel, exact: true });
  await input.pressSequentially(needle);
  await expect(input).toHaveValue(needle);
  await expect(grid.getByRole("row")).toHaveCount(2);
  await expect(grid).toContainText(needle);
  if (section !== "port-forwarding") expect((await calls(page, command)).length).toBe(before);
  await input.press("ControlOrMeta+A");
  await input.press("Backspace");
  await expect(input).toHaveValue("");
  await expect(grid.getByRole("row")).toHaveCount(count + 1, { timeout: 20_000 });
  if (section !== "port-forwarding") expect((await calls(page, command)).length).toBe(before);
});

for (const [section, kind, label] of [["pods", "pods", "Pods"], ["nodes", "nodes", "Nodes"]]) {
  test(`${label}: 100 retained rows coalesce keyboard input and reject late results`, async ({ page }) => {
    await fixture(page, section, true);
    const grid = page.getByRole("table", { name: label, exact: true });
    await expect(grid.getByRole("row")).toHaveCount(101);
    const before = (await calls(page, "cluster_resources", kind)).length;
    const input = page.getByRole("textbox", { name: `Search ${label}`, exact: false });
    await grid.getByRole("checkbox").nth(1).click();
    // Use the browser clock, with a transport guard before typing, not host time.
    await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1_000));
    await input.pressSequentially("abcdefghijklmnopqrst");
    await expect(input).toHaveValue("abcdefghijklmnopqrst");
    expect((await calls(page, "cluster_resources", kind)).length).toBe(before);
    await expect(page.getByRole("button", { name: "Delete selected resources", exact: true })).toHaveCount(0);
    if (kind === "nodes") await expect(page.getByRole("button", { name: "Next nodes page", exact: true })).toBeDisabled();
    else {
      await page.getByRole("button", { name: "Cluster actions", exact: true }).click();
      await expect(page.getByRole("menuitem", { name: "Next page", exact: true })).toBeDisabled();
      await page.keyboard.press("Escape");
    }
    await page.clock.fastForward(159);
    expect((await calls(page, "cluster_resources", kind)).length).toBe(before);
    // Check the debounce boundary without replaying every intervening frame.
    await page.clock.fastForward(1);
    await page.clock.resume();
    await expect(grid).toContainText("abcdefghijklmnopqrst");
    const searches = (await calls(page, "cluster_resources", kind)).slice(before);
    expect(searches.map(call => call.request.query)).toEqual(["abcdefghijklmnopqrst"]);
    expect(searches[0].request).toMatchObject({ clusterId: "one", kind, query: "abcdefghijklmnopqrst", continueToken: null });
    await input.fill("held");
    await expect.poll(async () => (await calls(page, "cluster_resources", kind)).at(-1)?.request.query).toBe("held");
    const held = (await calls(page, "cluster_resources", kind)).at(-1)!.request;
    await input.fill("final");
    await expect.poll(async () => (await calls(page, "cluster_resources_cancel")).some(call => call.request.readId === held.readId && call.request.sessionId === held.sessionId && call.request.clusterId === held.clusterId)).toBe(true);
    await expect(grid).toContainText("final");
    const final = (await calls(page, "cluster_resources", kind)).at(-1)!.request;
    expect(final).toMatchObject({ clusterId: held.clusterId, sessionId: held.sessionId, kind, query: "final", continueToken: null });
    expect(final.readId).not.toBe(held.readId);
    await page.evaluate(() => (window as SearchWindow).__releaseSearch());
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(grid).toContainText("final");
    await expect(grid).not.toContainText("held");
    await expect(input).toHaveValue("final");
    expect((await calls(page, "cluster_resources", kind)).slice(before).map(call => call.request.query))
      .toEqual(["abcdefghijklmnopqrst", "held", "final"]);
  });
}

test("namespace changes during the debounce cannot restore the old request scope", async ({ page }) => {
  await fixture(page, "pods", true);
  const grid = page.getByRole("table", { name: "Pods", exact: true });
  await expect(grid.getByRole("row")).toHaveCount(101);
  const input = page.getByRole("textbox", { name: "Search Pods", exact: true });
  const namespace = page.getByRole("button", { name: "Namespace", exact: true });
  await input.fill("held");
  await expect.poll(async () => (await calls(page, "cluster_resources", "pods")).at(-1)?.request.query).toBe("held");
  const held = (await calls(page, "cluster_resources", "pods")).at(-1)!.request;
  await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1_000));
  await input.fill("");
  await input.pressSequentially("scoped");
  await namespace.click();
  await page.getByRole("option", { name: "team-b", exact: true }).click();
  await expect.poll(async () => (await calls(page, "cluster_resources", "pods")).at(-1)?.request.namespace).toBe("team-b");
  await page.clock.fastForward(180);
  await page.clock.resume();
  await expect(input).toHaveValue("scoped");
  await expect(grid).toContainText("scoped");
  await expect.poll(async () => (await calls(page, "cluster_resources_cancel")).some(call =>
    call.request.clusterId === held.clusterId && call.request.sessionId === held.sessionId && call.request.readId === held.readId)).toBe(true);
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const searches = (await calls(page, "cluster_resources", "pods")).filter(call => call.request.query === "scoped");
  // The permission gate may commit after StrictMode's initial replay, so read ownership, not replay count, is the contract.
  expect(searches.length).toBeGreaterThanOrEqual(1);
  expect(new Set(searches.map(search => search.request.readId)).size).toBe(searches.length);
  for (const search of searches) {
    expect(search.request).toMatchObject({ clusterId: "one", sessionId: held.sessionId, kind: "pods", query: "scoped", namespace: "team-b", continueToken: null });
    expect(search.request.readId).not.toBe(held.readId);
  }
  await page.evaluate(() => (window as SearchWindow).__releaseSearch());
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  await expect(grid).toContainText("scoped");
  await expect(grid).not.toContainText("held");
  await expect(input).toHaveValue("scoped");
  expect((await calls(page, "cluster_resources", "pods")).filter(call => call.request.query === "scoped")).toEqual(searches);
});
