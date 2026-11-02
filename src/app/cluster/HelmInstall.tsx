import { useEffect, useRef, useState } from "react";
import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { AddButton } from "../components/AddButton";
import { Selector } from "@astryxdesign/core/Selector";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { YamlEditor } from "../components/YamlEditor";
import { TextInput } from "@astryxdesign/core/TextInput";
import { useUpdateBlocker } from "../updates/UpdateProvider";
import { TableActionOverlay } from "./TableToolbar";
import { useCatalog } from "../catalog/CatalogProvider";
import { nextReadId, type SessionRequest } from "./api";
import { NamespaceSelector } from "./NamespaceScope";
import { cancelHelmInstall, helmInstallInputError, helmInstallMessage, installHelm, type HelmInstallCancelRequest, type HelmInstallRequest, type HelmItem } from "./helmApi";

type Props = SessionRequest & { contextChart?: HelmItem | null; onContextClose?: () => void; namespace: string; charts: HelmItem[]; onInstalled: () => void };
const uncertainty = "Installation may already have reached the cluster. Cancellation does not roll it back. Refresh Releases and check the release before retrying.";

export function HelmInstall(props: Props) {
  return <ScopedHelmInstall key={JSON.stringify([props.clusterId, props.sessionId, props.namespace, props.contextChart])} {...props} />;
}
function ScopedHelmInstall(props: Props) {
  const { snapshot } = useCatalog();
  const cluster = snapshot?.clusters.find(item => item.id === props.clusterId);
  const [open, setOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => { if (props.contextChart) setOpen(true); }, [props.contextChart]);
  return <>
    {notice && <Text role="status">{notice}</Text>}
    <TableActionOverlay label="Helm actions"><AddButton label="Install Helm chart" tooltip="Install Helm chart"
      style={{ pointerEvents: "auto", borderRadius: "var(--radius-container)" }}
      isDisabled={!cluster} onClick={() => { setNotice(null); setOpen(true); }} /></TableActionOverlay>
    {open && cluster && <InstallEditor {...props} clusterName={cluster.name} contextName={cluster.contextName}
      onClose={sent => { setOpen(false); props.onContextClose?.(); if (sent) setNotice(uncertainty); }} onInstalled={() => {
        setOpen(false); props.onContextClose?.(); setNotice("Helm release installed. Refreshing inventory…"); props.onInstalled();
      }} />}
  </>;
}
function InstallEditor({ clusterId, sessionId, namespace: initialNamespace, charts, contextChart, clusterName, contextName, onClose, onInstalled }: Props & {
  clusterName: string; contextName: string; onClose: (sent: boolean) => void;
}) {
  useUpdateBlocker(true, "Helm installation is open.");
  const [namespace, setNamespace] = useState(initialNamespace);
  const [releaseName, setReleaseName] = useState("");
  const [chart, setChart] = useState(contextChart?.name ?? "");
  const [version, setVersion] = useState(contextChart?.version ?? "");
  const [valuesYaml, setValuesYaml] = useState("{}\n");
  const [validated, setValidated] = useState<HelmInstallRequest | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const active = useRef<HelmInstallCancelRequest | null>(null);
  const alive = useRef(false);
  const sent = useRef(false);
  const cancel = () => {
    const request = active.current;
    active.current = null;
    if (request) void cancelHelmInstall(request).catch(() => {});
  };
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; cancel(); };
  }, []);
  const close = () => { alive.current = false; cancel(); onClose(sent.current); };
  const edit = (action: () => void) => {
    if (active.current) return;
    action(); setValidated(null); setError(null);
  };
  const changeNamespace = (value: string) => {
    if (value === namespace) return;
    cancel(); setBusy(false); setValidated(null); setError(null); setNamespace(value);
    if (sent.current) setUncertain(true);
  };
  const submit = async () => {
    if (!alive.current || active.current) return;
    const dryRun = !validated;
    const input = validated ?? { clusterId, sessionId, namespace, releaseName, chart, version: version || null, valuesYaml, dryRun: true, readId: 0 };
    const invalid = helmInstallInputError(input);
    if (invalid) { setError(invalid); return; }
    let request: HelmInstallRequest;
    try { request = { ...input, readId: nextReadId(), dryRun }; }
    catch (reason) { setError(helmInstallMessage(reason)); return; }
    const scope = { clusterId, sessionId, readId: request.readId };
    active.current = scope; setBusy(true); setError(null);
    if (!dryRun) sent.current = true;
    try {
      await installHelm(request);
      if (!alive.current || active.current !== scope) return;
      if (dryRun) setValidated(request);
      else { sent.current = false; onInstalled(); }
    } catch (reason) {
      if (!alive.current || active.current !== scope) return;
      setValidated(null); setError(helmInstallMessage(reason));
      if (!dryRun) setUncertain(true);
    } finally {
      if (alive.current && active.current === scope) { active.current = null; setBusy(false); }
    }
  };
  const cachedCharts = [...new Set(charts.map(item => item.name))];
  return <Dialog isOpen onOpenChange={value => { if (!value) close(); }} purpose="form" width="min(44rem, 100%)">
    <DialogHeader title={validated ? "Confirm Helm installation" : "Install Helm chart"} onOpenChange={value => { if (!value) close(); }} />
    <VStack gap={4} padding={4} style={{ minHeight: 0, overflowY: "auto", overflowWrap: "anywhere" }}>
      <VStack gap={1} aria-label="Installation target">
        <Text>Cluster: {clusterName}</Text><Text>Cluster ID: {clusterId}</Text><Text>Context: {contextName}</Text>
        {validated && <><Text>Namespace: {validated.namespace}</Text><Text>Release: {validated.releaseName}</Text><Text>Chart: {validated.chart}</Text><Text>Version: {validated.version ?? "Repository default (not pinned)"}</Text></>}
      </VStack>
      {validated ? <Text>Server dry-run passed. Confirm installation of this exact target. Values are hidden during confirmation. An unpinned version is resolved again by Helm.</Text> : <>
        <NamespaceSelector value={namespace} onChange={changeNamespace} allowAll={false} isLabelHidden={false} />
        <TextInput label="Release name" value={releaseName} onChange={value => edit(() => setReleaseName(value))} isReadOnly={busy} autoComplete="off" />
        {cachedCharts.length > 0 && <Selector label="Cached chart" value={cachedCharts.includes(chart) ? chart : ""} placeholder="Choose a cached chart" options={cachedCharts.map(value => ({ value, label: value }))}
          onChange={value => edit(() => setChart(value))} isDisabled={busy} />}
        <TextInput label="Chart repository/name" value={chart} onChange={value => edit(() => setChart(value))} isReadOnly={busy} autoComplete="off" placeholder="repository/chart" />
        <TextInput label="Chart version (optional)" value={version} onChange={value => edit(() => setVersion(value))} isReadOnly={busy} autoComplete="off" />
        <YamlEditor label="Values YAML" value={valuesYaml} onChange={value => edit(() => setValuesYaml(value))} rows={10} isReadOnly={busy} />
        <Text type="supporting">Values stay in this dialog and are sent only to the Rust engine. Helm may store supplied values in the cluster release record.</Text>
      </>}
      {error && <Text role="alert">{error}</Text>}
      {(uncertain || busy && sent.current) && <Text role="status">{uncertainty}</Text>}
      <HStack gap={2} wrap="wrap">
        <Button label={busy ? "Cancel request" : "Cancel"} variant="ghost" onClick={close} />
        {validated && <Button label="Edit installation" variant="secondary" isDisabled={busy} onClick={() => edit(() => {})} />}
        <Button label={validated ? "Install release" : "Run server dry-run"} isLoading={busy} isDisabled={busy || !namespace || !releaseName || !chart} onClick={() => void submit()} />
      </HStack>
    </VStack>
  </Dialog>;
}
