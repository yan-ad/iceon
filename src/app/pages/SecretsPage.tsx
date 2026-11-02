import { Layout, LayoutContent } from "@astryxdesign/core/Layout";
import { VStack } from "@astryxdesign/core/Stack";
import { Heading } from "@astryxdesign/core/Heading";
import { VisuallyHidden } from "@astryxdesign/core/VisuallyHidden";
import { SecretsPanel } from "../secrets/SecretsPanel";

export default function SecretsPage() {
  return <Layout height="fill" content={<LayoutContent padding={0} isScrollable={false}>
    <VStack style={{ height: "100%", minHeight: 0, minWidth: 0, overflow: "hidden" }}>
      <VisuallyHidden><Heading level={1}>Secrets</Heading></VisuallyHidden>
      <SecretsPanel />
    </VStack>
  </LayoutContent>} />;
}
