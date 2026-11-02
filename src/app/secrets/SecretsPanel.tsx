import { useCallback, useEffect, useRef, useState } from "react";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { IconPlus } from "@tabler/icons-react";
import { TableActionOverlay, TableToolbar } from "../cluster/TableToolbar";
import { Button } from "@astryxdesign/core/Button";
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Selector } from "@astryxdesign/core/Selector";
import { Table } from "@astryxdesign/core/Table";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { hasNativeSecrets, secretsCancel, secretsDisconnect, secretsError, secretsList, secretsLogin, secretsProfileRemove, secretsProfileSave, secretsProfiles, secretsReveal, secretsResume, secretsSessionOpen, secretsClose, type SecretList, type SecretProfile, type SecretProfileInput, type SecretReadRequest, type SecretReveal, type SecretSession } from "./api";

const emptyProfile: SecretProfileInput = { provider: "vault", name: "", address: "https://", namespace: "", authMount: "userpass", kvMount: "secret", basePath: "" };
function message(error: unknown): string {
  return {
    invalidInput: "Check the connection fields and paths.", notFound: "The connection, saved login, or secret was not found.",
    storeUnavailable: "The connection store is unavailable.", credentialStoreUnavailable: "The system credential store is unavailable. Login without remembering, or retry later.",
    authFailed: "Authentication failed. Log in again or check the saved login.", permissionDenied: "The provider denied this request.",
    sessionExpired: "This session expired. Resume or log in again.", staleSession: "This session is no longer current. Close and reopen the connection.",
    cancelled: "The read was cancelled.", busy: "Another secrets operation is in progress. Retry when it finishes.", network: "The provider could not be reached.",
    tls: "The TLS connection was not trusted.", invalidResponse: "The secrets service returned an invalid response.", tooLarge: "The requested response exceeds the read limit.", unsupportedAuth: "This authentication method is not supported.",
  }[secretsError(error)];
}

export function SecretsPanel() {
  const [profiles, setProfiles] = useState<SecretProfile[]>([]);
  const [selected, setSelected] = useState<SecretProfile | null>(null);
  const [adding, setAdding] = useState(false);
  const addButton = useRef<HTMLButtonElement>(null);
  const wasAdding = useRef(false);
  useEffect(() => {
    if (wasAdding.current && !adding) addButton.current?.focus();
    wasAdding.current = adding;
  }, [adding]);
  const [form, setForm] = useState<SecretProfileInput>(emptyProfile);
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState<SecretProfile | null>(null);
  const alive = useRef(false);
  const refresh = useCallback(async () => {
    setBusy(true); setError(null);
    try { const values = await secretsProfiles(); if (alive.current) { setProfiles(values); setLoaded(true); } }
    catch (reason) { if (alive.current) setError(message(reason)); }
    finally { if (alive.current) setBusy(false); }
  }, []);
  useEffect(() => {
    alive.current = true;
    if (hasNativeSecrets()) void refresh();
    return () => { alive.current = false; };
  }, [refresh]);
  async function saveProfile() {
    setBusy(true); setError(null);
    try {
      const saved = await secretsProfileSave(form);
      if (alive.current) { setProfiles(values => [...values, saved]); setLoaded(true); setSelected(saved); setAdding(false); setForm(emptyProfile); }
    } catch (reason) { if (alive.current) setError(message(reason)); }
    finally { if (alive.current) setBusy(false); }
  }
  async function removeProfile() {
    if (!removing) return;
    setBusy(true); setError(null); setSelected(null);
    try {
      await secretsProfileRemove(removing.id);
      if (alive.current) { setProfiles(values => values.filter(item => item.id !== removing.id)); setRemoving(null); }
    } catch (reason) { if (alive.current) setError(message(reason)); }
    finally { if (alive.current) setBusy(false); }
  }
  const available = hasNativeSecrets();
  return <VStack gap={0} aria-label="Secrets management" style={{ position: "relative", flex: 1, minHeight: 0, minWidth: 0, overflow: "hidden" }}>
    <TableToolbar label="Saved connections">
      <Button label="Refresh connections" variant="ghost" size="sm" isDisabled={!available || busy} onClick={() => void refresh()} />
    </TableToolbar>
    <VStack gap={3} padding={4} style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "auto", paddingBottom: "calc(var(--spacing-10) * 2 + var(--spacing-4))" }}>
    {!available && <Text>Secrets management is available in the desktop app only. Browser preview cannot connect to a provider or read secrets.</Text>}
    {error && !adding && <Text role="alert">{error}</Text>}
    {busy && <Text role="status">Loading connections…</Text>}
    {loaded && !busy && profiles.length === 0 && <EmptyState
      title="No saved connections"
      description="Add a provider connection to browse secrets without storing credentials in the profile."
      actions={<Button label="Add connection" variant="secondary" onClick={() => { setRemoving(null); setForm(emptyProfile); setError(null); setAdding(true); }} />}
      headingLevel={2}
      style={{ minHeight: "calc(var(--spacing-10) * 6)", minWidth: 0, overflowWrap: "anywhere" }} />}
    {profiles.length > 0 && <Table data={profiles} idKey="id" density="compact" columns={[
      { key: "name", header: "Connection" }, { key: "provider", header: "Provider", renderCell: () => <Text>Vault KV v2</Text> }, { key: "address", header: "Endpoint" },
      { key: "actions", header: "Actions", renderCell: item => <HStack gap={1} wrap="wrap"><Button label={`Open ${item.name}`} variant="ghost" size="sm" isDisabled={busy} onClick={() => { setSelected(item); setAdding(false); setRemoving(null); }} /><Button label={`Remove ${item.name}`} variant="ghost" size="sm" isDisabled={busy} onClick={() => { setSelected(null); setAdding(false); setRemoving(item); }} /></HStack> },
    ]} />}
    {removing && <VStack gap={2} role="group" aria-label="Confirm connection removal"><Text>Remove {removing.name}? This removes its connection metadata and saved login. Secrets on the provider are not changed.</Text><HStack gap={2} wrap="wrap"><Button label="Remove connection" variant="primary" isDisabled={busy} onClick={() => void removeProfile()} /><Button label="Keep connection" variant="secondary" isDisabled={busy} onClick={() => setRemoving(null)} /></HStack></VStack>}
    {adding && <Dialog isOpen purpose="form" width="min(36rem, 100%)" onOpenChange={open => { if (!open && !busy) setAdding(false); }}><DialogHeader title="Add secret connection" />
    <VStack as="form" gap={2} onSubmit={event => { event.preventDefault(); if (!busy) void saveProfile(); }} aria-label="New secrets connection">
      <Text>Connection metadata is saved locally. Credentials are handled separately when you log in.</Text>
      {error && <Text role="alert">{error}</Text>}
      <Selector label="Provider" value={form.provider} options={[{ label: "Vault KV v2", value: "vault" }]} onChange={() => {}} />
      <TextInput label="Connection name" value={form.name} onChange={name => setForm(value => ({ ...value, name }))} isRequired isDisabled={busy} />
      <TextInput label="HTTPS endpoint" placeholder="https://vault.example.com" value={form.address} onChange={address => setForm(value => ({ ...value, address }))} isRequired isDisabled={busy} />
      <TextInput label="Namespace" isOptional value={form.namespace} onChange={namespace => setForm(value => ({ ...value, namespace }))} isDisabled={busy} />
      <HStack gap={2} wrap="wrap"><TextInput label="Username/password auth mount" value={form.authMount} onChange={authMount => setForm(value => ({ ...value, authMount }))} isDisabled={busy} /><TextInput label="KV v2 mount" value={form.kvMount} onChange={kvMount => setForm(value => ({ ...value, kvMount }))} isDisabled={busy} /></HStack>
      <TextInput label="Base path" isOptional value={form.basePath} onChange={basePath => setForm(value => ({ ...value, basePath }))} isDisabled={busy} />
      <HStack gap={2} wrap="wrap"><Button label="Save connection" type="submit" variant="primary" isDisabled={busy} /><Button label="Cancel connection" variant="ghost" isDisabled={busy} onClick={() => setAdding(false)} /></HStack>
    </VStack></Dialog>}
    {selected && <SecretConnection key={selected.id} profile={selected} onClose={() => setSelected(null)} />}
    </VStack>
    <TableActionOverlay label="Secrets actions"><IconButton ref={addButton} label="Add secret connection" tooltip="Add secret connection" icon={<Icon icon={IconPlus} />} variant="primary" isDisabled={!available || busy} onClick={() => { setRemoving(null); setForm(emptyProfile); setError(null); setAdding(true); }} /></TableActionOverlay>
  </VStack>;
}

function SecretConnection({ profile, onClose }: { profile: SecretProfile; onClose: () => void }) {
  const [session, setSession] = useState<SecretSession | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(false);
  const [path, setPath] = useState("");
  const [pathDraft, setPathDraft] = useState("");
  const [history, setHistory] = useState<string[]>([]);
  const [listing, setListing] = useState<SecretList | null>(null);
  const [revealed, setRevealed] = useState<SecretReveal | null>(null);
  const [busy, setBusy] = useState(true);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const sessionRef = useRef<SecretSession | null>(null);
  const pending = useRef<SecretReadRequest | null>(null);
  const sequence = useRef(0);
  const readSequence = useRef(0);
  const clearReads = useCallback(() => {
    readSequence.current += 1;
    const previous = pending.current; pending.current = null;
    if (previous) void secretsCancel(previous).catch(() => {});
    setRevealed(null); setReading(false);
  }, []);
  useEffect(() => {
    const generation = ++sequence.current;
    const ownerId = crypto.randomUUID();
    setBusy(true);
    void secretsSessionOpen(profile.id, ownerId).then(opened => {
      if (sequence.current !== generation) { void secretsClose({ profileId: opened.profileId, sessionId: opened.sessionId }).catch(() => {}); return; }
      sessionRef.current = opened; setSession(opened);
    }).catch(reason => { if (sequence.current === generation) setError(message(reason)); }).finally(() => { if (sequence.current === generation) setBusy(false); });
    return () => {
      sequence.current += 1; clearReads(); setPassword("");
      const current = sessionRef.current; sessionRef.current = null;
      if (current) {
        void secretsClose({ profileId: current.profileId, sessionId: current.sessionId }).catch(() => {});
      }
    };
  }, [profile.id, clearReads]);
  useEffect(() => {
    const hide = () => { clearReads(); setPassword(""); };
    const visibility = () => { if (document.hidden) hide(); };
    window.addEventListener("blur", hide); document.addEventListener("visibilitychange", visibility);
    return () => { window.removeEventListener("blur", hide); document.removeEventListener("visibilitychange", visibility); };
  }, [clearReads]);
  useEffect(() => {
    if (!revealed) return;
    const timer = window.setTimeout(() => setRevealed(null), Math.max(0, Math.min(30_000, revealed.expiresAt - Date.now())));
    return () => window.clearTimeout(timer);
  }, [revealed]);
  useEffect(() => {
    if (session?.status !== "connected" || session.expiresAt === null) return;
    const deadline = session.expiresAt;
    let timer: number | undefined;
    let cancelled = false;
    const expire = () => {
      if (cancelled) return;
      const remaining = deadline - Date.now();
      if (remaining > 0) {
        timer = window.setTimeout(expire, Math.min(remaining, 2_147_483_647));
        return;
      }
      clearReads(); setListing(null); setPassword("");
      setSession(current => current ? { ...current, status: "expired" } : null);
      if (sessionRef.current) sessionRef.current = { ...sessionRef.current, status: "expired" };
    };
    expire();
    return () => { cancelled = true; if (timer !== undefined) window.clearTimeout(timer); };
  }, [session, clearReads]);
  function readFailure(reason: unknown) {
    const code = secretsError(reason); setError(message(code));
    if (code === "sessionExpired" || code === "staleSession") {
      clearReads(); setListing(null);
      if (sessionRef.current) { sessionRef.current = { ...sessionRef.current, status: "expired" }; setSession(sessionRef.current); }
    }
  }
  async function browse(nextPath: string, track = true) {
    const current = sessionRef.current; if (!current || current.status !== "connected") return;
    clearReads(); setError(null); setNotice(null); setListing(null); setPath(nextPath); setPathDraft(nextPath);
    if (track && nextPath !== path) setHistory(values => [...values, path]);
    const id = ++readSequence.current, generation = sequence.current;
    const request = { profileId: current.profileId, sessionId: current.sessionId, readId: id, path: nextPath };
    pending.current = request; setReading(true);
    try {
      const result = await secretsList(request);
      if (sequence.current === generation && readSequence.current === id) setListing(result);
    } catch (reason) { if (sequence.current === generation && readSequence.current === id) readFailure(reason); }
    finally { if (sequence.current === generation && readSequence.current === id) { pending.current = null; setReading(false); } }
  }
  async function showSecret(name: string) {
    const current = sessionRef.current; if (!current || current.status !== "connected") return;
    clearReads(); setError(null); setNotice(null);
    const id = ++readSequence.current, generation = sequence.current;
    const request = { profileId: current.profileId, sessionId: current.sessionId, readId: id, path: path ? `${path}/${name}` : name };
    pending.current = request; setReading(true);
    const startedAt = Date.now();
    try {
      const result = await secretsReveal(request);
      if (sequence.current === generation && readSequence.current === id && document.visibilityState !== "hidden") {
        const expiresAt = Math.min(result.expiresAt, startedAt + 30_000);
        if (expiresAt > Date.now()) setRevealed({ ...result, expiresAt });
      }
    } catch (reason) { if (sequence.current === generation && readSequence.current === id) readFailure(reason); }
    finally { if (sequence.current === generation && readSequence.current === id) { pending.current = null; setReading(false); } }
  }
  async function authenticate(resume: boolean) {
    const current = sessionRef.current; if (!current || busy) return;
    const generation = sequence.current;
    const request = { profileId: current.profileId, sessionId: current.sessionId, username, password, remember };
    setPassword(""); setBusy(true); setError(null); setNotice(null); clearReads();
    if (resume) request.password = "";
    try {
      const authentication = resume ? secretsResume({ profileId: current.profileId, sessionId: current.sessionId }) : secretsLogin(request);
      request.password = "";
      const next = await authentication;
      if (sequence.current !== generation) return;
      sessionRef.current = next; setSession(next);
      if (next.status === "connected") await browse("");
    } catch (reason) { if (sequence.current === generation) readFailure(reason); }
    finally { request.password = ""; if (sequence.current === generation) setBusy(false); }
  }
  async function disconnect(forget: boolean) {
    const current = sessionRef.current; if (!current || busy) return;
    const generation = sequence.current;
    clearReads(); setPassword(""); setListing(null); setError(null); setBusy(true);
    const signedOut = { ...current, status: "signedOut" as const, expiresAt: null };
    sessionRef.current = signedOut; setSession(signedOut);
    try { await secretsDisconnect({ profileId: current.profileId, sessionId: current.sessionId, forget }); if (sequence.current === generation) setNotice(forget ? "Disconnected. Saved login forgotten." : "Disconnected. Any saved login is retained."); }
    catch (reason) { if (sequence.current === generation) setError(message(reason)); }
    finally { if (sequence.current === generation) { clearReads(); setPassword(""); setListing(null); setBusy(false); } }
  }
  const connected = session?.status === "connected";
  return <VStack gap={3} aria-label={`${profile.name} secrets`} style={{ minWidth: 0 }}>
    <HStack gap={2} wrap="wrap" hAlign="between"><Text weight="bold">{profile.name} · Vault KV v2</Text><Button label="Close connection" variant="ghost" size="sm" onClick={onClose} /></HStack>
    <Text>{profile.address} · Namespace: {profile.namespace || "default"}</Text>
    <Text>Auth mount: {profile.authMount} · KV mount: {profile.kvMount} · Base path: {profile.basePath || "/"}</Text>
    {busy && <Text role="status">Updating secrets session…</Text>}{error && <Text role="alert">{error}</Text>}{notice && <Text role="status">{notice}</Text>}
    {session && <Text>Session: {session.status === "signedOut" ? "Signed out" : session.status === "expired" ? "Expired" : "Connected"}. Credential store: {session.credentialStore === "unavailable" ? "Unavailable" : session.credentialStore === "macosKeychain" ? "macOS Keychain" : session.credentialStore === "windowsCredentialManager" ? "Windows Credential Manager" : "Linux Secret Service"}.</Text>}
    {session && !connected && <VStack as="form" gap={2} onSubmit={event => { event.preventDefault(); void authenticate(false); }} aria-label="Provider login">
      <TextInput label="Username" value={username} onChange={setUsername} autoComplete="off" isDisabled={busy} />
      <TextInput label="Password" type="password" value={password} onChange={setPassword} autoComplete="off" isDisabled={busy} />
      <CheckboxInput label="Remember login" description="Stores the Vault token in the system credential store, never your password." value={remember} onChange={setRemember} isDisabled={busy || session.credentialStore === "unavailable"} />
      <HStack gap={2} wrap="wrap"><Button label="Log in" type="submit" variant="primary" isDisabled={busy || !username || !password} /><Button label="Resume saved login" variant="secondary" isDisabled={busy || session.credentialStore === "unavailable"} onClick={() => void authenticate(true)} /></HStack>
    </VStack>}
    {session && <HStack gap={2} wrap="wrap"><Button label="Forget saved login" variant="ghost" size="sm" isDisabled={busy || session.credentialStore === "unavailable"} onClick={() => void disconnect(true)} />{connected && <Button label="Disconnect" variant="secondary" size="sm" isDisabled={busy} onClick={() => void disconnect(false)} />}</HStack>}
    {connected && <VStack gap={2} style={{ minWidth: 0 }}>
      <HStack gap={2} wrap="wrap"><Button label="Back" variant="ghost" size="sm" isDisabled={busy || history.length === 0} onClick={() => { const previous = history[history.length - 1]; if (previous !== undefined) { setHistory(values => values.slice(0, -1)); void browse(previous, false); } }} /><Button label="Up" variant="ghost" size="sm" isDisabled={busy || !path} onClick={() => void browse(path.split("/").slice(0, -1).join("/"))} /><Button label="Refresh folder" variant="ghost" size="sm" isDisabled={busy} onClick={() => void browse(path, false)} /></HStack>
      <HStack as="form" gap={2} vAlign="end" onSubmit={event => { event.preventDefault(); void browse(pathDraft); }}><TextInput label="Folder path relative to base path" value={pathDraft} onChange={value => { clearReads(); setPathDraft(value); }} width="100%" /><Button label="Go" type="submit" variant="secondary" isDisabled={busy} /></HStack>
      <Text>Current folder: /{path}</Text>
      {reading && <Text role="status">Reading provider…</Text>}
      {listing && <><Table data={listing.entries} idKey={entry => `${entry.directory}:${entry.name}`} density="compact" columns={[
        { key: "name", header: "Name" }, { key: "directory", header: "Kind", renderCell: entry => <Text>{entry.directory ? "Folder" : "Secret"}</Text> },
        { key: "actions", header: "Actions", renderCell: entry => <Button label={`${entry.directory ? "Open" : "Reveal"} ${entry.name}`} size="sm" variant="ghost" isDisabled={busy} onClick={() => entry.directory ? void browse(path ? `${path}/${entry.name}` : entry.name) : void showSecret(entry.name)} /> },
       ]} emptyState={<EmptyState isCompact title="This folder is empty" description="Refresh or choose another folder to continue browsing." actions={<Button label="Refresh folder" variant="secondary" size="sm" isDisabled={busy} onClick={() => void browse(path, false)} />} />} />{listing.partial && <Text role="status">The listing is partial. Not all entries are shown.</Text>}</>}
      {revealed && <VStack gap={2}><HStack gap={2} wrap="wrap" hAlign="between"><Text weight="bold">{revealed.path} · version {revealed.version}</Text><Button label="Hide values" variant="secondary" size="sm" onClick={clearReads} /></HStack><Text>Values hide after at most 30 seconds, on navigation, or when this window loses focus.</Text>{revealed.fields.length === 0 ? <Text>This secret has no fields.</Text> : <Table data={revealed.fields} idKey="name" density="compact" columns={[{ key: "name", header: "Field", renderCell: field => <Text style={{ overflowWrap: "anywhere" }}>{field.name}</Text> }, { key: "value", header: "Value", renderCell: field => <Text style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{field.value}</Text> }]} />}</VStack>}
    </VStack>}
  </VStack>;
}
