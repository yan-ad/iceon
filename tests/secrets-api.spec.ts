import { expect, test } from "@playwright/test";

test("secrets adapters reject malformed and cross-session replies without leaking provider errors", async ({ page }) => {
  test.skip(process.env.ICEON_TEST_PRODUCTION === "1", "Source-module adapter checks run against Vite; UI tests also cover production.");
  await page.goto("/");
  const result = await page.evaluate(async () => {
    const modulePath = "/src/app/secrets/api.ts";
    const api = await import(/* @vite-ignore */ modulePath);
    const failures: string[] = [];
    const request = { profileId: "a".repeat(32), sessionId: "session", path: "sample", readId: 1 };
    const root = window as any;
    root.isTauri = true;
    let reply: any;
    root.__TAURI_INTERNALS__ = { invoke: async () => reply };
    const expectInvalid = async (name: string, run: () => Promise<unknown>) => {
      try { await run(); failures.push(`${name}: accepted`); }
      catch (error) { if (error !== "invalidResponse") failures.push(`${name}: wrong error`); }
    };
    const session = { profileId: request.profileId, sessionId: request.sessionId, credentialStore: "macosKeychain", status: "connected", expiresAt: Date.now() + 60_000 };
    reply = session;
    if ((await api.secretsResume(request)).credentialStore !== "macosKeychain") failures.push("camelCase credential store rejected");
    for (const change of [{ sessionId: "wrong" }, { profileId: "b".repeat(32) }, { credentialStore: "macos_keychain" }, { expiresAt: "soon" }, { status: "unknown" }]) {
      reply = { ...session, ...change };
      await expectInvalid("session", () => api.secretsResume(request));
    }
    const listing = { ...request, entries: [{ name: "example", directory: false }], partial: false };
    for (const change of [{ sessionId: "wrong" }, { profileId: "wrong" }, { readId: 2 }, { path: "wrong" }, { partial: null }, { entries: [{ name: "example", directory: "false" }] }]) {
      reply = { ...listing, ...change };
      await expectInvalid("listing", () => api.secretsList(request));
    }
    const reveal = { ...request, fields: [{ name: "example", value: "SYNTHETIC" }], version: 1, expiresAt: Date.now() + 30_000 };
    for (const change of [{ sessionId: "wrong" }, { readId: 2 }, { path: "wrong" }, { fields: [{ name: "example", value: {} }] }, { expiresAt: Infinity }]) {
      reply = { ...reveal, ...change };
      await expectInvalid("reveal", () => api.secretsReveal(request));
    }
    root.__TAURI_INTERNALS__.invoke = async () => { throw "untrusted provider text with SYNTHETIC"; };
    await expectInvalid("untrusted error", () => api.secretsProfiles());
    root.isTauri = false;
    let invoked = false;
    root.__TAURI_INTERNALS__.invoke = async () => { invoked = true; };
    try { await api.secretsProfiles(); failures.push("browser accepted"); }
    catch (error) { if (error !== "storeUnavailable") failures.push("browser wrong error"); }
    if (invoked) failures.push("browser invoked IPC");
    return failures;
  });
  expect(result).toEqual([]);
});
