import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@astryxdesign/core/Button";
import { Heading } from "@astryxdesign/core/Heading";
import { AddButton } from "../components/AddButton";
import { VStack } from "@astryxdesign/core/Stack";
import { Table, proportional, type TableColumn } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { IconSearch } from "@tabler/icons-react";
import { ContentError } from "../components/ContentError";
import { ContentLoading } from "../components/ContentLoading";
import { connectionMessage, errorCode, nextReadId, type SessionRequest } from "./api";
import { NamespaceSelector } from "./NamespaceScope";
import { usePermissionChecks } from "./PermissionProvider";
import { cancelManagedUsers, loadManagedUsers, validUsersNamespace, type ManagedUser, type ManagementSnapshot } from "./managedUsersApi";
import { ManagedUserEditor } from "./ManagedUserEditor";
import { TableActionOverlay, TableToolbar, tableViewport } from "./TableToolbar";
import { useRegisterWorkspaceRefresh } from "./WorkspaceRefresh";

export function ManagedUsersPage(props: SessionRequest) {
  return <ManagedUsersSession key={`${props.clusterId}:${props.sessionId}`} {...props} />;
}
function ManagedUsersSession({ clusterId, sessionId }: SessionRequest) {
  const [namespace, setNamespace] = useState("");
  const [query, setQuery] = useState("");
  const [revision, setRevision] = useState(0);
  const [snapshot, setSnapshot] = useState<ManagementSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<ManagedUser | null>(null);
  const activeRead = useRef<(() => void) | null>(null);
  const changeNamespace = (next: string) => {
    if (!validUsersNamespace(next) || next === namespace) return;
    activeRead.current?.(); activeRead.current = null;
    setSnapshot(null); setSelected(null); setError(null); setLoading(false); setQuery(""); setNamespace(next);
  };
  const permissionChecks = usePermissionChecks(namespace ? [{ kind: "serviceAccounts" as const, action: "list" as const, namespace }, { kind: "serviceAccounts" as const, action: "create" as const, namespace }, { kind: "clusterRoles" as const, action: "list" as const, namespace: null }] : []);
  const permissionKnown = namespace !== "" && permissionChecks.decision({ kind: "serviceAccounts", action: "list", namespace }) === "allowed" && permissionChecks.decision({ kind: "clusterRoles", action: "list", namespace: null }) === "allowed";
  const permissionDenied = namespace !== "" && (permissionChecks.decision({ kind: "serviceAccounts", action: "list", namespace }) === "denied" || permissionChecks.decision({ kind: "clusterRoles", action: "list", namespace: null }) === "denied");
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  useRegisterWorkspaceRefresh({ clusterId, section: "users", enabled: !!namespace && !loading && !selected, refresh });
  const editing = selected !== null;
  useEffect(() => {
    if (editing) return;
    if (!namespace || permissionChecks.loading || !permissionKnown) { setSnapshot(null); setLoading(false); return; }
    const request = { clusterId, sessionId, namespace, readId: nextReadId() };
    let disposed = false;
    const dispose = () => { if (disposed) return; disposed = true; void cancelManagedUsers(request).catch(() => {}); };
    activeRead.current = dispose;
    setLoading(true); setError(null); setSnapshot(null);
    void loadManagedUsers(request).then(value => { if (!disposed) setSnapshot(value); }).catch(reason => { if (!disposed) setError(connectionMessage(errorCode(reason))); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { dispose(); if (activeRead.current === dispose) activeRead.current = null; };
  }, [clusterId, sessionId, namespace, revision, permissionChecks.loading, permissionKnown, editing]);
  const items = useMemo(() => { const needle = query.trim().toLocaleLowerCase(); return (snapshot?.users ?? []).filter(user => user.name.toLocaleLowerCase().includes(needle)); }, [snapshot, query]);
  const open = (user: ManagedUser) => setSelected(user);
  const columns: TableColumn<ManagedUser>[] = [
    { key: "name", header: "Name", width: proportional(3), renderCell: user => <Button label={`Edit ${user.name}`} variant="ghost" size="sm" onClick={() => open(user)}>{user.name}</Button> },
    { key: "namespace", header: "Namespace", width: proportional(2), renderCell: user => <Text>{user.namespace}</Text> },
    { key: "assignments", header: "Assignments", width: proportional(2), renderCell: user => <Text>{user.assignments.length} namespaced, {user.clusterAccess} cluster</Text> },
    { key: "actions", header: "Actions", width: proportional(1), renderCell: user => <Button label={`Edit ${user.name}`} size="sm" variant="ghost" onClick={() => open(user)}>Edit</Button> },
  ];
  if (!namespace) return <VStack gap={4} padding={6} hAlign="center" vAlign="center" style={{ flex: "1 1 0", minHeight: 0 }}>
    <Heading level={2}>Choose a user namespace</Heading>
    <Text>User management is scoped to one explicit namespace. No users or templates are fetched until you choose it.</Text>
    <NamespaceSelector label="User namespace" isLabelHidden={false} allowAll={false} value="" onChange={changeNamespace} />
  </VStack>;
  const unavailable = permissionChecks.loading || !permissionKnown || loading || !!error && !snapshot;
  const canCreate = permissionChecks.decision({ kind: "serviceAccounts", action: "create", namespace }) === "allowed" && permissionChecks.decision({ kind: "clusterRoles", action: "list", namespace: null }) === "allowed";
  return <VStack gap={0} style={{ position: "relative", flex: "1 1 0", minWidth: 0, minHeight: 0, overflow: "hidden" }}>
    <TableToolbar label="Managed users" context={<NamespaceSelector label="User namespace" allowAll={false} value={namespace} onChange={changeNamespace} />}>
      <TextInput label="Search users" isLabelHidden placeholder="Search users" value={query} onChange={setQuery} size="sm" startIcon={IconSearch} hasClear />
    </TableToolbar>
    <VStack gap={0} style={{ flex: "1 1 0", minWidth: 0, minHeight: 0, overflow: "auto", paddingBottom: "calc(var(--spacing-10) * 2 + var(--spacing-4))" }}>
    {permissionChecks.loading ? <ContentLoading label="Checking managed user access…" /> : !permissionKnown ? <ContentError title={permissionDenied ? "Access denied" : "Access is unverified"} message="Managed users require ServiceAccount list and ClusterRole list access. No user data was requested." onRetry={permissionChecks.retry} /> : loading ? <ContentLoading label="Loading managed users…" /> : error && !snapshot ? <ContentError title="Managed users unavailable" message={error} onRetry={refresh} /> : null}
    {snapshot?.truncated && <Text role="status" style={{ padding: "var(--spacing-3)" }}>The managed user snapshot is partial. Missing users or templates are not treated as absent.</Text>}
    {error && <Text role="alert" style={{ padding: "var(--spacing-3)" }}>{error} Previously loaded users are stale.</Text>}
    {!unavailable && !items.length && <Text role="status" style={{ padding: "var(--spacing-3)" }}>{query ? "No managed users match this search." : "No managed users in this namespace."}</Text>}
    {!unavailable && !!items.length && <Table aria-label="Managed users" data={items} columns={columns} idKey="name" density="compact" plugins={{ viewport: tableViewport<ManagedUser>() }} />}
    <Text type="supporting" style={{ padding: "var(--spacing-3)" }}>Inspired by Permission Manager by SIGHUP (BSD-3-Clause). Managed users are ServiceAccounts, not generated credentials.</Text>
    </VStack>
    <TableActionOverlay label="Managed user actions">
      <AddButton label="Create New User" tooltip="Create New User"
        isDisabled={!canCreate || unavailable || !snapshot} onClick={() => setSelected({ name: "", namespace, uid: null, resourceVersion: null, assignments: [], clusterAccess: "none", clusterTemplate: null })} />
    </TableActionOverlay>
    {selected && snapshot && <ManagedUserEditor clusterId={clusterId} sessionId={sessionId} initial={selected} templates={snapshot.templates} onClose={changed => { setSelected(null); if (changed) refresh(); }} />}
  </VStack>;
}
export default ManagedUsersPage;
