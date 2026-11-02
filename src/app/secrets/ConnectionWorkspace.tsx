import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@astryxdesign/core/Button";
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { useRegisterPageRefresh } from "../cluster/WorkspaceRefresh";
import { ContentError } from "../components/ContentError";
import { SecretInspector } from "./SecretInspector";
import { secretsCancel, secretsClose, secretsDisconnect, secretsError, secretsList, secretsLogin, secretsMessage, secretsMetadata, secretsMounts, secretsResume, secretsReveal, secretsSessionOpen, type SecretList, type SecretMetadata, type SecretMounts, type SecretProfile, type SecretReadRequest, type SecretReveal, type SecretSession } from "./api";

export function ConnectionWorkspace({ profile, active, onClose }: { profile: SecretProfile; active: boolean; onClose: () => void }) {
  const [session, setSession] = useState<SecretSession | null>(null);
  const [busy, setBusy] = useState(true);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const [loginOpen, setLoginOpen] = useState(true);
  const [mounts, setMounts] = useState<SecretMounts | null>(null);
  const [mount, setMount] = useState<string | null>(null);
  const [path, setPath] = useState("");
  const [listing, setListing] = useState<SecretList | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [metadata, setMetadata] = useState<SecretMetadata | null>(null);
  const [revealed, setRevealed] = useState<SecretReveal | null>(null);
  const [narrow, setNarrow] = useState(false);
  const root = useRef<HTMLElement | null>(null);
  const loginHeading = useRef<HTMLParagraphElement>(null);
  const trigger = useRef<HTMLElement | null>(null);
  const generation = useRef(0);
  const readId = useRef(0);
  const pending = useRef<SecretReadRequest | null>(null);
  const sessionRef = useRef<SecretSession | null>(null);
  const activeRef = useRef(active); activeRef.current = active;
  const authBusy = useRef(false);
  const cancelReads = useCallback(() => {
    ++readId.current;
    const previous = pending.current; pending.current = null;
    if (previous) void secretsCancel(previous).catch(() => {});
    setReading(false); setRevealed(null);
  }, []);
  const hideValues = useCallback(() => { setRevealed(null); if (pending.current) cancelReads(); }, [cancelReads]);
  const clearData = useCallback(() => { cancelReads(); setMounts(null); setMount(null); setPath(""); setListing(null); setSelected(null); setMetadata(null); setDetailError(null); }, [cancelReads]);
  useEffect(() => {
    const life = ++generation.current;
    const owner = crypto.randomUUID();
    setBusy(true); authBusy.current = true;
    void secretsSessionOpen(profile.id, owner).then(opened => {
      if (generation.current !== life) { void secretsClose(opened).catch(() => {}); return; }
      sessionRef.current = opened; setSession(opened);
    }).catch(reason => { if (generation.current === life) setError(secretsMessage(reason)); }).finally(() => { if (generation.current === life) { authBusy.current = false; setBusy(false); } });
    return () => {
      ++generation.current; ++readId.current;
      const previous = pending.current; pending.current = null;
      if (previous) void secretsCancel(previous).catch(() => {});
      const current = sessionRef.current; sessionRef.current = null;
      if (current) void secretsClose(current).catch(() => {});
    };
  }, [profile.id]);
  useEffect(() => {
    if (!root.current) return;
    const observer = new ResizeObserver(entries => { if (entries[0]) setNarrow(entries[0].contentRect.width < 760); });
    observer.observe(root.current); return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!active) { cancelReads(); setPassword(""); setSubmitted(false); }
  }, [active, cancelReads]);
  useEffect(() => {
    const blur = () => { hideValues(); setPassword(""); };
    const visibility = () => { if (document.hidden) blur(); };
    window.addEventListener("blur", blur); document.addEventListener("visibilitychange", visibility);
    return () => { window.removeEventListener("blur", blur); document.removeEventListener("visibilitychange", visibility); };
  }, [hideValues]);
  useEffect(() => {
    if (!revealed) return;
    const timer = window.setTimeout(hideValues, Math.max(0, Math.min(30_000, revealed.expiresAt - Date.now())));
    return () => window.clearTimeout(timer);
  }, [revealed, hideValues]);
  useEffect(() => {
    if (session?.status !== "connected" || session.expiresAt === null) return;
    const deadline = session.expiresAt;
    let timer: number | undefined;
    const expire = () => {
      const remaining = deadline - Date.now();
      if (remaining > 0) { timer = window.setTimeout(expire, Math.min(remaining, 2_147_483_647)); return; }
      clearData(); setPassword(""); setLoginOpen(true);
      if (sessionRef.current) { sessionRef.current = { ...sessionRef.current, status: "expired" }; setSession(sessionRef.current); }
    };
    expire(); return () => { if (timer !== undefined) window.clearTimeout(timer); };
  }, [session, clearData]);
  function failure(reason: unknown, details = false) {
    (details ? setDetailError : setError)(secretsMessage(reason));
    if (["sessionExpired", "staleSession"].includes(secretsError(reason))) {
      clearData(); setPassword(""); setLoginOpen(true);
      if (sessionRef.current) { sessionRef.current = { ...sessionRef.current, status: "expired" }; setSession(sessionRef.current); }
    }
  }
  async function read<T>(scope: { path: string; mount?: string }, action: (request: SecretReadRequest) => Promise<T>, apply: (result: T) => void, details = false) {
    const current = sessionRef.current;
    if (!current || current.status !== "connected" || !activeRef.current) return;
    cancelReads();
    const id = ++readId.current, life = generation.current;
    const request = { profileId: current.profileId, sessionId: current.sessionId, readId: id, ...scope };
    pending.current = request; setReading(true); (details ? setDetailError : setError)(null);
    try {
      const result = await action(request);
      if (generation.current === life && readId.current === id && activeRef.current) apply(result);
    } catch (reason) { if (generation.current === life && readId.current === id && activeRef.current) failure(reason, details); }
    finally { if (generation.current === life && readId.current === id) { pending.current = null; setReading(false); } }
  }
  function loadMounts() {
    setSelected(null); setMetadata(null); setMount(null); setListing(null);
    return read({ path: "" }, secretsMounts, setMounts);
  }
  function browse(engine: string, folder: string) {
    setSelected(null); setMetadata(null); setDetailError(null); setListing(null); setMount(engine); setPath(folder);
    return read({ mount: engine, path: folder }, secretsList, setListing);
  }
  function inspect(secretPath: string) {
    trigger.current = document.activeElement as HTMLElement | null;
    setSelected(secretPath); setMetadata(null); setDetailError(null);
    if (mount) void read({ mount, path: secretPath }, secretsMetadata, setMetadata, true);
  }
  function closeInspector() { cancelReads(); setSelected(null); setMetadata(null); setDetailError(null); requestAnimationFrame(() => trigger.current?.focus()); }
  async function authenticate(resume: boolean) {
    const current = sessionRef.current;
    if (!current || authBusy.current || !activeRef.current) return;
    setSubmitted(true);
    if (!resume && (!username.trim() || !password)) {
      const input = root.current?.querySelector<HTMLInputElement>(!username.trim() ? 'input[name="username"]' : 'input[name="password"]'); input?.focus(); return;
    }
    const life = generation.current;
    authBusy.current = true; setBusy(true); setError(null); setNotice(null); clearData();
    const request = { profileId: current.profileId, sessionId: current.sessionId, username, password, remember };
    setPassword(""); setSubmitted(false);
    try {
      const authentication = resume ? secretsResume(current) : secretsLogin(request); request.password = "";
      const next = await authentication;
      if (generation.current !== life) return;
      sessionRef.current = next; setSession(next); setSubmitted(false);
      if (next.status === "connected") { setLoginOpen(false); if (activeRef.current) await loadMounts(); }
    } catch (reason) { if (generation.current === life) failure(reason); }
    finally { request.password = ""; if (generation.current === life) { authBusy.current = false; setBusy(false); } }
  }
  async function disconnect(forget: boolean) {
    const current = sessionRef.current;
    if (!current || authBusy.current) return;
    const life = generation.current; authBusy.current = true; setBusy(true); clearData(); setPassword(""); setError(null); setNotice(null);
    sessionRef.current = { ...current, status: "signedOut", expiresAt: null }; setSession(sessionRef.current); setLoginOpen(true);
    try { await secretsDisconnect({ ...current, forget }); if (generation.current === life) setNotice(forget ? "Saved login forgotten." : "Disconnected. Saved login retained."); }
    catch (reason) { if (generation.current === life) failure(reason); }
    finally { if (generation.current === life) { authBusy.current = false; setBusy(false); } }
  }
  const connected = session?.status === "connected";
  useEffect(() => { if (active && connected && !mounts && !mount && !authBusy.current && !error) void loadMounts(); }, [active, connected]);
  useEffect(() => { if (active && loginOpen && !connected) loginHeading.current?.focus(); }, [active, loginOpen, connected]);
  const aside = connected ? !!selected : loginOpen;
  useEffect(() => {
    if (!active || !aside) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing || document.querySelector('[role="dialog"]')) return;
      event.preventDefault(); if (connected) closeInspector(); else { setPassword(""); setLoginOpen(false); }
    };
    window.addEventListener("keydown", escape); return () => window.removeEventListener("keydown", escape);
  });
  useRegisterPageRefresh({ scope: "/secrets", label: selected ? "Refresh secret details" : mount ? "Refresh secrets" : "Refresh engines", enabled: !!connected && !busy && !reading, refresh: () => { if (selected && mount) { setMetadata(null); void read({ mount, path: selected }, secretsMetadata, setMetadata, true); } else if (mount) void browse(mount, path); else void loadMounts(); } }, active);
  async function reveal() {
    if (!mount || !selected || !metadata || detailError || busy || reading) return;
    const start = Date.now();
    await read({ mount, path: selected }, secretsReveal, result => {
      const expiresAt = Math.min(result.expiresAt, start + 30_000);
      if (!document.hidden && expiresAt > Date.now()) setRevealed({ ...result, expiresAt });
    }, true);
  }
  const store = session?.credentialStore === "macosKeychain" ? "macOS Keychain" : session?.credentialStore === "windowsCredentialManager" ? "Windows Credential Manager" : session?.credentialStore === "linuxSecretService" ? "Linux Secret Service" : "Unavailable";
  return <VStack ref={element => { root.current = element; }} gap={0} aria-label={`${profile.name} secrets`} aria-hidden={!active} style={{ display: active ? "flex" : "none", flex: 1, minWidth: 0, minHeight: 0, overflow: "hidden" }}>
    <HStack padding={3} gap={3} hAlign="between" wrap="wrap" style={{ flexShrink: 0 }}><VStack gap={1} style={{ minWidth: 0 }}><Text weight="semibold" style={{ overflowWrap: "anywhere" }}>{profile.name}</Text><Text type="supporting" style={{ overflowWrap: "anywhere" }}>{profile.address}</Text><Text type="supporting">{connected ? "Connected" : session?.status === "expired" ? "Expired" : "Signed out"} · Credential store: {store}</Text></VStack><HStack gap={2} wrap="wrap">{connected && <Button label="Disconnect" variant="secondary" size="sm" isDisabled={busy} onClick={() => void disconnect(false)} />}<Button label="Close connection" variant="ghost" size="sm" onClick={onClose} /></HStack></HStack>
    {error && <Text role="alert" style={{ padding: "var(--spacing-3)", overflowWrap: "anywhere" }}>{error}{connected ? " Use Refresh in the toolbar to retry." : ""}</Text>}
    {notice && <Text role="status" style={{ padding: "var(--spacing-3)" }}>{notice}</Text>}
    <HStack gap={0} style={{ flex: 1, minWidth: 0, minHeight: 0, overflow: "hidden" }}>
      <VStack gap={3} padding={4} aria-hidden={narrow && aside} style={{ flex: "1 1 0", width: 0, minWidth: 0, minHeight: 0, overflow: "auto", display: narrow && aside ? "none" : "flex", paddingBottom: "calc(var(--spacing-10) * 2 + var(--spacing-4))" }}>
        {!connected ? <EmptyState headingLevel={2} title={busy ? "Opening connection" : session?.status === "expired" ? "Session expired" : "Sign in to browse secrets"} description={busy ? "Preparing the Rust-owned provider session." : "Sign in or resume a saved login. Opening a connection does not authenticate it."} actions={!loginOpen && session ? <Button label="Show sign in" variant="secondary" onClick={() => setLoginOpen(true)} /> : undefined} /> : <>
          <HStack gap={2} wrap="wrap"><Button label="Engines" variant={mount ? "ghost" : "secondary"} isDisabled={busy || reading || !mount} size="sm" onClick={() => void loadMounts()} />{mount && <><Text style={{ overflowWrap: "anywhere" }}>{mount}/{path}</Text><Button label="Up" variant="ghost" size="sm" isDisabled={busy || reading || !path} onClick={() => void browse(mount, path.split("/").slice(0, -1).join("/"))} /></>}</HStack>
          {reading && <Text role="status">{selected ? "Reading secret details…" : mount ? "Loading secrets…" : "Loading engines…"}</Text>}
          {error && !(mount ? listing : mounts) && <ContentError title={mount ? "Secrets could not be loaded" : "Engines could not be loaded"} message={`${error} Use Refresh in the toolbar to retry.`} />}
          {!mount && mounts && <><Table data={mounts.mounts} idKey="path" density="compact" columns={[{ key: "path", header: "Engine", renderCell: engine => <Button label={`Open ${engine.path}`} variant="ghost" size="sm" isDisabled={!engine.supported || busy || reading} onClick={() => void browse(engine.path, "")} /> }, { key: "type", header: "Type", renderCell: engine => <Text>{engine.type}{engine.version ? ` v${engine.version}` : ""}</Text> }, { key: "supported", header: "Status", renderCell: engine => <Text>{engine.supported ? "Supported" : "Unsupported"}</Text> }]} emptyState={<EmptyState isCompact title="No visible engines" description="No secret engines were returned for this login." />} />{mounts.partial && <Text role="status">The engine listing is partial. Not all engines are shown.</Text>}</>}
          {mount && listing && <><Table data={listing.entries} idKey={entry => `${entry.directory}:${entry.name}`} density="compact" columns={[{ key: "name", header: "Name", renderCell: entry => <Button label={`Open ${entry.name}`} variant="ghost" size="sm" isDisabled={busy || reading} onClick={() => entry.directory ? void browse(mount, path ? `${path}/${entry.name}` : entry.name) : inspect(path ? `${path}/${entry.name}` : entry.name)} /> }, { key: "directory", header: "Kind", renderCell: entry => <Text>{entry.directory ? "Folder" : "Secret"}</Text> }]} emptyState={<EmptyState isCompact title="This folder is empty" description="Choose another folder or use Refresh in the toolbar to check again." />} />{listing.partial && <Text role="status">The secret listing is partial. Not all entries are shown.</Text>}</>}
        </>}
      </VStack>
      {aside && <VStack as="aside" role="complementary" aria-label={connected ? "Secret details" : `${profile.name} connection`} gap={0} style={{ width: narrow ? "100%" : "min(26rem, 100%)", maxWidth: "100%", minWidth: 0, flexShrink: 0, minHeight: 0, overflow: "auto", borderInlineStart: "var(--border-width) solid var(--color-border)", paddingBottom: "calc(var(--spacing-10) * 2 + var(--spacing-4))" }}>
        {connected && selected && mount ? <SecretInspector key={`${mount}:${selected}`} path={selected} mount={mount} metadata={metadata} revealed={revealed} busy={reading || busy} error={detailError} onReveal={() => void reveal()} onHide={hideValues} onClose={closeInspector} /> : <VStack padding={4} gap={4}>
          <HStack gap={2} hAlign="between" wrap="wrap"><Text ref={loginHeading} role="heading" aria-level={2} tabIndex={-1} weight="semibold">Sign in</Text><Button label="Close sign in" size="sm" variant="ghost" onClick={() => { setPassword(""); setLoginOpen(false); }} /></HStack>
          {busy && <Text role="status">Updating secrets session…</Text>}
          {session && <VStack as="form" aria-label="Provider login" gap={4} onSubmit={event => { event.preventDefault(); void authenticate(false); }}>
            <TextInput htmlName="username" label="Username" value={username} onChange={setUsername} autoComplete="username" isRequired isDisabled={busy} status={submitted && !username.trim() ? { type: "error", message: "Enter your username." } : undefined} statusVariant="detached" />
            <TextInput htmlName="password" label="Password" type="password" value={password} onChange={setPassword} autoComplete="current-password" isRequired isDisabled={busy} status={submitted && !password && !busy ? { type: "error", message: "Enter your password." } : undefined} statusVariant="detached" />
            <CheckboxInput label="Remember login" description="Stores only the Vault token in the system credential store, never your password." value={remember} onChange={setRemember} isDisabled={busy || session.credentialStore === "unavailable"} />
            <HStack gap={2} wrap="wrap"><Button label="Log in" type="submit" variant="primary" isDisabled={busy} /><Button label="Resume saved login" variant="secondary" isDisabled={busy || session.credentialStore === "unavailable"} onClick={() => void authenticate(true)} /></HStack>
            <Button label="Forget saved login" variant="ghost" size="sm" isDisabled={busy || session.credentialStore === "unavailable"} onClick={() => void disconnect(true)} />
          </VStack>}
        </VStack>}
      </VStack>}
    </HStack>
  </VStack>;
}
