import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useDebouncedSearch } from "./useDebouncedSearch";
import { ContentLoading } from "../components/ContentLoading";
import { ContentError } from "../components/ContentError";
import { Button } from "@astryxdesign/core/Button";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Selector } from "@astryxdesign/core/Selector";
import { Table, proportional, type TableColumn } from "@astryxdesign/core/Table";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { IconSearch, IconX, IconCircleCheck, IconAlertTriangle, IconHourglass, IconCircleDashed, IconRefresh } from "@tabler/icons-react";
import { Tooltip } from "@astryxdesign/core/Tooltip";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { cancelResources, connectionMessage, errorCode, loadResources, nextReadId,
  type ConnectionErrorCode, type ResourceItem, type ResourcePage, type SessionRequest } from "./api";
import type { ResourceView } from "./resourceViews";
import { useRegisterWorkspacePageActions, useRegisterWorkspaceRefresh } from "./WorkspaceRefresh";
import { usePodDock } from "./PodDock";
import type { PodMode } from "./podApi";
import { openPodSync, readPodSync, closePodSync, type PodSyncStatus } from "./podSyncApi";
import { HelmTablePage } from "./HelmTablePage";
import { PortForwardTablePage } from "./PortForwardTablePage";
import { ResourceActions } from "./ResourceActions";
import { useResourceSelection } from "./ResourceSelection";
import { resourceRowKey } from "./deleteApi";
import { NamespaceSelector, useNamespaceSelection } from "./NamespaceScope";
import { TableToolbar, tableViewport } from "./TableToolbar";
import { StatusBadge } from "./StatusBadge";
import { ResourceRelationLink, useResourceInspector } from "./ResourceInspector";
import type { ResourceReference } from "./detailApi";
import { useResourceRowMenu } from "./RowContextMenu";
import { sameInventory, useAutoRefresh } from "./useAutoRefresh";
import { usePermissionChecks, useResourcePermissions } from "./PermissionProvider";
import { ReadOnlyNotice } from "./ReadOnlyNotice";
import { ResourceAge } from "./ResourceAge";

function PodHealth({ item }: { item: ResourceItem }) {
  const state = item.columns.health;
  const warning = item.columns.warning;
  const message = state === "terminating" ? "Terminating: Pod shutdown is in progress"
    : state === "healthy" ? "Ready: all required containers and readiness gates are ready"
    : state === "completed" ? "Completed: Pod finished successfully"
    : state === "warning" ? `Pod needs attention: ${warning?.replace(/([a-z])([A-Z])/g, "$1 $2") ?? "not ready"}` : "Pod readiness unknown";
  const icon = state === "terminating" ? IconHourglass : state === "healthy" || state === "completed" ? IconCircleCheck : state === "warning" ? IconAlertTriangle : IconCircleDashed;
  return <Tooltip content={message}><HStack role="img" aria-label={message} vAlign="center"><Icon icon={icon} color={state === "healthy" || state === "completed" ? "success" : state === "warning" || state === "terminating" ? "warning" : "secondary"} /></HStack></Tooltip>;
}
const relationColumns = new Set(["node", "controlledBy", "storageClass", "volume", "claim", "roleRef", "policy", "target", "class", "involvedObject"]);
const stateColumns = new Set(["phase", "suspend", "globalDefault", "readOnly", "allowExpansion", "privileged", "hostNetwork"]);
const ResourceTable = memo(Table) as typeof Table;

function columns(view: ResourceView, openResource: (reference: ResourceReference) => void, openRelated: (reference: ResourceReference, column: string) => void, onNamespace: (namespace: string) => void, openPod?: (item: ResourceItem, mode: PodMode) => void): TableColumn<ResourceItem>[] {
  const reference = (item: ResourceItem): ResourceReference => ({ kind: view.kind!, namespace: item.namespace, name: item.name, uid: item.uid });
  return [
    { key: "name", header: "Name", width: proportional(2), renderCell: (item) => <Button label={item.name} variant="ghost" size="sm" onClick={() => openPod ? openPod(item, "details") : openResource(reference(item))} /> },
    ...(view.kind === "pods" ? [{ key: "health", header: "Health", renderCell: (item: ResourceItem) => <PodHealth item={item} /> }] : []),
    ...(view.namespaced ? [{ key: "namespace", header: "Namespace", renderCell: (item: ResourceItem) => item.namespace ? <Button label={item.namespace} aria-label={`Filter to namespace ${item.namespace}`} variant="ghost" size="sm" onClick={() => onNamespace(item.namespace!)} /> : <Text>N/A</Text> }] : []),
    ...view.columns.map((column) => ({ key: column.key, header: column.label,
      renderCell: (item: ResourceItem) => view.kind === "events" && column.key === "message" && item.columns.message
        ? <Tooltip content="Open full Event diagnostic text (bounded and redacted)" focusTrigger="always"><Button label={item.columns.message} style={{ color: (item.columns.type || item.status) === "Warning" ? "var(--color-text-red)" : undefined }} variant="ghost" size="sm" onClick={() => openResource(reference(item))} /></Tooltip>
        : view.kind === "events" && column.key === "lastSeen"
          ? item.columns.lastSeen ? <Tooltip content={item.columns.lastSeen}><HStack><ResourceAge timestamp={item.columns.lastSeen} fallback={item.columns.lastSeen} /></HStack></Tooltip> : <Text>N/A</Text>
        : relationColumns.has(column.key) && item.columns[column.key]
        ? column.key === "class" && view.kind === "ingresses" ? <ResourceRelationLink reference={{ kind: "ingressClasses", namespace: null, name: item.columns[column.key] }}>{item.columns[column.key]}</ResourceRelationLink>
          : <Button label={item.columns[column.key]} variant="ghost" size="sm" onClick={() => openRelated(reference(item), column.key)} />
        : stateColumns.has(column.key) ? <StatusBadge status={item.columns[column.key]} /> : <Text maxLines={1}>{item.columns[column.key] || "N/A"}</Text> })),
    { key: "age", header: "Age", renderCell: (item) => <ResourceAge timestamp={item.createdAt} /> },
    { key: "status", header: "Status", renderCell: (item) => <StatusBadge status={item.status} /> },

  ];
}

export function ResourceTablePage({ view, clusterId, sessionId }: SessionRequest & { view: ResourceView }) {
  const [selectedNamespace, setNamespace] = useNamespaceSelection();
  const [filters, setFilters] = useState({ query: "", status: "", group: "" });
  const namespace = view.namespaced ? selectedNamespace : "";
  return <ResourceTableScope key={JSON.stringify([clusterId, sessionId, view.section, namespace])} view={view} clusterId={clusterId} sessionId={sessionId} namespace={namespace} setNamespace={setNamespace} filters={filters} setFilters={setFilters} />;
}

type TableFilters = { query: string; status: string; group: string };
function ResourceTableScope({ view, clusterId, sessionId, namespace, setNamespace, filters, setFilters }: SessionRequest & { view: ResourceView; namespace: string; setNamespace: (namespace: string) => void;
  filters: TableFilters; setFilters: React.Dispatch<React.SetStateAction<TableFilters>> }) {
  const [tokens, setTokens] = useState<(string | null)[]>([null]);
  const [index, setIndex] = useState(0);
  const scope = useCallback((next: string) => { setNamespace(next); setTokens([null]); setIndex(0); }, [setNamespace]);
  const filter = (next: Partial<typeof filters>) => { setFilters(current => ({ ...current, ...next })); setTokens([null]); setIndex(0); };
  return <VStack gap={0} style={{ flex: 1, minWidth: 0, minHeight: 0, overflow: "hidden" }}>
    {!view.kind ? view.service === "port-forwarding" ? <PortForwardTablePage clusterId={clusterId} sessionId={sessionId} /> :
      <HelmTablePage clusterId={clusterId} sessionId={sessionId} view={view.service === "helm-charts" ? "charts" : "releases"} /> : <ResourceSnapshot key={`${clusterId}:${sessionId}:${view.section}:${namespace}`}
      clusterId={clusterId} sessionId={sessionId} view={view} namespace={namespace} token={tokens[index]} index={index}
      filters={filters} onFilter={filter} onNamespace={scope}
      previous={() => setIndex((value) => value - 1)} next={(token) => {
        setTokens((previous) => [...previous.slice(0, index + 1), token]); setIndex((value) => value + 1);
      }} />}
  </VStack>;
}

type SnapshotProps = SessionRequest & {
  view: ResourceView; namespace: string; token: string | null; index: number; previous: () => void; next: (token: string) => void;
  filters: { query: string; status: string; group: string };
  onFilter: (next: Partial<{ query: string; status: string; group: string }>) => void;
  onNamespace: (namespace: string) => void;
};
function ResourceSnapshot(props: SnapshotProps) {
  return props.view.kind === "pods" ? <PodSnapshot {...props} /> : <ResourceSnapshotContent {...props} />;
}
function PodSnapshot(props: SnapshotProps) {
  const { openPod } = usePodDock();
  const open = useCallback((item: ResourceItem, mode: PodMode) => {
    if (item.namespace) openPod({ namespace: item.namespace, name: item.name, ...(item.uid ? { uid: item.uid } : {}) }, mode);
  }, [openPod]);
  return <ResourceSnapshotContent {...props} openPod={open} />;
}
function ResourceSnapshotContent({ view, clusterId, sessionId, namespace, token, index, previous, next, openPod, filters, onFilter, onNamespace }: SnapshotProps & {
  openPod?: (item: ResourceItem, mode: PodMode) => void;
}) {
  const { openResource, openRelated } = useResourceInspector();
  const [revision, setRevision] = useState(0);
  const [page, setPage] = useState<ResourcePage | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<ConnectionErrorCode | null>(null);
  const { committed: query, pending } = useDebouncedSearch(filters.query);
  const { status, group } = filters;
  const [live, setLive] = useState(false);
  const [syncStatus, setSyncStatus] = useState<PodSyncStatus>("stopped");
  const listCheck = { kind: view.kind!, namespace: namespace || null, action: "list" as const };
  const permissions = usePermissionChecks([listCheck]);
  const resourcePermissions = useResourcePermissions(view.kind!, namespace || null);
  const accessWarning = !permissions.loading && permissions.decision(listCheck) === "unknown" || !resourcePermissions.loading && (["list", "create", "update", "patch", "delete"] as const).some(action => (namespace || action === "list" || !view.namespaced) && resourcePermissions.decision({ kind: view.kind!, namespace: namespace || null, action }) === "unknown")
    ? "Permissions could not be verified. Editing is unavailable."
    : resourcePermissions.readOnly ? "Read-only access. Your Kubernetes permissions allow viewing these resources, but not changing them." : undefined;
  const lastListDecision = useRef(permissions.decision(listCheck));
  if (!permissions.loading) lastListDecision.current = permissions.decision(listCheck);
  const listDecision = permissions.loading ? lastListDecision.current : permissions.decision(listCheck);
  const checking = permissions.loading && !permissions.observed(listCheck);
  const denied = listDecision === "denied";
  const readable = !checking && !denied;
  const watchCheck = { kind: view.kind!, namespace: namespace || null, action: "watch" as const };
  const watchPermissions = usePermissionChecks(view.kind === "pods" ? [watchCheck] : []);
  const watchAllowed = !permissions.loading && listDecision === "allowed" && !watchPermissions.loading && watchPermissions.decision(watchCheck) === "allowed";
  const filtered = !!(filters.query.trim() || filters.status || filters.group);
  const requestFiltered = !!(query.trim() || status || group);
  const syncing = live && watchAllowed && view.kind === "pods" && token === null && !filtered && !pending && !requestFiltered;
  const auto = useAutoRefresh({ scope: JSON.stringify([clusterId, sessionId, view.kind, namespace, token, query, status, group]), interval: 10_000,
    busy: loading, paused: !readable || pending || syncing || index > 0, refresh: () => setRevision(value => value + 1) });
  useRegisterWorkspacePageActions({ clusterId, section: view.section,
    previous: { enabled: readable && index > 0 && !loading && !pending, run: previous },
    next: { enabled: readable && !pending && !syncing && !!page?.continueToken && !loading && !error, run: () => { if (page?.continueToken) next(page.continueToken); } },
    live: view.kind === "pods" ? { enabled: watchAllowed && token === null && !filtered && !pending, checked: live && watchAllowed, run: () => { if (permissions.decision(listCheck) === "allowed" && watchPermissions.decision(watchCheck) === "allowed") setLive(value => !value); } } : undefined,
  });
  useRegisterWorkspaceRefresh({ clusterId, section: view.section, enabled: denied || readable && !loading && !pending, refresh: () => { if (denied) permissions.retry(); else setRevision((value) => value + 1); } });
  useEffect(() => {
    if (!view.kind || !readable || syncing || pending) return;
    let disposed = false;
    const request = { clusterId, sessionId, kind: view.kind, readId: nextReadId(), namespace: namespace || null, continueToken: token,
      ...(requestFiltered ? { query: query.trim() || null, status: status || null, group: group || null } : {}) };
    setLoading(true);
    if (!auto.background.current) setError(null);
    void loadResources(request).then((result) => { if (!disposed) auto.deliver(() => {
      if (disposed) return;
      setError(null); setPage(current => sameInventory(current, result) ? current : result);
      auto.complete();
    }); })
      .catch((reason) => { if (!disposed) auto.deliver(() => { if (!disposed) { setError(errorCode(reason)); auto.complete(reason); } }); })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; void cancelResources(request).catch(() => {}); };
  }, [clusterId, sessionId, view.kind, namespace, token, index, revision, syncing, query, status, group, requestFiltered, pending, readable]);
  useEffect(() => {
    if (!syncing) return;
    let disposed = false;
    let syncId: string | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let sequence = -1;
    const request = { clusterId, sessionId, kind: "pods" as const, readId: nextReadId(), namespace: namespace || null, continueToken: null };
    const close = (id: string) => { void closePodSync({ clusterId, sessionId, syncId: id }).catch(() => {}); };
    const accept = (snapshot: Awaited<ReturnType<typeof openPodSync>>) => {
      if (snapshot.sequence < sequence) throw "invalidResponse";
      sequence = snapshot.sequence;
      setSyncStatus(snapshot.status); setError(snapshot.error);
      if (snapshot.page) { setPage(snapshot.page); }
      setLoading(snapshot.status === "opening" && snapshot.page === null);
    };
    const poll = async () => {
      if (disposed || !syncId) return;
      try {
        const snapshot = await readPodSync({ clusterId, sessionId, syncId, readId: nextReadId() }, request);
        if (disposed) return;
        accept(snapshot);
        if (snapshot.status === "stopped" || snapshot.status === "error") { close(syncId); return; }
        timer = setTimeout(() => void poll(), 500);
      } catch (reason) {
        if (!disposed) { setError(errorCode(reason)); setSyncStatus("error"); setLoading(false); if (syncId) close(syncId); }
      }
    };
    setLoading(true); setError(null); setSyncStatus("opening");
    void openPodSync(request).then((snapshot) => {
      if (disposed) { close(snapshot.syncId); return; }
      syncId = snapshot.syncId; accept(snapshot);
      if (snapshot.status === "stopped" || snapshot.status === "error") close(syncId);
      else timer = setTimeout(() => void poll(), 500);
    }).catch((reason) => {
      if (!disposed) { setError(errorCode(reason)); setSyncStatus("error"); setLoading(false); if (syncId) close(syncId); }
    });
    return () => { disposed = true; clearTimeout(timer); if (syncId) close(syncId); };
  }, [syncing, clusterId, sessionId, namespace, index, revision]);
  const rows = useMemo(() => page?.items ?? [], [page]);
  const statuses = useMemo(() => [...new Set([...rows.map(item => item.status), ...(filters.status ? [filters.status] : [])])].sort(), [rows, filters.status]);
  const groups = useMemo(() => [...new Set([...rows.map(item => item.columns.group).filter(Boolean), ...(filters.group ? [filters.group] : [])])].sort(), [rows, filters.group]);
  const actionScope = JSON.stringify([clusterId, sessionId, view.kind, namespace, token, index, query, status, group, pending, syncing]);
  const rowsFingerprint = useMemo(() => JSON.stringify(rows), [rows]);
  const selectionScope = useMemo(() => JSON.stringify([actionScope, rowsFingerprint]), [actionScope, rowsFingerprint]);
  const visibleLoading = loading && !auto.background.current;
  const inventoryUnavailable = !readable || pending || visibleLoading || !!error || syncing && syncStatus !== "syncing";
  const selection = useResourceSelection(rows, selectionScope, !inventoryUnavailable);
  const rowMenu = useResourceRowMenu({ clusterId, sessionId, rows, scope: selectionScope, kind: view.kind!, unavailable: inventoryUnavailable, openResource, openPod });
  const tableColumns = useMemo(() => columns(view, openResource, openRelated, onNamespace, openPod), [view, openResource, openRelated, onNamespace, openPod]);
  const plugins = useMemo(() => ({ viewport: tableViewport<ResourceItem>(), selection: selection.plugin, rowContext: rowMenu.plugin }), [selection.plugin, rowMenu.plugin]);
  const emptyFiltered = rows.length === 0 && filtered;
  const emptyState = useMemo(() => {
    const resource = view.label.toLowerCase();
    return <EmptyState
      title={emptyFiltered ? `No ${resource} match your filters.` : `No ${resource} found`}
      description={emptyFiltered
        ? "Try changing your search or clearing a filter."
        : page?.truncated ? "This page is empty. Refresh or continue to the next page."
        : view.namespaced ? namespace ? `No ${resource} were found in namespace ${namespace}.` : `No ${resource} were found across all namespaces.`
        : `No ${resource} were found in this cluster.`}
      actions={<Button
        label={emptyFiltered ? "Clear filters" : "Refresh"}
        icon={<Icon icon={emptyFiltered ? IconX : IconRefresh} />}
        variant="secondary"
        isDisabled={loading || pending}
        onClick={() => emptyFiltered ? onFilter({ query: "", status: "", group: "" }) : setRevision(value => value + 1)} />}
      headingLevel={2}
      style={{ minHeight: "calc(var(--spacing-10) * 6)", width: "calc(100cqi - var(--spacing-1))", minWidth: 0,
        position: "sticky", insetInlineStart: 0, overflowWrap: "anywhere" }} />;
  }, [emptyFiltered, loading, namespace, onFilter, page?.truncated, pending, view.label, view.namespaced]);
  return <VStack gap={0} style={{ position: "relative", flex: 1, minWidth: 0, minHeight: 0, overflow: "hidden", containerType: "inline-size" }}>
    <TableToolbar label={view.label} count={readable && page ? `${rows.length}${page.truncated ? "+" : ""} items` : undefined}
      context={view.namespaced && <NamespaceSelector value={namespace} onChange={onNamespace} isLabelHidden accessWarning={accessWarning} />}>
      <TextInput label={`Search ${view.label}`} isLabelHidden value={filters.query} onChange={value => onFilter({ query: value })} placeholder={`Search ${view.label}…`}
        startIcon={IconSearch} hasClear size="sm" width="min(18rem, 100%)" />
      <Selector label="Status" isLabelHidden value={filters.status} onChange={value => onFilter({ status: value })} size="sm" presentation="adaptive"
        options={[{ value: "", label: "All statuses" }, ...statuses.map(value => ({ value, label: value }))]} />
      {view.kind === "customResourceDefinitions" && <Selector label="Group" isLabelHidden value={filters.group} onChange={value => onFilter({ group: value })} size="sm" presentation="adaptive" hasSearch
        options={[{ value: "", label: "All groups" }, ...groups.map(value => ({ value, label: value }))]} />}
      {filtered && <IconButton label="Clear filters" tooltip="Clear filters" icon={<Icon icon={IconX} />} size="sm" variant="ghost" onClick={() => onFilter({ query: "", status: "", group: "" })} />}
    </TableToolbar>
    {checking ? <ContentLoading label="Checking access…" /> : denied ? <ContentError title="Access denied"
      message={view.namespaced && !namespace ? "All-namespaces access is denied. Choose a namespace to check access within it." : "Your Kubernetes permissions do not allow viewing these resources in this scope."}
      onRetry={permissions.retry} /> : <>
    {!view.namespaced && !permissions.loading && listDecision === "unknown" && <ReadOnlyNotice unverified />}
    {syncing && syncStatus !== "syncing" && <Text role="status">Live sync: {syncStatus}</Text>}
    <ContentLoading label={`Loading ${view.label.toLowerCase()}…`} refreshing active={visibleLoading && !!page} />
    {loading && !page && <ContentLoading label={`Loading ${view.label.toLowerCase()}…`} />}
    {error && <ContentError title={`Could not load ${view.label.toLowerCase()}`} message={`${connectionMessage(error)} (${error})`} retained={rows.length > 0}
      disabled={loading || pending} onRetry={() => setRevision(value => value + 1)} />}
    {view.kind && <ResourceActions key={actionScope} clusterId={clusterId} sessionId={sessionId} kind={view.kind} namespace={namespace} createLabel={view.createLabel}
      hidePermissionNotice={view.namespaced} selected={selection.selected} disabled={inventoryUnavailable} contextTarget={rowMenu.contextTarget} onContextClose={rowMenu.onContextClose} onAccepted={selection.remove} onRefresh={() => setRevision(value => value + 1)} />}
    {page && (!error || rows.length > 0) && <ResourceTable aria-label={view.label} data={rows} columns={tableColumns} density="compact" idKey={resourceRowKey} hasHover textOverflow="truncate" aria-busy={visibleLoading || pending}
      plugins={plugins} emptyState={visibleLoading || pending ? false : emptyState} />}
    </>}
  </VStack>;
}
