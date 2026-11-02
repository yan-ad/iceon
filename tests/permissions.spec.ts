import { installNamespaceSnapshotMock } from "./helpers/namespaceSnapshotMock";
import { expect, test } from "@playwright/test";

test("permission replies are exact, bounded and session scoped", async ({ page }) => {
  await installNamespaceSnapshotMock(page);
  await page.goto("/");
  expect(await page.evaluate(async () => {
    const path = "/src/app/cluster/permissionsApi.ts";
    const { parsePermissionReview, permissionCheck } = await import(/* @vite-ignore */ path);
    const now = Date.now();
    const request = { clusterId: "cluster", sessionId: "session", readId: 1, checks: [{ kind: "pods", action: "list", namespace: "team", name: null }] };
    const reply = { clusterId: request.clusterId, sessionId: request.sessionId, readId: request.readId, authorizationEpoch: "epoch", revision: 1, checkedAt: now, expiresAt: now + 30_000, items: [{ check: request.checks[0], decision: "allowed", reason: null }] };
    parsePermissionReview(reply, request, now);
    const invalid = [
      { ...reply, sessionId: "other" }, { ...reply, readId: 2 }, { ...reply, authorizationEpoch: "secret\nunsafe" },
      { ...reply, revision: NaN }, { ...reply, expiresAt: now + 600_000 },
      { ...reply, items: [] }, { ...reply, items: [reply.items[0], reply.items[0]] },
      { ...reply, items: [{ ...reply.items[0], check: { ...request.checks[0], namespace: "other" } }] },
      { ...reply, items: [{ ...reply.items[0], decision: "yes" }] },
      { ...reply, items: [{ ...reply.items[0], reason: "reviewDenied" }] },
      { ...reply, checks: request.checks }, { ...reply, expiresAt: now },
    ];
    return { rejected: invalid.map(value => { try { parsePermissionReview(value, request, now); return false; } catch { return true; } }),
      clusterScope: permissionCheck({ kind: "nodes", action: "list", namespace: "team" }).namespace };
  })).toEqual({ rejected: Array(12).fill(true), clusterScope: null });
});

for (const theme of ["light", "dark"] as const) {
  test(`${theme}: denied Nodes stays hidden during navigation and delayed permission renewal`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.clock.install();
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize({ width: 1200, height: 800 });
    await page.addInitScript(() => {
      const state = { hold: true, pending: 0, reviews: 0, opens: 0, nodes: "denied", releases: [] as (() => void)[] };
      Object.assign(window, { isTauri: true, __sidebarPermissions: state, __TAURI_INTERNALS__: {
        invoke: async (command: string, args: any) => {
          const request = args?.request;
          if (command === "catalog_list") return { revision: 1, discoveryError: null, webLinks: [], clusters: [
            { id: "one", name: "Development", contextName: "dev", source: "kubeconfig", labels: [], color: "#476BA1", status: "disconnected", pinned: false },
          ] };
          if (command === "cluster_session_open") { state.opens++; return { clusterId: "one", sessionId: "session", contextName: "dev", status: "disconnected", apiVersion: null, error: null }; }
          if (command === "cluster_connect") return { ...request, contextName: "dev", status: "connected", apiVersion: "v1.35.8", error: null };
          if (command === "cluster_permissions_review") {
            state.reviews++;
            if (state.hold) { state.pending++; await new Promise<void>(resolve => state.releases.push(resolve)); state.pending--; }
            return { clusterId: request.clusterId, sessionId: request.sessionId, readId: request.readId, authorizationEpoch: "epoch", revision: state.reviews, checkedAt: Date.now(), expiresAt: Date.now() + 30_000,
              items: request.checks.map((check: any) => ({ check, decision: check.kind === "nodes" ? state.nodes : "allowed", reason: check.kind === "nodes" && state.nodes === "denied" ? "reviewDenied" : null })) };
          }
          if (command === "cluster_resources") return { ...request, namespace: request.namespace ?? null, continueToken: null, metrics: null, items: [], truncated: false, fetchedAt: Date.now() };
          return null;
        },
      } });
    });
    await installNamespaceSnapshotMock(page);
    await page.goto("/#/cluster/one?section=deployments");
    const nav = page.locator("#cluster-sidebar");
    const nodes = nav.getByRole("link", { name: "Nodes", exact: true });
    await expect.poll(() => page.evaluate(() => (window as any).__sidebarPermissions.pending)).toBe(1);
    await expect(nodes).toHaveCount(0);
    await page.evaluate(() => {
      const state = (window as any).__sidebarPermissions;
      state.hold = false; state.releases.splice(0).forEach((release: () => void) => release());
    });
    await expect(page.getByRole("table", { name: "Deployments", exact: true })).toBeVisible();
    const opens = await page.evaluate(() => (window as any).__sidebarPermissions.opens);
    await expect(nodes).toHaveCount(0);
    await page.evaluate(() => {
      const state = (window as any).__sidebarPermissions;
      state.hold = true;
      state.appearances = 0;
      new MutationObserver(() => {
        if (document.querySelector('#cluster-sidebar a[aria-label="Nodes"]')) state.appearances++;
      }).observe(document.querySelector("#cluster-sidebar")!, { childList: true, subtree: true });
    });
    await page.clock.fastForward(30_001);
    await expect.poll(() => page.evaluate(() => (window as any).__sidebarPermissions.pending)).toBe(1);
    await nav.getByRole("link", { name: "Pods", exact: true }).click();
    await expect(page).toHaveURL(/section=pods$/);
    await nav.getByRole("link", { name: "Events", exact: true }).click();
    await expect(page).toHaveURL(/section=events$/);
    await expect(nodes).toHaveCount(0);
    await page.evaluate(() => {
      const state = (window as any).__sidebarPermissions;
      state.hold = false; state.releases.splice(0).forEach((release: () => void) => release());
    });
    await expect(page.getByRole("table", { name: "Events", exact: true })).toBeVisible();
    expect(await page.evaluate(() => ({ appearances: (window as any).__sidebarPermissions.appearances, opens: (window as any).__sidebarPermissions.opens }))).toEqual({ appearances: 0, opens });
    await page.evaluate(() => { (window as any).__sidebarPermissions.nodes = "allowed"; });
    await page.clock.fastForward(30_001);
    await expect(nodes).toBeVisible();
    expect(errors).toEqual([]);
  });
}

test("permission queue deduplicates subscribers and serializes batches above 32", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).isTauri = true;
    (window as any).__TAURI_INTERNALS__ = { invoke: async (command: string, args: any) => {
      if (command !== "cluster_permissions_review") return null;
      const request = args.request;
      const state = ((window as any).__permissionTest ??= { active: 0, peak: 0, batches: [] });
      state.active++; state.peak = Math.max(state.peak, state.active); state.batches.push(request.checks);
      await new Promise(resolve => setTimeout(resolve, 10)); state.active--;
      return { clusterId: request.clusterId, sessionId: request.sessionId, readId: request.readId, authorizationEpoch: "epoch", revision: state.batches.length, checkedAt: Date.now(), expiresAt: Date.now() + 30_000,
        items: request.checks.map((check: any) => ({ check, decision: "allowed", reason: null })) };
    } };
  });
  await installNamespaceSnapshotMock(page);
  await page.goto("/");
  const result = await page.evaluate(async () => {
    const path = "/src/app/cluster/PermissionProvider.tsx";
    const api = "/src/app/cluster/permissionsApi.ts";
    const { PermissionStore } = await import(/* @vite-ignore */ path);
    const { permissionCheck } = await import(/* @vite-ignore */ api);
    const store = new PermissionStore();
    const checks = Array.from({ length: 41 }, (_, index) => permissionCheck({ kind: "pods", action: "get", namespace: "team", name: `pod-${index}` }));
    const release = store.setContext({ clusterId: "cluster", sessionId: "session" });
    const first = store.request(checks), second = store.request(checks.slice(0, 2)); second();
    for (let i = 0; i < 100 && store.loading(checks); i++) await new Promise(resolve => setTimeout(resolve, 10));
    const decision = store.decision(checks[0]), lastDecision = store.decision(checks[40]); first(); release();
    return { ...(window as any).__permissionTest, decision, lastDecision, afterRelease: store.decision(checks[0]), otherNamespace: store.decision({ ...checks[0], namespace: "other" }) };
  });
  expect(result.peak).toBe(1);
  expect(result.batches.map((batch: unknown[]) => batch.length)).toEqual([32, 9]);
  expect(result.decision).toBe("allowed");
  expect(result.lastDecision).toBe("allowed");
  expect(result.afterRelease).toBe("unknown");
  expect(result.otherNamespace).toBe("unknown");
});

test("a denied cluster-scoped direct route never mounts the resource reader", async ({ page }) => {
  await page.addInitScript(() => {
    const reads: string[] = [];
    Object.assign(window, { isTauri: true, __permissionReads: reads, __TAURI_INTERNALS__: {
      invoke: async (command: string, args: any) => {
        const request = args?.request;
        if (command === "catalog_list") return { revision: 1, discoveryError: null, webLinks: [], clusters: [
          { id: "one", name: "Development", contextName: "dev", source: "kubeconfig", labels: [], color: "#476BA1", status: "disconnected", pinned: false },
        ] };
        if (command === "cluster_session_open") return { clusterId: "one", sessionId: "session", contextName: "dev", status: "disconnected", apiVersion: null, error: null };
        if (command === "cluster_connect") return { ...request, contextName: "dev", status: "connected", apiVersion: "v1.35.8", error: null };
        if (command === "cluster_permissions_review") return { clusterId: request.clusterId, sessionId: request.sessionId, readId: request.readId, authorizationEpoch: "epoch", revision: 1, checkedAt: Date.now(), expiresAt: Date.now() + 30_000,
          items: request.checks.map((check: any) => ({ check, decision: "denied", reason: "reviewDenied" })) };
        if (command === "cluster_resources") { reads.push(request.kind); throw "permissionDenied"; }
        return null;
      },
    } });
  });
  await installNamespaceSnapshotMock(page);
  await page.goto("/#/cluster/one?section=nodes");
  await expect(page.getByText("Access denied", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__permissionReads)).toEqual([]);
  await expect(page.getByRole("link", { name: "Nodes", exact: true })).toHaveCount(0);
});

test("permission cache expires and removing its last subscriber stops refresh timers", async ({ page }) => {
  await page.addInitScript(() => {
    Object.assign(window, { isTauri: true, __permissionReviews: 0, __TAURI_INTERNALS__: {
      invoke: async (command: string, args: any) => {
        if (command !== "cluster_permissions_review") return null;
        (window as any).__permissionReviews++;
        const { clusterId, sessionId, readId, checks } = args.request;
        return { clusterId, sessionId, readId, authorizationEpoch: "epoch", revision: (window as any).__permissionReviews,
          checkedAt: Date.now(), expiresAt: Date.now() + 200,
          items: checks.map((check: any) => ({ check, decision: "allowed", reason: null })) };
      },
    } });
  });
  await installNamespaceSnapshotMock(page);
  await page.goto("/");
  expect(await page.evaluate(async () => {
    const path = "/src/app/cluster/PermissionProvider.tsx";
    const { PermissionStore } = await import(/* @vite-ignore */ path);
    const store = new PermissionStore();
    const checks = [{ kind: "pods", action: "list", namespace: "team", name: null }] as const;
    const beforeContext = store.loading([...checks]);
    const release = store.setContext({ clusterId: "cluster", sessionId: "session" });
    const beforeSubscription = store.loading([...checks]), empty = store.loading([]);
    const unsubscribe = store.request([...checks]);
    for (let index = 0; index < 100 && (window as any).__permissionReviews < 2; index++) await new Promise(resolve => setTimeout(resolve, 10));
    const refreshed = (window as any).__permissionReviews >= 2;
    unsubscribe();
    const count = (window as any).__permissionReviews;
    await new Promise(resolve => setTimeout(resolve, 250));
    const expired = store.decision(checks[0]);
    const stopped = (window as any).__permissionReviews === count;
    release();
    return { beforeContext, beforeSubscription, empty, refreshed, expired, stopped };
  })).toEqual({ beforeContext: true, beforeSubscription: true, empty: false, refreshed: true, expired: "unknown", stopped: true });
});

test("replacing a permission scope cancels its exact read and rejects late replies", async ({ page }) => {
  await page.addInitScript(() => {
    Object.assign(window, { isTauri: true, __permissionRequests: [], __permissionCancels: [], __TAURI_INTERNALS__: {
      invoke: async (command: string, args: any) => {
        if (command === "cluster_permissions_cancel") { (window as any).__permissionCancels.push(args.request); return null; }
        if (command !== "cluster_permissions_review") return null;
        const request = args.request;
        (window as any).__permissionRequests.push(request);
        if (request.sessionId === "old") await new Promise(resolve => { (window as any).__finishOldPermission = resolve; });
        const { clusterId, sessionId, readId, checks } = request;
        return { clusterId, sessionId, readId, authorizationEpoch: sessionId, revision: 1, checkedAt: Date.now(), expiresAt: Date.now() + 30_000,
          items: checks.map((check: any) => ({ check, decision: sessionId === "old" ? "allowed" : "denied", reason: sessionId === "old" ? null : "reviewDenied" })) };
      },
    } });
  });
  await installNamespaceSnapshotMock(page);
  await page.goto("/");
  const result = await page.evaluate(async () => {
    const path = "/src/app/cluster/PermissionProvider.tsx";
    const { PermissionStore } = await import(/* @vite-ignore */ path);
    const store = new PermissionStore();
    const checks = [{ kind: "pods", action: "list", namespace: "team", name: null }] as const;
    const releaseOld = store.setContext({ clusterId: "cluster", sessionId: "old" });
    const unsubscribeOld = store.request([...checks]);
    await new Promise(resolve => setTimeout(resolve, 0));
    const releaseNew = store.setContext({ clusterId: "cluster", sessionId: "new" });
    const unsubscribeNew = store.request([...checks]);
    unsubscribeOld(); releaseOld();
    for (let index = 0; index < 100 && store.loading([...checks]); index++) await new Promise(resolve => setTimeout(resolve, 10));
    (window as any).__finishOldPermission();
    await new Promise(resolve => setTimeout(resolve, 0));
    const decision = store.decision(checks[0]), epoch = store.authorizationEpoch();
    unsubscribeNew(); releaseNew();
    return { decision, epoch, requests: (window as any).__permissionRequests, cancels: (window as any).__permissionCancels };
  });
  expect(result.decision).toBe("denied");
  expect(result.epoch).toBe("new");
  expect(result.requests.map((request: any) => request.sessionId)).toEqual(["old", "new"]);
  expect(result.cancels).toEqual([result.requests[0]]);
});

test("an epoch change invalidates earlier batches even when decisions change", async ({ page }) => {
  await page.addInitScript(() => {
    Object.assign(window, { isTauri: true, __permissionBatches: [], __TAURI_INTERNALS__: {
      invoke: async (command: string, args: any) => {
        if (command !== "cluster_permissions_review") return null;
        const { clusterId, sessionId, readId, checks } = args.request;
        const batches = (window as any).__permissionBatches;
        batches.push(checks.length);
        const old = batches.length === 1;
        return { clusterId, sessionId, readId, authorizationEpoch: old ? "old" : "new", revision: old ? 100 : batches.length - 1,
          checkedAt: Date.now(), expiresAt: Date.now() + 30_000,
          items: checks.map((check: any) => ({ check, decision: old ? "allowed" : "denied", reason: old ? null : "reviewDenied" })) };
      },
    } });
  });
  await installNamespaceSnapshotMock(page);
  await page.goto("/");
  expect(await page.evaluate(async () => {
    const path = "/src/app/cluster/PermissionProvider.tsx";
    const { PermissionStore } = await import(/* @vite-ignore */ path);
    const store = new PermissionStore();
    const checks = Array.from({ length: 41 }, (_, index) => ({ kind: "pods" as const, action: "get" as const, namespace: "team", name: `pod-${index}` }));
    const release = store.setContext({ clusterId: "cluster", sessionId: "session" });
    const unsubscribe = store.request(checks);
    for (let index = 0; index < 100 && store.loading(checks); index++) await new Promise(resolve => setTimeout(resolve, 10));
    const decisions = checks.map(check => store.decision(check)), epoch = store.authorizationEpoch();
    unsubscribe(); release();
    return { decisions, epoch, batches: (window as any).__permissionBatches };
  })).toEqual({ decisions: Array(41).fill("denied"), epoch: "new", batches: [32, 9, 32] });
});

test("subscriptions registered before a session survive context ownership and dispose independently", async ({ page }) => {
  await page.addInitScript(() => {
    Object.assign(window, { isTauri: true, __permissionRequests: [], __TAURI_INTERNALS__: {
      invoke: async (command: string, args: any) => {
        if (command !== "cluster_permissions_review") return null;
        const { clusterId, sessionId, readId, checks } = args.request;
        (window as any).__permissionRequests.push(args.request);
        return { clusterId, sessionId, readId, authorizationEpoch: sessionId, revision: (window as any).__permissionRequests.length,
          checkedAt: Date.now(), expiresAt: Date.now() + 30_000,
          items: checks.map((check: any) => ({ check, decision: "allowed", reason: null })) };
      },
    } });
  });
  await installNamespaceSnapshotMock(page);
  await page.goto("/");
  const result = await page.evaluate(async () => {
    const path = "/src/app/cluster/PermissionProvider.tsx";
    const { PermissionStore } = await import(/* @vite-ignore */ path);
    const store = new PermissionStore();
    const checks = Array.from({ length: 41 }, (_, index) => ({ kind: "pods" as const, action: "get" as const, namespace: "team", name: `pod-${index}` }));
    const unsubscribe = store.request(checks);
    await new Promise(resolve => setTimeout(resolve, 0));
    const beforeContext = (window as any).__permissionRequests.length;
    const releaseOld = store.setContext({ clusterId: "cluster", sessionId: "old" });
    for (let index = 0; index < 100 && store.loading(checks); index++) await new Promise(resolve => setTimeout(resolve, 10));
    const decisions = checks.map(check => store.decision(check));
    unsubscribe();
    const disposed = !store.hasSubscription(checks);
    const releaseNew = store.setContext({ clusterId: "cluster", sessionId: "new" });
    releaseOld();
    const next = [{ kind: "deployments" as const, action: "list" as const, namespace: "team", name: null }];
    const unsubscribeNext = store.request(next);
    for (let index = 0; index < 100 && store.loading(next); index++) await new Promise(resolve => setTimeout(resolve, 10));
    const nextDecision = store.decision(next[0]);
    unsubscribeNext(); releaseNew();
    await new Promise(resolve => setTimeout(resolve, 0));
    return { beforeContext, decisions, disposed, nextDecision, requests: (window as any).__permissionRequests,
      released: !store.hasSubscription(next), afterRelease: store.decision(next[0]) };
  });
  expect(result.beforeContext).toBe(0);
  expect(result.decisions).toEqual(Array(41).fill("allowed"));
  expect(result.disposed).toBe(true);
  expect(result.nextDecision).toBe("allowed");
  expect(result.requests.map((request: any) => [request.sessionId, request.checks.length])).toEqual([["old", 32], ["old", 9], ["new", 1]]);
  expect(result.requests[2].checks[0].kind).toBe("deployments");
  expect(result.released).toBe(true);
  expect(result.afterRelease).toBe("unknown");
});

test("routine permission renewal keeps the mounted deployment and namespace readers", async ({ page }) => {
  await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
  // Freeze before navigation: the running clock can overtake a timestamp sampled
  // through another browser round-trip on a busy CI worker.
  await page.clock.pauseAt(new Date("2026-01-01T00:01:00Z"));
  await page.addInitScript(() => {
    const state = { renewal: false, active: 0, completed: 0, reads: [] as string[] };
    Object.assign(window, { isTauri: true, __renewal: state, __TAURI_INTERNALS__: {
      invoke: async (command: string, args: any) => {
        const request = args?.request;
        if (command === "catalog_list") return { revision: 1, discoveryError: null, webLinks: [], clusters: [
          { id: "one", name: "Development", contextName: "dev", source: "kubeconfig", labels: [], color: "#476BA1", status: "disconnected", pinned: false },
        ] };
        if (command === "cluster_session_open") return { clusterId: "one", sessionId: "session", contextName: "dev", status: "disconnected", apiVersion: null, error: null };
        if (command === "cluster_connect") return { ...request, contextName: "dev", status: "connected", apiVersion: "v1.35.8", error: null };
        if (command === "cluster_permissions_review") {
          if (state.renewal) { state.active++; await new Promise(resolve => setTimeout(resolve, 30)); state.active--; state.completed++; }
          const { clusterId, sessionId, readId, checks } = request;
          return { clusterId, sessionId, readId, authorizationEpoch: "epoch", revision: readId,
            checkedAt: Date.now(), expiresAt: Date.now() + 1_000,
            items: checks.map((check: any) => ({ check, decision: "allowed", reason: null })) };
        }
        if (command === "cluster_resources") {
          state.reads.push(request.kind);
          return { ...request, namespace: request.namespace ?? null, continueToken: null, metrics: null, items: [], truncated: false, fetchedAt: Date.now() };
        }
        return null;
      },
    } });
  });
  await installNamespaceSnapshotMock(page);
  await page.goto("/#/cluster/one?section=deployments");
  await expect.poll(() => page.evaluate(() => (window as any).__renewal.reads.includes("deployments") && (window as any).__renewal.reads.includes("namespaces"))).toBe(true);
  const initial = await page.evaluate(() => {
    (window as any).__renewal.renewal = true;
    return [...(window as any).__renewal.reads];
  });
  await page.clock.fastForward(1_001);
  await expect.poll(() => page.evaluate(() => (window as any).__renewal.active)).toBe(1);
  expect(await page.evaluate(() => (window as any).__renewal.reads)).toEqual(initial);
  await expect(page.getByText("Checking access…", { exact: true })).toHaveCount(0);
  await page.clock.runFor(30);
  await expect.poll(() => page.evaluate(() => (window as any).__renewal.completed)).toBeGreaterThan(0);
  await page.clock.runFor(100);
  expect(await page.evaluate(() => (window as any).__renewal.reads)).toEqual(initial);
  await expect(page.getByText("Checking access…", { exact: true })).toHaveCount(0);
});
