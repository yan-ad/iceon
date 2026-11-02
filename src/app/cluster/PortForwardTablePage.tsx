import { memo, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { Button } from "@astryxdesign/core/Button";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Selector } from "@astryxdesign/core/Selector";
import { Table, proportional, type TableColumn } from "@astryxdesign/core/Table";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Text } from "@astryxdesign/core/Text";
import { ContentLoading } from "../components/ContentLoading";
import { ContentError } from "../components/ContentError";
import { TextInput } from "@astryxdesign/core/TextInput";
import { IconPlus, IconSearch, IconX } from "@tabler/icons-react";
import { TableActionOverlay, TableToolbar, tableViewport } from "./TableToolbar";
import type { SessionRequest } from "./api";
import { podDetail, podForwardList, podForwardStart, podForwardStop, podMessage, type PodForward } from "./podApi";
import { useRegisterWorkspaceRefresh } from "./WorkspaceRefresh";
import { useRowContextMenu, type RowMenuAction } from "./RowContextMenu";
import { NamespaceSelector } from "./NamespaceScope";
import { StatusBadge } from "./StatusBadge";
import { useResourceInspector } from "./ResourceInspector";

type PortForwardTablePageProps = SessionRequest;

const ForwardTable = memo(Table) as typeof Table;

const podPattern = /^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/;

function portValue(value: string, allowAuto: boolean) {
  if (!/^\d+$/.test(value)) return null;
  const port = Number(value);
  if (!Number.isSafeInteger(port) || port < (allowAuto ? 0 : 1) || port > 65535) return null;
  return port;
}

export function PortForwardTablePage({ clusterId, sessionId }: PortForwardTablePageProps) {
  return <PortForwardSession key={`${clusterId}:${sessionId}`} clusterId={clusterId} sessionId={sessionId} />;
}

function PortForwardSession({ clusterId, sessionId }: PortForwardTablePageProps) {
  const { openResource, openSummary } = useResourceInspector();
  const [forwards, setForwards] = useState<PodForward[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const pending = query !== deferredQuery;
  const [status, setStatus] = useState("");
  const [namespaceFilter, setNamespaceFilter] = useState("");
  const [namespace, setNamespace] = useState("");
  const [pod, setPod] = useState("");
  const [remotePort, setRemotePort] = useState("");
  const [localPort, setLocalPort] = useState("0");
  const [busy, setBusy] = useState(false);
  const alive = useRef(true);
  const loading = useRef(false);
  const [isLoading, setIsLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const acting = useRef(false);
  const generation = useRef(0);
  const request = useMemo(() => ({ clusterId, sessionId }), [clusterId, sessionId]);

  const load = useCallback(async (manual = false) => {
    if (loading.current) return;
    loading.current = true;
    setIsLoading(true);
    if (manual) setRefreshing(true);
    const owner = generation.current;
    try {
      const next = await podForwardList(request);
      if (alive.current && owner === generation.current) {
        setForwards(next);
        setError(null);
      }
    } catch (reason) {
      if (alive.current && owner === generation.current) setError(podMessage(reason));
    } finally {
      loading.current = false;
      if (alive.current && owner === generation.current) {
        setIsLoading(false);
        setRefreshing(false);
      }
    }
  }, [request]);

  useRegisterWorkspaceRefresh({ clusterId, section: "port-forwarding", enabled: !isLoading && !pending, refresh: () => void load(true) });

  useEffect(() => {
    alive.current = true;
    void load();
    const timer = window.setInterval(() => void load(), 2000);
    return () => {
      alive.current = false;
      generation.current += 1;
      window.clearInterval(timer);
    };
  }, [load]);

  const namespaces = useMemo(() => [...new Set((forwards ?? []).map((item) => item.namespace))].sort(), [forwards]);
  const statuses = useMemo(() => [...new Set((forwards ?? []).map((item) => item.status))].sort(), [forwards]);
  const searchable = useMemo(() => (forwards ?? []).map(item => ({ item, text:
    [item.id, item.namespace, item.name, item.remotePort, item.localPort, item.status].join(" ").toLocaleLowerCase(),
  })), [forwards]);
  const items = useMemo(() => {
    const needle = deferredQuery.trim().toLocaleLowerCase();
    return searchable.filter(({ item, text }) => (!needle || text.includes(needle)) && (!status || item.status === status) && (!namespaceFilter || item.namespace === namespaceFilter)).map(({ item }) => item);
  }, [searchable, namespaceFilter, deferredQuery, status]);

  const podValid = pod.length > 0 && pod.length <= 253 && podPattern.test(pod);
  const remote = portValue(remotePort, false);
  const local = portValue(localPort, true);
  const formValid = namespace.length > 0 && podValid && remote !== null && local !== null;

  async function start() {
    if (!formValid || remote === null || local === null || acting.current) return;
    acting.current = true;
    setBusy(true);
    setCreateError(null);
    try {
      const details = await podDetail({ clusterId, sessionId, namespace, name: pod });
      if (!alive.current) return;
      // The workspace session owns started forwards, including late start completions.
      await podForwardStart({ clusterId, sessionId, namespace, name: pod, uid: details.uid, remotePort: remote, localPort: local });
      if (alive.current) {
        setCreateOpen(false);
        setPod("");
        setRemotePort("");
        setLocalPort("0");
        await load();
      }
    } catch (reason) {
      if (alive.current) setCreateError(podMessage(reason));
    } finally {
      acting.current = false;
      if (alive.current) setBusy(false);
    }
  }

  async function stop(id: string) {
    if (acting.current) return;
    acting.current = true;
    setBusy(true);
    setActionError(null);
    try {
      await podForwardStop({ ...request, id });
      await load();
    } catch (reason) {
      if (alive.current) setActionError(podMessage(reason));
    } finally {
      acting.current = false;
      if (alive.current) setBusy(false);
    }
  }

  const inspectForward = useCallback((item: PodForward) => {
    openSummary({ title: item.id, status: item.status, facts: [
      { label: "Kind", value: "Port forward" },
      { label: "Namespace", value: item.namespace || "Unknown" },
      { label: "Pod", value: item.name || "Unknown" },
      { label: "Port", value: String(item.remotePort) },
      { label: "Local port", value: String(item.localPort) },
      { label: "Address", value: "127.0.0.1" },
      ...(item.error ? [{ label: "Error", value: podMessage(item.error) }] : []),
    ] });
  }, [openSummary]);

  const actions = useCallback((item: PodForward): RowMenuAction[] => [{ action: "details" }, { action: item.status === "stopped" || item.status === "error" ? "remove" : "stop", disabled: pending || busy || !!error }], [pending, busy, error]);
  const rowMenu = useRowContextMenu({ clusterId, sessionId, rows: items, scope: JSON.stringify([deferredQuery, status, namespaceFilter]),
    rowKey: item => item.id, rowLabel: item => item.id, target: item => ({ type: "forward", id: item.id }),
    actions,
    onAction: (item, action) => { if (action === "details") inspectForward(item); else void stop(item.id); },
  });
  const columns = useMemo<TableColumn<PodForward>[]>(() => [
    { key: "id", header: "Name", width: proportional(2), renderCell: (item) => <Button label={item.id} variant="ghost" size="sm" onClick={() => inspectForward(item)} /> },
    { key: "namespace", header: "Namespace", renderCell: (item) => <Button label={item.namespace || "Unknown"} variant="ghost" size="sm" isDisabled={!item.namespace} onClick={() => openResource({ kind: "namespaces", namespace: null, name: item.namespace })} /> },
    { key: "pod", header: "Pod", width: proportional(2), renderCell: (item) => <Button label={item.name || "Unknown"} variant="ghost" size="sm" isDisabled={!item.namespace || !item.name} onClick={() => openResource({ kind: "pods", namespace: item.namespace, name: item.name })} /> },
    { key: "remotePort", header: "Port", renderCell: (item) => <Text>{item.remotePort}</Text> },
    { key: "localPort", header: "Local Port", renderCell: (item) => <Text>{item.localPort}</Text> },
    { key: "address", header: "Address", renderCell: () => <Text>127.0.0.1</Text> },
    { key: "status", header: "Status", renderCell: (item) => <VStack gap={1}><StatusBadge status={item.status} variant={item.error ? "error" : undefined} />{item.error && <Text type="supporting">{podMessage(item.error)}</Text>}</VStack> },

  ], [inspectForward, openResource]);
  const plugins = useMemo(() => ({ viewport: tableViewport<PodForward>(), rowContext: rowMenu }), [rowMenu]);
  const hasFilters = !!(deferredQuery.trim() || status || namespaceFilter);
  const emptyState = useMemo(() => <EmptyState
    title={hasFilters ? "No forwards match the current filters." : "No port forwards"}
    description={hasFilters ? "Try changing your search or clearing a filter." : "Create a port forward to connect a local port to a Pod in this session."}
    actions={<Button label={hasFilters ? "Clear filters" : "Create forward"} variant="secondary"
      icon={<Icon icon={hasFilters ? IconX : IconPlus} />} isDisabled={busy || pending}
      onClick={() => {
        if (hasFilters) { setQuery(""); setStatus(""); setNamespaceFilter(""); }
        else { setCreateError(null); setCreateOpen(true); }
      }} />}
    headingLevel={2}
    style={{ minHeight: "calc(var(--spacing-10) * 6)", width: "calc(100cqi - var(--spacing-1))", minWidth: 0,
      position: "sticky", insetInlineStart: 0, overflowWrap: "anywhere" }} />, [hasFilters, busy, pending]);

  return <VStack gap={3} style={{ position: "relative", flex: 1, minWidth: 0, minHeight: 0, overflow: "hidden", containerType: "inline-size" }}>
      <TableToolbar label="Port Forwarding" count={forwards ? `${items.length} / ${forwards.length}` : undefined}>
        <TextInput label="Search forwards" isLabelHidden placeholder="Search forwards…" startIcon={IconSearch} value={query} onChange={setQuery} hasClear size="sm" width="min(18rem, 100%)" />
        <Selector label="Status" isLabelHidden value={status} onChange={setStatus} size="sm" options={[{ value: "", label: "All statuses" }, ...statuses.map((value) => ({ value, label: value }))]} />
        <Selector label="Namespace filter" isLabelHidden value={namespaceFilter} onChange={setNamespaceFilter} size="sm" options={[{ value: "", label: "All namespaces" }, ...namespaces.map((value) => ({ value, label: value }))]} />
      </TableToolbar>
    <ContentLoading label="Loading port forwards…" refreshing active={refreshing && !!forwards} />
    {error && <ContentError title="Port forwards unavailable" message={`${items.length > 0 ? "Retained forward states may be stale. " : ""}${error}`} retained={items.length > 0} onRetry={() => void load(true)} disabled={isLoading} />}
    {actionError && <Text role="alert">{actionError}</Text>}
    {pending && <Text role="status">Updating search results…</Text>}
    {!forwards && !error && <ContentLoading label="Loading port forwards…" />}
    {forwards && (!error || items.length > 0) && <ForwardTable aria-label="Port forwards" aria-busy={pending || refreshing} data={items} columns={columns} idKey="id" density="compact" hasHover textOverflow="truncate" plugins={plugins}
      emptyState={pending || refreshing ? false : emptyState} />}
    <TableActionOverlay label="Port forward actions"><IconButton label="Create port forward" tooltip="Create port forward" icon={<Icon icon={IconPlus} />} variant="primary" size="lg" elevation="med"
      style={{ pointerEvents: "auto", borderRadius: "var(--radius-container)" }}
      isDisabled={busy} onClick={() => { setCreateError(null); setCreateOpen(true); }} /></TableActionOverlay>
    <Dialog isOpen={createOpen} onOpenChange={open => { if (!busy) setCreateOpen(open); }} purpose={busy ? "required" : "form"} width="min(32rem, 100%)">
      <DialogHeader title="Create port forward" onOpenChange={busy ? undefined : setCreateOpen} />
      <VStack gap={3} padding={4} style={{ minHeight: 0, overflowY: "auto" }}>
        <NamespaceSelector value={namespace} onChange={setNamespace} label="Pod namespace" allowAll={false} isLabelHidden={false} />
        <TextInput label="Pod" value={pod} onChange={setPod} placeholder="Pod name" size="sm" status={pod && !podValid ? { type: "error", message: "Enter a valid Pod name." } : undefined} />
        <TextInput label="Port" value={remotePort} onChange={setRemotePort} placeholder="Remote port" size="sm" status={remotePort && remote === null ? { type: "error", message: "Use a port from 1 to 65535." } : undefined} />
        <TextInput label="Local port" value={localPort} onChange={setLocalPort} placeholder="0 for auto" size="sm" status={localPort && local === null ? { type: "error", message: "Use 0 or a port from 1 to 65535." } : undefined} />
        {createError && <Text role="alert">{createError}</Text>}
        <HStack gap={2} hAlign="end" wrap="wrap">
          <Button label="Cancel" size="sm" isDisabled={busy} onClick={() => setCreateOpen(false)} />
          <Button label={busy ? "Working…" : "Start forward"} size="sm" isDisabled={!formValid || busy} onClick={() => void start()} />
        </HStack>
      </VStack>
    </Dialog>
  </VStack>;
}

export default PortForwardTablePage;
