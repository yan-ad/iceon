import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Button } from "@astryxdesign/core/Button";
import { Collapsible } from "@astryxdesign/core/Collapsible";
import { Icon } from "@astryxdesign/core/Icon";
import { IconFolder, IconFileUpload } from "@tabler/icons-react";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { Layout, LayoutContent, LayoutFooter } from "@astryxdesign/core/Layout";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Selector } from "@astryxdesign/core/Selector";
import { useCatalog } from "./CatalogProvider";
import { AddButton } from "../components/AddButton";
import { chooseKubeconfig, filePickerError } from "./filePicker";
import { bindFileDrop } from "./fileDrop";
import { platformGuidance, platformOptions } from "./platformGuidance";
import { getWindowLayout } from "../layouts/window-layout";

export function CatalogForms() {
  const { run, busy, available, error, snapshot } = useCatalog();
  const [isOpen, setOpen] = useState(false);
  const [path, setPath] = useState("");
  const [platform, setPlatform] = useState("generic");
  const [helpOpen, setHelpOpen] = useState(false);
  const [picking, setPicking] = useState(false);
  const [pickerError, setPickerError] = useState<string | null>(null);
  const [dropReady, setDropReady] = useState(false);
  const [hovering, setHovering] = useState(false);
  const [selectionStatus, setSelectionStatus] = useState("");
  const pickerSession = useRef(0);
  const pickerPending = useRef(false);
  const dropSurface = useRef<HTMLElement>(null);
  const pathInput = useRef<HTMLInputElement>(null);
  const acceptDrop = useRef(false);
  acceptDrop.current = isOpen && !busy && !picking && available && !!snapshot;
  const native = isTauri();
  const nativeDrop = native && getWindowLayout().platform !== "browser";

  useEffect(() => () => { pickerSession.current += 1; acceptDrop.current = false; }, []);
  useEffect(() => {
    if (!isOpen || !nativeDrop || busy || picking) { setDropReady(false); setHovering(false); return; }
    return bindFileDrop({
      subscribe: handler => getCurrentWebview().onDragDropEvent(event => handler(event.payload)),
      surface: () => dropSurface.current,
      scale: () => window.devicePixelRatio,
      canAccept: () => acceptDrop.current && !pickerPending.current,
      hovering: setHovering,
      select: selected => {
        setPath(selected); setPickerError(null); setSelectionStatus("File selected. Import when ready.");
        pathInput.current?.focus({ preventScroll: true });
      },
      error: setPickerError,
      ready: () => setDropReady(true),
    });
  }, [isOpen, nativeDrop, busy, picking]);

  function invalidatePicker() {
    pickerSession.current += 1;
    pickerPending.current = false;
    setPicking(false);
    setPickerError(null);
    setHovering(false);
    setDropReady(false);
    setSelectionStatus("");
  }
  function open() {
    invalidatePicker();
    setPlatform("generic");
    setHelpOpen(false);
    setOpen(true);
  }
  async function chooseFile() {
    if (pickerPending.current || busy || !available || !isOpen) return;
    pickerPending.current = true;
    setPicking(true);
    setPickerError(null);
    const session = pickerSession.current;
    try {
      const selected = await chooseKubeconfig();
      if (session === pickerSession.current && selected !== null) {
        setPath(selected); setSelectionStatus("File selected. Import when ready.");
      }
    } catch {
      if (session === pickerSession.current) setPickerError(filePickerError);
    } finally {
      if (session === pickerSession.current) {
        pickerPending.current = false;
        setPicking(false);
      }
    }
  }
  function close() {
    if (!busy) {
      acceptDrop.current = false;
      invalidatePicker();
      setOpen(false);
      setPath("");
      setPlatform("generic");
    }
  }
  async function submit() {
    if (pickerPending.current || busy || !available || !path.trim()) return;
    acceptDrop.current = false;
    const saved = await run("catalog_import_kubeconfig", { path });
    if (saved) {
      invalidatePicker();
      setOpen(false);
      setPath("");
    }
  }
  const guidance = platformGuidance.find(item => item.id === platform) ?? platformGuidance[0];
  const disabled = busy || !available || !snapshot;
  return (
    <>
      <HStack gap={2} style={{ flexShrink: 0 }}>
        <AddButton
          label="Add config"
          tooltip="Add config"
          isDisabled={disabled}
          onClick={open}
        />
      </HStack>
      <Dialog isOpen={isOpen} onOpenChange={open => { if (!open) close(); }} purpose={busy ? "required" : "form"}>
        <Layout
          header={<DialogHeader title="Import kubeconfig" onOpenChange={busy ? undefined : close} />}
          content={
            <LayoutContent>
              <VStack gap={4}>
                <Text as="p">Import context metadata. Credentials stay out of the catalog. No connections or authentication commands will run.</Text>
                <VStack
                  ref={dropSurface}
                  as="section"
                  gap={3}
                  padding={4}
                  aria-label="Kubeconfig file"
                  style={{
                    minWidth: 0,
                    border: "var(--border-width) dashed var(--color-border)",
                    borderRadius: "var(--radius-container)",
                    background: hovering ? "var(--color-fill-ghost-hover)" : undefined,
                    outline: hovering ? "var(--focus-outline-width) solid var(--color-border)" : undefined,
                  }}
                >
                  <HStack gap={2} vAlign="center">
                    <Icon icon={IconFileUpload} />
                    <VStack gap={0.5}>
                      <Text weight="bold">{hovering ? "Drop to select" : "Select a kubeconfig"}</Text>
                      <Text type="supporting" size="sm">{dropReady ? "Drop one file here, browse, or paste its path." : "Choose a file or paste its full path."}</Text>
                    </VStack>
                  </HStack>
                  <HStack gap={2} vAlign="end">
                    <TextInput
                      ref={pathInput}
                      label="Kubeconfig path"
                      containerStyle={{ flex: "1 1 0", minWidth: 0 }}
                      value={path}
                      onChange={value => { setPath(value); setPickerError(null); setSelectionStatus(""); }}
                      isRequired
                      isDisabled={busy || picking}
                      autoComplete="off"
                      placeholder="/path/to/config"
                    />
                    <Button
                      label="Choose File"
                      icon={<Icon icon={IconFolder} />}
                      variant="secondary"
                      style={{ flexShrink: 0 }}
                      isDisabled={busy || picking || !available || !native}
                      isLoading={picking}
                      onClick={() => void chooseFile()}
                    />
                  </HStack>
                  {selectionStatus && <Text role="status" size="sm">{selectionStatus}</Text>}
                  {pickerError && <Text role="alert" size="sm">{pickerError}</Text>}
                </VStack>
                <Collapsible trigger="Need help with your platform?" isOpen={helpOpen} onOpenChange={setHelpOpen} isDisabled={busy || picking}>
                  <VStack gap={3}>
                    <Selector label="Platform guidance" value={platform} options={platformOptions} hasSearch searchPlaceholder="Search platforms" onChange={setPlatform} isDisabled={busy || picking} />
                    <VStack gap={2} role="note" aria-label={`${guidance.label} access guidance`}>
                      <Text size="sm">{guidance.prepare}</Text>
                      <Text size="sm" type="supporting">{guidance.access}</Text>
                    </VStack>
                  </VStack>
                </Collapsible>
                <Text type="supporting" size="sm">Use a trusted file. Opening a workspace may run its credential helper.</Text>
                {error && <Text role="alert">{error}</Text>}
              </VStack>
            </LayoutContent>
          }
          footer={
            <LayoutFooter hasDivider>
              <HStack gap={2} hAlign="end">
                <Button label="Cancel" variant="ghost" isDisabled={busy} onClick={close} />
                <Button label={busy ? "Saving…" : "Import contexts"} isDisabled={busy || picking || !path.trim()} onClick={() => void submit()} />
              </HStack>
            </LayoutFooter>
          }
        />
      </Dialog>
    </>
  );
}
