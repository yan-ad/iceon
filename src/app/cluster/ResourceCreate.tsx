import { useEffect, useRef, useState } from "react";
import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { AddButton } from "../components/AddButton";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { YamlEditor } from "../components/YamlEditor";
import { useCatalog } from "../catalog/CatalogProvider";
import { nextReadId, type ResourceKind, type SessionRequest } from "./api";
import { cancelResourceCreate, createMessage, createResource, creationNamespaced, type CreateCancelRequest, type CreateResult } from "./createApi";
import { resourceCreation } from "./resourceViews";
import { usePermissionChecks } from "./PermissionProvider";
import { NamespaceSelector } from "./NamespaceScope";
import { useUpdateBlocker } from "../updates/UpdateProvider";

type Props = SessionRequest & { kind: ResourceKind; namespace?: string; label?: string; onCreated: () => void; inline?: boolean; onNotice?: (notice: string) => void };
const uncertainty = "Creation may already have reached the API server. Cancellation does not roll it back. Refresh the inventory and check the resource before retrying.";

export function ResourceCreate(props: Props) {
  return <ScopedResourceCreate key={JSON.stringify([props.clusterId, props.sessionId, props.kind, props.namespace ?? ""])} {...props} />;
}
function ScopedResourceCreate(props: Props) {
  const { snapshot } = useCatalog();
  const cluster = snapshot?.clusters.find(item => item.id === props.clusterId);
  const [open, setOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const label = props.label ?? `Create ${resourceCreation[props.kind].kind}`;
  const chooseNamespace = creationNamespaced(props.kind) && !props.namespace;
  const check = { kind: props.kind, namespace: props.namespace || null, action: chooseNamespace ? "list" as const : "create" as const };
  const permissions = usePermissionChecks([check]);
  return <>
    {!props.onNotice && notice && <Text role="status">{notice}</Text>}
    {permissions.decision(check) === "allowed" && <AddButton label={label} tooltip={label}
      style={props.inline ? { pointerEvents: "auto", borderRadius: "var(--radius-container)" } : { position: "absolute", right: "var(--spacing-4)", bottom: "var(--spacing-4)", borderRadius: "var(--radius-container)" }}
      isDisabled={!cluster} onClick={() => { if (permissions.decision(check) === "allowed") { setNotice(null); setOpen(true); } }} />}
    {open && cluster && <CreateEditor {...props} label={label} clusterName={cluster.name} contextName={cluster.contextName}
      onClose={(sent) => { setOpen(false); if (sent) { setNotice(uncertainty); props.onNotice?.(uncertainty); } }} onCreated={() => {
        const message = `${resourceCreation[props.kind].kind} created. Refreshing inventory…`;
        setOpen(false); setNotice(message); props.onNotice?.(message); props.onCreated();
      }} />}
  </>;
}
export function CreateEditor({ clusterId, sessionId, kind, namespace, label, clusterName, contextName, onCreated, onClose }: Props & {
  label: string; clusterName: string; contextName: string; onClose: (sent: boolean) => void;
}) {
  const info = resourceCreation[kind];
  const namespaced = creationNamespaced(kind);
  useUpdateBlocker(true, "Resource creation is open.");
  const [targetNamespace, setTargetNamespace] = useState(namespace || "default");
  const check = { kind, namespace: namespaced ? targetNamespace : null, action: "create" as const };
  const permissions = usePermissionChecks([check]);
  const allowed = () => permissions.decision(check) === "allowed";
  const [yaml, setYaml] = useState(() => `apiVersion: ${info.apiVersion}\nkind: ${info.kind}\nmetadata:\n  name: \"\"\n${namespaced && namespace ? `  namespace: ${namespace}\n` : ""}`);
  const [validated, setValidated] = useState<CreateResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const active = useRef<CreateCancelRequest | null>(null);
  const alive = useRef(false);
  const sent = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      const request = active.current;
      active.current = null;
      if (request) void cancelResourceCreate(request).catch(() => {});
    };
  }, []);
  const close = () => {
    alive.current = false;
    const request = active.current;
    active.current = null;
    if (request) void cancelResourceCreate(request).catch(() => {});
    onClose(sent.current);
  };
  const submit = async (dryRun: boolean) => {
    if (!alive.current || active.current || !allowed() || !dryRun && !validated) return;
    const request = { clusterId, sessionId, kind, readId: nextReadId() };
    active.current = request;
    setBusy(true); setError(null);
    if (!dryRun) sent.current = true;
    try {
      const result = await createResource({ ...request, namespace: namespaced ? targetNamespace : null, yaml, dryRun });
      if (!alive.current || active.current !== request) return;
      if (dryRun) setValidated(result);
      else {
        if (result.name !== validated!.name || result.namespace !== validated!.namespace) throw "invalidResponse";
        sent.current = false;
        onCreated();
      }
    } catch (reason) {
      if (!alive.current || active.current !== request) return;
      setValidated(null);
      setError(createMessage(reason));
      if (!dryRun) setUncertain(true);
    } finally {
      if (alive.current && active.current === request) { active.current = null; setBusy(false); }
    }
  };
  return <Dialog isOpen onOpenChange={value => { if (!value) close(); }} purpose="form" width="min(44rem, 100%)">
    <DialogHeader title={validated ? `Confirm ${info.kind} creation` : label} onOpenChange={value => { if (!value) close(); }} />
    <VStack gap={4} padding={4} style={{ minHeight: 0, overflowY: "auto", overflowWrap: "anywhere" }}>
      <VStack gap={1} aria-label="Creation target">
        <Text>Cluster: {clusterName}</Text><Text>Cluster ID: {clusterId}</Text><Text>Context: {contextName}</Text>
        <Text>Kind: {info.kind} ({info.apiVersion})</Text>
        <Text>Namespace: {validated ? validated.namespace ?? "Cluster-scoped" : namespaced ? targetNamespace : "Cluster-scoped"}</Text>
        {namespaced && !namespace && !validated && <NamespaceSelector value={targetNamespace} allowAll={false} onChange={value => { if (!busy && value) { setTargetNamespace(value); setValidated(null); } }} />}
        {validated && <Text>Name: {validated.name}</Text>}
      </VStack>
      {validated ? <Text>Server dry-run passed. Confirm creation of this exact resource. Manifest contents are hidden during confirmation.</Text> : <>
        <Text>Metadata scaffold only. Fill in a name and all required resource-specific fields. YAML is kept only in this dialog and sent to the Rust engine; do not paste credentials you do not intend to send to this cluster.</Text>
        <YamlEditor label="Resource YAML" value={yaml} rows={14} isReadOnly={busy}
          onChange={value => { if (!active.current) { setYaml(value); setValidated(null); setError(null); } }} />
      </>}
      {error && <Text role="alert">{error}</Text>}
      {!allowed() && <Text role="status">{permissions.loading ? "Checking create access…" : permissions.decision(check) === "denied" ? "Creation is not allowed in this namespace." : "Access unverified. Creation is disabled until access is verified."}</Text>}
      {(uncertain || busy && sent.current) && <Text role="status">{uncertainty}</Text>}
      <HStack gap={2} wrap="wrap">
        <Button label={busy ? "Cancel request" : "Cancel"} variant="ghost" onClick={close} />
        {validated && <Button label="Edit YAML" variant="secondary" isDisabled={busy} onClick={() => setValidated(null)} />}
        <Button label={validated ? `Create ${info.kind}` : "Validate YAML"} isLoading={busy} isDisabled={!yaml.trim() || !allowed()}
          onClick={() => void submit(!validated)} />
      </HStack>
    </VStack>
  </Dialog>;
}
