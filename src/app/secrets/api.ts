import { invoke, isTauri } from "@tauri-apps/api/core";

export type SecretsProvider = "vault";
export type SecretsStatus = "signedOut" | "connected" | "expired";
export type CredentialStore = "macosKeychain" | "windowsCredentialManager" | "linuxSecretService" | "unavailable";
export type SecretsError = "invalidInput" | "notFound" | "storeUnavailable" | "credentialStoreUnavailable" | "authFailed" | "permissionDenied" | "sessionExpired" | "staleSession" | "cancelled" | "busy" | "network" | "tls" | "invalidResponse" | "tooLarge" | "unsupportedAuth";

export type SecretProfile = {
  id: string; provider: SecretsProvider; name: string; address: string; namespace: string;
  authMount: string; kvMount: string; basePath: string; pinned: boolean;
};
export type SecretProfileInput = Omit<SecretProfile, "id" | "pinned">;
export type SecretSessionRequest = { profileId: string; sessionId: string };
export type SecretReadRequest = SecretSessionRequest & { readId: number; path: string; mount?: string };
export type SecretSession = SecretSessionRequest & { status: SecretsStatus; expiresAt: number | null; credentialStore: CredentialStore };
export type SecretEntry = { name: string; directory: boolean };
type ReadScope = SecretReadRequest & { mount: string };
export type SecretList = ReadScope & { entries: SecretEntry[]; partial: boolean };
export type SecretReveal = ReadScope & { fields: { name: string; value: string }[]; version: number; expiresAt: number };
export type SecretMount = { path: string; type: string; version: number | null; supported: boolean };
export type SecretMounts = SecretSessionRequest & { readId: number; mounts: SecretMount[]; partial: boolean };
export type SecretVersion = { version: number; createdAt: string; deletionTime: string; destroyed: boolean };
export type SecretMetadata = ReadScope & { currentVersion: number; createdAt: string | null; updatedAt: string | null; versions: SecretVersion[]; partial: boolean };

const messages: Record<SecretsError, string> = {
  invalidInput: "Check the connection fields and paths.", notFound: "The connection, saved login, or secret was not found.",
  storeUnavailable: "The connection store is unavailable.", credentialStoreUnavailable: "The system credential store is unavailable. Login without remembering, or retry later.",
  authFailed: "Authentication failed. Log in again or check the saved login.", permissionDenied: "The provider denied this request.",
  sessionExpired: "This session expired. Resume or log in again.", staleSession: "This session is no longer current. Close and reopen the connection.",
  cancelled: "The read was cancelled.", busy: "Another secrets operation is in progress. Retry when it finishes.", network: "The provider could not be reached.",
  tls: "The TLS connection was not trusted.", invalidResponse: "The secrets service returned an invalid response.", tooLarge: "The requested response exceeds the read limit.", unsupportedAuth: "This authentication method is not supported.",
};
const ID = /^[a-f0-9]{32}$/;
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;
const MAX_STRING = 1_048_576;
const encoder = new TextEncoder();
export function secretsError(reason: unknown): SecretsError {
  return typeof reason === "string" && Object.prototype.hasOwnProperty.call(messages, reason) ? reason as SecretsError : "invalidResponse";
}
function unique<T>(items: T[], key: (item: T) => string | number): T[] {
  if (new Set(items.map(key)).size !== items.length) fail();
  return items;
}
function timestamp(value: unknown, allowEmpty = false): string {
  const result = text(value, 128);
  if (allowEmpty && result === "") return result;
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(result) || !Number.isFinite(Date.parse(result))) fail();
  return result;
}
export function secretsMessage(reason: unknown): string { return messages[secretsError(reason)]; }
export function hasNativeSecrets(): boolean { return typeof window !== "undefined" && isTauri(); }
function fail(): never { throw "invalidResponse"; }
function text(value: unknown, max = MAX_STRING): string {
  if (typeof value !== "string" || value.length > max || encoder.encode(value).length > max) return fail();
  return value;
}
function object(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : fail();
}
function bool(value: unknown): boolean { return typeof value === "boolean" ? value : fail(); }
function id(value: unknown): string { const result = text(value, 32); return ID.test(result) ? result : fail(); }
function nonempty(value: unknown, max = 128): string { const result = text(value, max); return result ? result : fail(); }
function number(value: unknown, minimum = 0): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= minimum ? value : fail();
}
function array<T>(value: unknown, parse: (item: unknown) => T, max = 10_000): T[] {
  return Array.isArray(value) && value.length <= max ? value.map(parse) : fail();
}
function path(value: unknown, empty = false): string {
  const result = text(value, 4096);
  if (!result && empty) return result;
  if (result.split("/").some(part => !part || encoder.encode(part).length > 1024 || part === "." || part === ".." || CONTROL.test(part) || /[\\%]/.test(part))) fail();
  return result;
}
function profileInput(value: unknown): SecretProfileInput {
  const v = object(value), name = text(v.name, 128), address = text(v.address, 2048);
  const namespace = path(v.namespace, true), authMount = path(v.authMount), kvMount = path(v.kvMount), basePath = path(v.basePath, true);
  if (v.provider !== "vault" || !name.trim() || CONTROL.test(name) || !/^[\x00-\x7f]*$/.test(namespace) || CONTROL.test(address)) fail();
  let url: URL; try { url = new URL(address); } catch { return fail(); }
  if (!/^https:\/\/[^/?#\\\s]+\/?$/.test(address) || url.protocol !== "https:" || !url.hostname || url.username || url.password || url.search || url.hash || url.pathname !== "/") fail();
  return { provider: "vault", name, address, namespace, authMount, kvMount, basePath };
}
function profile(value: unknown): SecretProfile {
  const v = object(value);
  return { ...profileInput(v), id: id(v.id), pinned: v.pinned === undefined ? false : bool(v.pinned) };
}
function scope(value: unknown): SecretSessionRequest {
  const v = object(value);
  return { profileId: id(v.profileId), sessionId: nonempty(v.sessionId) };
}
function readRequest(value: unknown): SecretReadRequest {
  const v = object(value);
  return { ...scope(v), readId: number(v.readId, 1), path: path(v.path, true), ...(v.mount === undefined ? {} : { mount: path(v.mount) }) };
}
function readScope(value: unknown): ReadScope {
  const v = object(value);
  return { ...readRequest(v), mount: path(v.mount) };
}
function session(value: unknown): SecretSession {
  const v = object(value), status = v.status, credentialStore = v.credentialStore;
  if (status !== "signedOut" && status !== "connected" && status !== "expired") fail();
  if (credentialStore !== "macosKeychain" && credentialStore !== "windowsCredentialManager" && credentialStore !== "linuxSecretService" && credentialStore !== "unavailable") fail();
  return { ...scope(v), status, credentialStore, expiresAt: v.expiresAt === null ? null : number(v.expiresAt) };
}
function checkScope(result: SecretSessionRequest, request: SecretSessionRequest): void {
  if (result.profileId !== request.profileId || result.sessionId !== request.sessionId) fail();
}
function checkRead(result: ReadScope, request: SecretReadRequest): void {
  checkScope(result, request);
  if (result.readId !== request.readId || result.path !== request.path || (request.mount !== undefined && result.mount !== request.mount)) fail();
}
async function native(command: string, args?: Record<string, unknown>): Promise<unknown> {
  if (!hasNativeSecrets()) throw "storeUnavailable";
  try { return await invoke<unknown>(command, args); } catch (reason) { throw secretsError(reason); }
}
async function empty(command: string, args: Record<string, unknown>): Promise<void> { if (await native(command, args) !== null) fail(); }

export async function secretsProfiles(): Promise<SecretProfile[]> {
  const profiles = array(await native("secrets_profiles"), profile);
  if (new Set(profiles.map(item => item.id)).size !== profiles.length) fail();
  return profiles;
}
export async function secretsProfileSave(request: SecretProfileInput): Promise<SecretProfile> {
  return profile(await native("secrets_profile_save", { request: profileInput(request) }));
}
export async function secretsProfileUpdate(request: SecretProfileInput & { id: string }): Promise<SecretProfile>;
export async function secretsProfileUpdate(request: SecretProfileInput, profileId: string): Promise<SecretProfile>;
export async function secretsProfileUpdate(request: SecretProfileInput & { id?: string }, profileId = request.id): Promise<SecretProfile> {
  const selected = id(profileId);
  const result = profile(await native("secrets_profile_update", { request: { id: selected, ...profileInput(request) } }));
  if (result.id !== selected) fail();
  return result;
}
export async function secretsProfilePin(profileId: string, pinned: boolean): Promise<SecretProfile> {
  const result = profile(await native("secrets_profile_pin", { profileId: id(profileId), pinned: bool(pinned) }));
  if (result.id !== profileId || result.pinned !== pinned) fail();
  return result;
}
export async function secretsProfileRemove(profileId: string): Promise<void> { await empty("secrets_profile_remove", { profileId: id(profileId) }); }
export async function secretsSessionOpen(profileId: string, ownerId: string): Promise<SecretSession> {
  const owner = nonempty(ownerId, 64); if (!/^[a-zA-Z0-9-]+$/.test(owner)) fail();
  const result = session(await native("secrets_session_open", { profileId: id(profileId), ownerId: owner }));
  if (result.profileId !== profileId) fail();
  return result;
}
export async function secretsLogin(request: SecretSessionRequest & { username: string; password: string; remember: boolean }): Promise<SecretSession> {
  const safe = { ...scope(request), username: text(request.username), password: text(request.password), remember: bool(request.remember) };
  const result = session(await native("secrets_login", { request: safe })); checkScope(result, safe); return result;
}
export async function secretsResume(request: SecretSessionRequest): Promise<SecretSession> {
  const safe = scope(request), result = session(await native("secrets_resume", { request: safe })); checkScope(result, safe); return result;
}
export async function secretsDisconnect(request: SecretSessionRequest & { forget: boolean }): Promise<void> {
  await empty("secrets_disconnect", { request: { ...scope(request), forget: bool(request.forget) } });
}
export async function secretsList(request: SecretReadRequest): Promise<SecretList> {
  const safe = readRequest(request), v = object(await native("secrets_list", { request: safe }));
  const entries = unique(array(v.entries, item => { const e = object(item), name = path(e.name); if (name.includes("/")) fail(); return { name, directory: bool(e.directory) }; }, 1000), entry => `${entry.directory}:${entry.name}`);
  const result = { ...readScope(v), entries, partial: bool(v.partial) }; checkRead(result, safe); return result;
}
export async function secretsReveal(request: SecretReadRequest): Promise<SecretReveal> {
  const safe = readRequest(request); if (!safe.path) fail();
  const v = object(await native("secrets_reveal", { request: safe }));
  const fields = array(v.fields, item => { const f = object(item); return { name: text(f.name, 1024), value: text(f.value) }; });
  if (fields.reduce((size, field) => size + encoder.encode(field.name).length + encoder.encode(field.value).length, 0) > MAX_STRING) fail();
  const result = { ...readScope(v), fields, version: number(v.version, 1), expiresAt: number(v.expiresAt) }; checkRead(result, safe); return result;
}
export async function secretsMounts(request: SecretReadRequest): Promise<SecretMounts> {
  const safe = readRequest(request); if (safe.path !== "" || safe.mount !== undefined) fail();
  const v = object(await native("secrets_mounts", { request: safe }));
  const mounts = unique(array(v.mounts, item => {
    const m = object(item), type = nonempty(m.type, 128), version = m.version === null ? null : number(m.version, 1), supported = bool(m.supported);
    if (supported !== (type === "kv" && version === 2)) fail();
    return { path: path(m.path), type, version, supported };
  }, 1000), mount => mount.path);
  const result = { ...scope(v), readId: number(v.readId, 1), mounts, partial: bool(v.partial) };
  checkScope(result, safe); if (result.readId !== safe.readId) fail(); return result;
}
export async function secretsMetadata(request: SecretReadRequest): Promise<SecretMetadata> {
  const safe = readRequest(request); if (!safe.path || safe.mount === undefined) fail();
  const v = object(await native("secrets_metadata", { request: safe }));
  const versions = unique(array(v.versions, item => { const version = object(item); return { version: number(version.version, 1), createdAt: timestamp(version.createdAt), deletionTime: timestamp(version.deletionTime, true), destroyed: bool(version.destroyed) }; }, 200), version => version.version);
  const result = { ...readScope(v), currentVersion: number(v.currentVersion, 1), createdAt: v.createdAt === null ? null : timestamp(v.createdAt), updatedAt: v.updatedAt === null ? null : timestamp(v.updatedAt), versions, partial: bool(v.partial) };
  checkRead(result, safe); return result;
}
export async function secretsCancel(request: SecretReadRequest): Promise<void> {
  await empty("secrets_cancel", { request: { ...scope(request), readId: number(request.readId, 1) } });
}
export async function secretsClose(request: SecretSessionRequest): Promise<void> { await empty("secrets_close", { request: scope(request) }); }
