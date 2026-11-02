import { expect, test } from "@playwright/test";

test("secrets adapters validate mounts, metadata, profiles and every read scope without exposing provider errors", async ({ page }) => {
  test.skip(process.env.ICEON_TEST_PRODUCTION === "1", "Source-module adapter checks require Vite; UI integration also runs against production.");
  await page.goto("/");
  const failures = await page.evaluate(async () => {
    const modulePath = "/src/app/secrets/api.ts";
    const api = await import(/* @vite-ignore */ modulePath);
    const failures: string[] = [];
    const root = window as any;
    root.isTauri = true;
    let reply: any;
    const calls: any[] = [];
    root.__TAURI_INTERNALS__ = { invoke: async (command: string, args: unknown) => { calls.push({ command, args: structuredClone(args) }); return reply; } };
    const invalid = async (name: string, action: () => Promise<unknown>) => {
      try { await action(); failures.push(`${name}: accepted`); }
      catch (error) { if (error !== "invalidResponse") failures.push(`${name}: wrong error`); }
    };
    const scope = { profileId: "a".repeat(32), sessionId: "fixture-session" };
    const request = { ...scope, path: "folder/sample", mount: "development", readId: 7 };
    const profile = { id: scope.profileId, provider: "vault", name: "Fixture Vault", address: "https://vault.synthetic.invalid", namespace: "", authMount: "userpass", kvMount: "development", basePath: "", pinned: true };
    const session = { ...scope, credentialStore: "macosKeychain", status: "connected", expiresAt: Date.now() + 60_000 };
    for (const credentialStore of ["macosKeychain", "windowsCredentialManager", "linuxSecretService", "unavailable"]) {
      reply = { ...session, credentialStore };
      if ((await api.secretsResume(scope)).credentialStore !== credentialStore) failures.push("credential store enum");
    }
    for (const change of [{ sessionId: "wrong" }, { profileId: "b".repeat(32) }, { credentialStore: "macos_keychain" }, { expiresAt: "soon" }, { status: "unknown" }]) {
      reply = { ...session, ...change }; await invalid("session", () => api.secretsResume(scope));
    }
    reply = [profile];
    if (!(await api.secretsProfiles())[0].pinned) failures.push("profile pin lost");
    const { pinned: _pinned, ...legacyProfile } = profile;
    reply = [legacyProfile]; if ((await api.secretsProfiles())[0].pinned !== false) failures.push("legacy pin default");
    for (const change of [{ pinned: "true" }, { provider: "unknown" }, { address: "http://vault.synthetic.invalid" }, { kvMount: "../development" }, { id: "wrong" }]) {
      reply = [{ ...profile, ...change }]; await invalid("profile", () => api.secretsProfiles());
    }
    reply = [profile, profile]; await invalid("duplicate profiles", () => api.secretsProfiles());
    reply = [{ ...profile, token: "SYNTHETIC_TOKEN", password: "SYNTHETIC_PASSWORD" }];
    if (JSON.stringify(await api.secretsProfiles()).includes("SYNTHETIC_")) failures.push("profile projects unknown credentials");
    reply = profile; await api.secretsProfileUpdate(profile); await api.secretsProfilePin(profile.id, true);
    reply = { ...profile, id: "b".repeat(32) }; await invalid("update identity", () => api.secretsProfileUpdate(profile));
    reply = { ...profile, pinned: false }; await invalid("pin mismatch", () => api.secretsProfilePin(profile.id, true));
    const mountsRequest = { ...scope, path: "", readId: 8 };
    const mounts = { ...scope, readId: 8, mounts: [{ path: "development", type: "kv", version: 2, supported: true }, { path: "production", type: "kv", version: 2, supported: true }, { path: "pki", type: "pki", version: null, supported: false }], partial: false };
    reply = mounts; if ((await api.secretsMounts(mountsRequest)).mounts.length !== 3) failures.push("valid mounts rejected");
    for (const change of [{ profileId: "b".repeat(32) }, { sessionId: "wrong" }, { readId: 9 }, { partial: 0 }, { mounts: [{ path: "pki", type: "pki", version: null, supported: true }] }, { mounts: [{ path: "kv", type: "kv", version: 1, supported: true }] }, { mounts: [{ path: "kv", type: "kv", version: 2, supported: false }] }, { mounts: [{ path: "/bad/", type: "kv", version: 2, supported: true }] }]) {
      reply = { ...mounts, ...change }; await invalid("mounts", () => api.secretsMounts(mountsRequest));
    }
    const listing = { ...request, entries: [{ name: "sample", directory: false }, { name: "folder", directory: true }], partial: false };
    const metadata = { ...request, currentVersion: 11, createdAt: "2026-01-01T00:00:00Z", updatedAt: null, versions: [{ version: 11, createdAt: "2026-01-01T00:00:00Z", deletionTime: "", destroyed: false }], partial: true };
    const reveal = { ...request, fields: [{ name: "example", value: "SYNTHETIC" }], version: 11, expiresAt: Date.now() + 30_000 };
    for (const [name, valid, action] of [["listing", listing, api.secretsList], ["metadata", metadata, api.secretsMetadata], ["reveal", reveal, api.secretsReveal]] as const) {
      reply = valid; await action(request);
      reply = { ...valid, token: "SYNTHETIC_TOKEN", cache: { token: "SYNTHETIC_TOKEN" } };
      if (JSON.stringify(await action(request)).includes("SYNTHETIC_TOKEN")) failures.push(`${name} projects unknown tokens`);
      for (const change of [{ profileId: "b".repeat(32) }, { sessionId: "wrong" }, { readId: 9 }, { path: "folder/other" }, { mount: "production" }, { mount: undefined }, { path: "../sample" }]) {
        reply = { ...valid, ...change }; await invalid(`${name} scope`, () => action(request));
      }
    }
    for (const change of [{ partial: null }, { entries: [{ name: "example", directory: "false" }] }, { entries: [{ name: "nested/sample", directory: false }] }]) {
      reply = { ...listing, ...change }; await invalid("listing flags", () => api.secretsList(request));
    }
    for (const change of [{ currentVersion: 0 }, { currentVersion: 1.5 }, { partial: "true" }, { createdAt: 123 }, { versions: [{ version: 0, createdAt: "", deletionTime: "", destroyed: false }] }, { versions: [{ version: 11, createdAt: "", deletionTime: "", destroyed: "false" }] }]) {
      reply = { ...metadata, ...change }; await invalid("metadata fields", () => api.secretsMetadata(request));
    }
    for (const change of [{ fields: [{ name: "example", value: {} }] }, { version: 0 }, { expiresAt: Infinity }]) {
      reply = { ...reveal, ...change }; await invalid("reveal fields", () => api.secretsReveal(request));
    }
    const count = calls.length;
    for (const action of [() => api.secretsMounts({ ...mountsRequest, mount: "development" }), () => api.secretsMounts({ ...mountsRequest, path: "folder" }), () => api.secretsMetadata({ ...request, mount: undefined }), () => api.secretsMetadata({ ...request, path: "" }), () => api.secretsReveal({ ...request, path: "" }), () => api.secretsProfilePin(profile.id, "true"), () => api.secretsDisconnect({ ...scope, forget: "false" }), () => api.secretsSessionOpen("not-an-id", "fixture-owner"), () => api.secretsSessionOpen(profile.id, "bad/owner"), () => api.secretsProfileRemove("not-an-id"), () => api.secretsProfileUpdate({ ...profile, id: "not-an-id" }), () => api.secretsLogin({ ...scope, profileId: "not-an-id", username: "synthetic", password: "synthetic", remember: false }), () => api.secretsCancel({ ...request, readId: 0 }), () => api.secretsClose({ ...scope, sessionId: "" })]) await invalid("invalid request", action);
    if (calls.length !== count) failures.push("invalid request invoked IPC");
    root.__TAURI_INTERNALS__.invoke = async () => { throw "untrusted provider SYNTHETIC text"; };
    await invalid("untrusted error", () => api.secretsProfiles());
    if (api.secretsError({ message: "authFailed" }) !== "invalidResponse" || api.secretsMessage("SYNTHETIC").includes("SYNTHETIC")) failures.push("unsafe error mapping");
    root.isTauri = false;
    let invoked = false; root.__TAURI_INTERNALS__.invoke = async () => { invoked = true; };
    try { await api.secretsProfiles(); failures.push("browser accepted"); } catch (error) { if (error !== "storeUnavailable") failures.push("browser error"); }
    if (invoked || api.hasNativeSecrets()) failures.push("browser IPC available");
    return failures;
  });
  expect(failures).toEqual([]);
});

test("unprovided secrets context defaults to an unavailable store", async ({ page }) => {
  test.skip(process.env.ICEON_TEST_PRODUCTION === "1", "Source-module context contract requires Vite.");
  await page.goto("/");
  const result = await page.evaluate(async () => {
    const reactPath = "/node_modules/.vite/deps/react.js";
    const domPath = "/node_modules/.vite/deps/react-dom_client.js";
    const providerPath = "/src/app/secrets/SecretsProvider.tsx";
    const reactModule = await import(/* @vite-ignore */ reactPath);
    const React = reactModule.default ?? reactModule;
    const dom = await import(/* @vite-ignore */ domPath);
    const createRoot = dom.createRoot ?? dom.default.createRoot;
    const { useSecretsProfiles } = await import(/* @vite-ignore */ providerPath);
    let value: any;
    const host = document.createElement("section"); document.body.append(host);
    const root = createRoot(host);
    function Probe() { value = useSecretsProfiles(); return null; }
    root.render(React.createElement(Probe));
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    const errors = [];
    for (const operation of [() => value.save({}), () => value.pin("a".repeat(32), true), () => value.remove("a".repeat(32))]) {
      try { await operation(); errors.push("accepted"); } catch (error) { errors.push(error); }
    }
    const result = { available: value.available, profiles: value.profiles, loaded: value.loaded, loading: value.loading, error: value.error, errors };
    root.unmount(); host.remove(); return result;
  });
  expect(result).toEqual({ available: false, profiles: [], loaded: false, loading: false, error: null, errors: ["storeUnavailable", "storeUnavailable", "storeUnavailable"] });
});

test("strict collection policy rejects duplicate scopes and malformed timestamps", async ({ page }) => {
  test.skip(process.env.ICEON_TEST_PRODUCTION === "1", "Source-module adapter checks require Vite.");
  await page.goto("/");
  const failures = await page.evaluate(async () => {
    const modulePath = "/src/app/secrets/api.ts";
    const api = await import(/* @vite-ignore */ modulePath);
    const root = window as any; root.isTauri = true;
    let reply: any;
    root.__TAURI_INTERNALS__ = { invoke: async () => reply };
    const failures: string[] = [];
    const rejected = async (label: string, action: () => Promise<unknown>) => {
      try { await action(); failures.push(`${label}: accepted`); }
      catch (error) { if (error !== "invalidResponse") failures.push(`${label}: wrong error`); }
    };
    const request = { profileId: "a".repeat(32), sessionId: "synthetic-session", readId: 5, path: "sample", mount: "development" };
    const entry = { name: "sample", directory: false };
    reply = { ...request, entries: [entry, entry], partial: false };
    await rejected("duplicate list entry", () => api.secretsList(request));
    const mount = { path: "development", type: "kv", version: 2, supported: true };
    reply = { ...request, mounts: [mount, mount], partial: false };
    await rejected("duplicate engine path", () => api.secretsMounts({ ...request, path: "", mount: undefined }));
    const version = { version: 1, createdAt: "2026-01-01T00:00:00Z", deletionTime: "", destroyed: false };
    const metadata = { ...request, currentVersion: 1, createdAt: version.createdAt, updatedAt: null, versions: [version], partial: false };
    reply = { ...metadata, versions: [version, version] };
    await rejected("duplicate history version", () => api.secretsMetadata(request));
    for (const change of [{ createdAt: "not-a-date" }, { updatedAt: "2026-99-99T30:00:00Z" }, { versions: [{ ...version, createdAt: "tomorrow" }] }, { versions: [{ ...version, deletionTime: "not-a-date" }] }]) {
      reply = { ...metadata, ...change }; await rejected("invalid timestamp", () => api.secretsMetadata(request));
    }
    return failures;
  });
  expect(failures).toEqual([]);
});
