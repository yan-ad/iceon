import { useEffect, useRef, useState } from "react";
import { Button } from "@astryxdesign/core/Button";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { Table } from "@astryxdesign/core/Table";
import type { SecretMetadata, SecretReveal } from "./api";

export function SecretInspector({ path, mount, metadata, revealed, busy, error, onReveal, onHide, onClose }: { path: string; mount: string; metadata: SecretMetadata | null; revealed: SecretReveal | null; busy: boolean; error: string | null; onReveal: () => void; onHide: () => void; onClose: () => void }) {
  const heading = useRef<HTMLParagraphElement>(null);
  const [tab, setTab] = useState<"Overview" | "Version history" | "Paths">("Overview");
  useEffect(() => { heading.current?.focus(); }, [path, mount]);
  return <VStack gap={4} padding={4} style={{ minWidth: 0 }}>
    <HStack gap={2} hAlign="between" wrap="wrap"><Text ref={heading} role="heading" aria-level={2} tabIndex={-1} weight="semibold" style={{ overflowWrap: "anywhere" }}>{path}</Text><Button label="Close secret details" variant="ghost" size="sm" onClick={onClose} /></HStack>
    <HStack gap={1} wrap="wrap" role="group" aria-label="Secret detail sections">{(["Overview", "Version history", "Paths"] as const).map(item => <Button key={item} label={item} variant={item === tab ? "secondary" : "ghost"} size="sm" aria-pressed={item === tab} onClick={() => { onHide(); setTab(item); }} />)}</HStack>
    {busy && <Text role="status">Reading secret details…</Text>}
    {error && <Text role="alert">{error} Use Refresh in the toolbar to retry.</Text>}
    {metadata?.partial && <Text role="status">Secret metadata is partial. Some versions or fields may be missing.</Text>}
    {metadata && tab === "Overview" && <VStack gap={3}>
      <Text>Current version: {metadata.currentVersion}</Text>
      <Text style={{ overflowWrap: "anywhere" }}>Created: {metadata.createdAt || "Not reported"}</Text>
      <Text style={{ overflowWrap: "anywhere" }}>Updated: {metadata.updatedAt || "Not reported"}</Text>
      <Button label={revealed ? "Hide values" : "Reveal values"} variant="secondary" isDisabled={busy || !!error} onClick={revealed ? onHide : onReveal} />
      <Text type="supporting">Values are hidden by default. Revealed values hide within 30 seconds, on navigation, or when the window loses focus.</Text>
      {revealed && <VStack gap={3} aria-label="Revealed secret values"><Text>Revealed version: {revealed.version}</Text>{revealed.fields.length === 0 ? <Text>This secret has no fields.</Text> : revealed.fields.map((field, index) => <VStack gap={1} key={`${index}:${field.name}`}><Text weight="semibold" style={{ overflowWrap: "anywhere" }}>{field.name}</Text><Text style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{field.value}</Text></VStack>)}</VStack>}
    </VStack>}
    {metadata && tab === "Version history" && <Table data={metadata.versions} idKey="version" density="compact" columns={[{ key: "version", header: "Version" }, { key: "createdAt", header: "Created" }, { key: "state", header: "State", renderCell: version => <Text>{version.destroyed ? "Destroyed" : version.deletionTime ? "Deleted" : "Available"}</Text> }]} />}
    {metadata && tab === "Paths" && <VStack gap={3}><Text style={{ overflowWrap: "anywhere" }}>API data path: /v1/{mount}/data/{path}</Text><Text style={{ overflowWrap: "anywhere" }}>API metadata path: /v1/{mount}/metadata/{path}</Text><Text style={{ overflowWrap: "anywhere" }}>CLI path: {mount}/{path}</Text></VStack>}
  </VStack>;
}
