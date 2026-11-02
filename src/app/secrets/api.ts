import { invoke, isTauri } from "@tauri-apps/api/core";

export type SecretsProvider = "vault";
export type SecretsStatus = "signedOut" | "connected" | "expired";
export type CredentialStore = "macosKeychain" | "windowsCredentialManager" | "linuxSecretService" | "unavailable";
export type SecretsError = "invalidInput" | "notFound" | "storeUnavailable" | "credentialStoreUnavailable" | "authFailed" | "permissionDenied" | "sessionExpired" | "staleSession" | "cancelled" | "busy" | "network" | "tls" | "invalidResponse" | "tooLarge" | "unsupportedAuth";

export type SecretProfile = {
  id: string;
  provider: SecretsProvider;
  name: string;
  address: string;
  namespace: string;
  authMount: string;
  kvMount: string;
  basePath: string;
};
export type SecretProfileInput = Omit<SecretProfile, "id">;
export type SecretSession = { profileId: string; sessionId: string; status: SecretsStatus; expiresAt: number | null; credentialStore: CredentialStore };
export type SecretEntry = { name: string; directory: boolean };
export type SecretList = { profileId: string; sessionId: string; readId: number; path: string; entries: SecretEntry[]; partial: boolean };
export type SecretReveal = { profileId: string; sessionId: string; readId: number; path: string; fields: { name: string; value: string }[]; version: number; expiresAt: number };
export type SecretSessionRequest = { profileId: string; sessionId: string };
export type SecretReadRequest = SecretSessionRequest & { readId: number; path: string };

const errors = new Set<SecretsError>(["invalidInput", "notFound", "storeUnavailable", "credentialStoreUnavailable", "authFailed", "permissionDenied", "sessionExpired", "staleSession", "cancelled", "busy", "network", "tls", "invalidResponse", "tooLarge", "unsupportedAuth"]);
export function secretsError(reason: unknown): SecretsError {
  const value = typeof reason === "string" ? reason : "";
  return errors.has(value as SecretsError) ? value as SecretsError : "invalidResponse";
}
export function hasNativeSecrets(): boolean { return typeof window !== "undefined" && isTauri(); }
function native<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasNativeSecrets()) return Promise.reject("storeUnavailable");
  return invoke<T>(command, args).catch(reason => Promise.reject(secretsError(reason)));
}
function object(value: unknown): Record<string, unknown> | null { return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null; }
function stringValue(value: unknown): string | null { return typeof value === "string" ? value : null; }
function profile(value: unknown): SecretProfile | null {
  const v = object(value); if (!v) return null;
  const id = stringValue(v.id), provider = stringValue(v.provider), name = stringValue(v.name), address = stringValue(v.address), namespace = stringValue(v.namespace), authMount = stringValue(v.authMount), kvMount = stringValue(v.kvMount), basePath = stringValue(v.basePath);
  return id && /^[a-f0-9]{32}$/.test(id) && provider === "vault" && name !== null && address !== null && namespace !== null && authMount !== null && kvMount !== null && basePath !== null ? { id, provider: "vault", name, address, namespace, authMount, kvMount, basePath } : null;
}
function session(value: unknown): SecretSession {
  const v = object(value), profileId = stringValue(v?.profileId), sessionId = stringValue(v?.sessionId), status = stringValue(v?.status), store = stringValue(v?.credentialStore);
  if (!profileId || !/^[a-f0-9]{32}$/.test(profileId) || !sessionId || !["signedOut", "connected", "expired"].includes(status ?? "") || !["macosKeychain", "windowsCredentialManager", "linuxSecretService", "unavailable"].includes(store ?? "")) throw "invalidResponse";
  if (v?.expiresAt !== null && (typeof v?.expiresAt !== "number" || !Number.isFinite(v.expiresAt))) throw "invalidResponse";
  return { profileId, sessionId, status: status as SecretsStatus, expiresAt: typeof v?.expiresAt === "number" ? v.expiresAt : null, credentialStore: store as CredentialStore };
}
function list(value: unknown): SecretList {
  const v = object(value), profileId = stringValue(v?.profileId), sessionId = stringValue(v?.sessionId), path = stringValue(v?.path), entries = Array.isArray(v?.entries) ? v.entries : null;
  if (!profileId || !sessionId || path === null || !Number.isSafeInteger(v?.readId) || typeof v?.partial !== "boolean" || !entries) throw "invalidResponse";
  return { profileId, sessionId, readId: v.readId as number, path, partial: v?.partial === true, entries: entries.map(item => { const e = object(item); if (!e || typeof e.name !== "string" || typeof e.directory !== "boolean") throw "invalidResponse"; return { name: e.name, directory: e.directory }; }) };
}
function reveal(value: unknown): SecretReveal {
  const v = object(value), profileId = stringValue(v?.profileId), sessionId = stringValue(v?.sessionId), path = stringValue(v?.path), fields = Array.isArray(v?.fields) ? v.fields : null;
  if (!profileId || !sessionId || path === null || !fields || !Number.isSafeInteger(v?.readId) || !Number.isSafeInteger(v?.version) || typeof v?.expiresAt !== "number" || !Number.isFinite(v.expiresAt)) throw "invalidResponse";
  return { profileId, sessionId, readId: v.readId as number, path, version: v.version as number, expiresAt: v.expiresAt, fields: fields.map(item => { const f = object(item); if (!f || typeof f.name !== "string" || typeof f.value !== "string") throw "invalidResponse"; return { name: f.name, value: f.value }; }) };
}

export async function secretsProfiles(): Promise<SecretProfile[]> { const value = await native<unknown>("secrets_profiles"); if (!Array.isArray(value)) throw "invalidResponse"; return value.map(item => { const result = profile(item); if (!result) throw "invalidResponse"; return result; }); }
export async function secretsProfileSave(request: SecretProfileInput): Promise<SecretProfile> { const value = await native<unknown>("secrets_profile_save", { request }); const result = profile(value); if (!result) throw "invalidResponse"; return result; }
export async function secretsProfileRemove(profileId: string): Promise<void> { if (!/^[a-f0-9]{32}$/.test(profileId) || await native<unknown>("secrets_profile_remove", { profileId }) !== null) throw "invalidResponse"; }
export async function secretsSessionOpen(profileId: string, ownerId: string): Promise<SecretSession> { const result = session(await native("secrets_session_open", { profileId, ownerId })); if (result.profileId !== profileId) throw "invalidResponse"; return result; }
export async function secretsLogin(request: { profileId: string; sessionId: string; username: string; password: string; remember: boolean }): Promise<SecretSession> { const result = session(await native("secrets_login", { request: { ...request } })); checkScope(result, request); return result; }
export async function secretsResume(request: SecretSessionRequest): Promise<SecretSession> { const result = session(await native("secrets_resume", { request })); checkScope(result, request); return result; }
export async function secretsDisconnect(request: SecretSessionRequest & { forget: boolean }): Promise<void> { if (await native<unknown>("secrets_disconnect", { request }) !== null) throw "invalidResponse"; }
export async function secretsList(request: SecretReadRequest): Promise<SecretList> { const result = list(await native("secrets_list", { request })); checkRead(result, request); return result; }
export async function secretsReveal(request: SecretReadRequest): Promise<SecretReveal> { const result = reveal(await native("secrets_reveal", { request })); checkRead(result, request); return result; }
export async function secretsCancel(request: SecretReadRequest): Promise<void> { if (await native<unknown>("secrets_cancel", { request: { profileId: request.profileId, sessionId: request.sessionId, readId: request.readId } }) !== null) throw "invalidResponse"; }
export async function secretsClose(request: SecretSessionRequest): Promise<void> { if (await native<unknown>("secrets_close", { request }) !== null) throw "invalidResponse"; }

function checkScope(result: SecretSessionRequest, request: SecretSessionRequest): void {
  if (result.profileId !== request.profileId || result.sessionId !== request.sessionId) throw "invalidResponse";
}
function checkRead(result: SecretReadRequest, request: SecretReadRequest): void {
  checkScope(result, request);
  if (result.readId !== request.readId || result.path !== request.path) throw "invalidResponse";
}
