import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Table, type TablePlugin } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { AddButton } from "../components/AddButton";
import { ContentError } from "../components/ContentError";
import { ContentLoading } from "../components/ContentLoading";
import { TableActionOverlay, TableToolbar } from "../cluster/TableToolbar";
import { useRegisterPageRefresh } from "../cluster/WorkspaceRefresh";
import { ConnectionWorkspace } from "./ConnectionWorkspace";
import { ProfileForm } from "./ProfileForm";
import { useSecretsProfiles } from "./SecretsProvider";
import { secretsMessage, type SecretProfile } from "./api";

export function SecretsPanel() {
  const provider = useSecretsProfiles();
  const { connection } = useSearch({ strict: false }) as { connection?: string };
  const navigate = useNavigate();
  const [opened, setOpened] = useState<string[]>([]);
  const [form, setForm] = useState<SecretProfile | "new" | null>(null);
  const [removing, setRemoving] = useState<SecretProfile | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const dialogTrigger = useRef<HTMLElement | null>(null);
  const wasDialog = useRef(false);
  const [revisions, setRevisions] = useState<Record<string, number>>({});
  const selected = provider.profiles.find(item => item.id === connection);
  const liveIds = opened.filter(id => provider.profiles.some(profile => profile.id === id));
  const displayedIds = selected && !liveIds.includes(selected.id) ? [...liveIds, selected.id] : liveIds;
  useEffect(() => { if (selected) setOpened(current => current.includes(selected.id) ? current : [...current, selected.id]); }, [selected]);
  useEffect(() => {
    const dialogOpen = !!form || !!removing;
    if (wasDialog.current && !dialogOpen) requestAnimationFrame(() => (dialogTrigger.current?.isConnected ? dialogTrigger.current : addButton.current)?.focus());
    wasDialog.current = dialogOpen;
  }, [form, removing]);
  function go(id?: string) { void navigate({ to: "/secrets", search: id ? { connection: id } : {} }); }
  const activation: TablePlugin<SecretProfile> = {
    transformBodyRow: (props, profile) => ({ ...props, htmlProps: {
      ...props.htmlProps, tabIndex: 0, "aria-label": `Open connection ${profile.name}`, "aria-disabled": busy || provider.loading,
      onClick: event => { if (!busy && !provider.loading && !(event.target as Element).closest("button, a, input, select, textarea, [role='button']")) go(profile.id); },
      onKeyDown: event => { if (!busy && !provider.loading && event.target === event.currentTarget && !event.repeat && !event.nativeEvent.isComposing && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); go(profile.id); } },
    } }),
  };
  function close(id: string) {
    const remaining = displayedIds.filter(item => item !== id);
    setOpened(remaining);
    if (connection === id) go(remaining[remaining.length - 1]);
  }
  function openForm(profile: SecretProfile | "new") { dialogTrigger.current = document.activeElement as HTMLElement | null; setActionError(null); setForm(profile); }
  function confirmRemove(profile: SecretProfile) { dialogTrigger.current = document.activeElement as HTMLElement | null; setActionError(null); setRemoving(profile); }
  async function remove() {
    if (!removing || busy) return;
    const id = removing.id; setBusy(true); setActionError(null);
    try { await provider.remove(id); close(id); setRemoving(null); }
    catch (reason) { setActionError(secretsMessage(reason)); }
    finally { setBusy(false); }
  }
  async function pin(profile: SecretProfile) {
    if (busy || provider.loading) return;
    setBusy(true); setActionError(null);
    try { await provider.pin(profile.id, !profile.pinned); }
    catch (reason) { setActionError(secretsMessage(reason)); }
    finally { setBusy(false); }
  }
  useRegisterPageRefresh({ scope: "/secrets", label: "Refresh connections", enabled: provider.available && !provider.loading && !busy && !form && !removing, refresh: () => void provider.refresh() }, !connection);
  return <VStack gap={0} aria-label="Secrets management" style={{ position: "relative", flex: 1, minHeight: 0, minWidth: 0, overflow: "hidden" }}>
    <HStack gap={1} paddingInline={3} paddingBlock={2} wrap="wrap" role="group" aria-label="Secret connections" style={{ flexShrink: 0 }}>
      <Button label="Connections" variant={!connection ? "secondary" : "ghost"} aria-pressed={!connection} size="sm" onClick={() => go()} />
      {displayedIds.map(id => { const profile = provider.profiles.find(item => item.id === id)!; return <Button key={id} label={profile.name} variant={connection === id ? "secondary" : "ghost"} aria-pressed={connection === id} size="sm" onClick={() => go(id)} />; })}
    </HStack>
    {!provider.available ? <EmptyState headingLevel={2} title="Secrets requires the desktop app" description="Browser preview cannot connect to a secrets provider or read secrets." /> : <>
      {!connection && <>
        <TableToolbar label="Saved connections" />
        {actionError && !form && !removing && <Text role="alert" style={{ padding: "var(--spacing-3)", overflowWrap: "anywhere" }}>{actionError}</Text>}
        {provider.error && provider.profiles.length > 0 && <Text role="alert" style={{ padding: "var(--spacing-3)", overflowWrap: "anywhere" }}>{provider.error} Saved connections may be stale. Use Refresh in the toolbar to retry.</Text>}
        {provider.loading && <Text role="status" style={{ padding: "var(--spacing-3)" }}>Loading connections…</Text>}
        {provider.error && provider.profiles.length === 0 ? <ContentError title="Connections could not be loaded" message={`${provider.error} Use Refresh in the toolbar to retry.`} /> : !provider.loaded && provider.loading ? <ContentLoading label="Loading connections…" /> : <VStack gap={3} padding={4} style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "auto", paddingBottom: "calc(var(--spacing-10) * 2 + var(--spacing-4))" }}>
          {provider.loaded && provider.profiles.length === 0 && <EmptyState headingLevel={2} title="No saved connections" description="Add a provider connection, then sign in to browse its secret engines." />}
          {provider.profiles.length > 0 && <Table data={provider.profiles} idKey="id" density="compact" hasHover plugins={{ activation }} columns={[
            { key: "name", header: "Connection", renderCell: profile => <Button label={`Open ${profile.name}`} variant="ghost" size="sm" isDisabled={busy || provider.loading} onClick={() => go(profile.id)} /> },
            { key: "provider", header: "Provider", renderCell: () => <Text>Vault KV v2</Text> },
            { key: "address", header: "Endpoint", renderCell: profile => <Text style={{ overflowWrap: "anywhere" }}>{profile.address}</Text> },
            { key: "actions", header: "Actions", renderCell: profile => <HStack gap={1} wrap="wrap"><Button label={`Edit ${profile.name}`} variant="ghost" size="sm" isDisabled={busy || provider.loading} onClick={() => openForm(profile)} /><Button label={`${profile.pinned ? "Unpin" : "Pin"} ${profile.name}`} variant="ghost" size="sm" isDisabled={busy || provider.loading} onClick={() => void pin(profile)} /><Button label={`Remove ${profile.name}`} variant="ghost" size="sm" isDisabled={busy || provider.loading} onClick={() => confirmRemove(profile)} /></HStack> },
          ]} />}
        </VStack>}
      </>}
      {connection && !selected && (provider.loading || !provider.loaded && !provider.error ? <ContentLoading label="Loading connection…" /> : <EmptyState headingLevel={2} title={provider.error ? "Connection unavailable" : "Connection not found"} description={provider.error || "This saved connection no longer exists. Choose another connection."} actions={<Button label="View connections" variant="secondary" onClick={() => go()} />} />)}
      {displayedIds.map(id => { const profile = provider.profiles.find(item => item.id === id)!; return <ConnectionWorkspace key={`${id}:${revisions[id] ?? 0}`} profile={profile} active={connection === id && !form && !removing} onClose={() => close(id)} />; })}
    </>}
    {form && <ProfileForm profile={form === "new" ? undefined : form} save={provider.save} onClose={saved => {
      const editing = form !== "new"; setForm(null);
      if (saved) {
        if (editing) setRevisions(current => ({ ...current, [saved.id]: (current[saved.id] ?? 0) + 1 }));
        setOpened(current => current.includes(saved.id) ? current : [...current, saved.id]); go(saved.id);
      }
    }} />}
    {removing && <Dialog isOpen purpose="form" width="min(30rem, 100%)" onOpenChange={open => { if (!open && !busy) setRemoving(null); }}><DialogHeader title="Remove secret connection" /><VStack gap={4}><Text style={{ overflowWrap: "anywhere" }}>Remove {removing.name}? Its connection metadata and saved login will be removed. Secrets on the provider are not changed.</Text>{actionError && <Text role="alert">{actionError}</Text>}<HStack gap={2} wrap="wrap"><Button label="Remove connection" variant="primary" isDisabled={busy} onClick={() => void remove()} /><Button label="Keep connection" variant="secondary" isDisabled={busy} onClick={() => setRemoving(null)} /></HStack></VStack></Dialog>}
    <TableActionOverlay label="Secrets actions"><AddButton ref={addButton} label="Add secret connection" isDisabled={!provider.available || busy || provider.loading} onClick={() => openForm("new")} /></TableActionOverlay>
  </VStack>;
}
