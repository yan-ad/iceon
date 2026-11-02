import type { IconType } from "@astryxdesign/core/Icon";
import { IconHexagons, IconKey } from "@tabler/icons-react";

export const destinations = [
  { path: "/catalog/clusters", label: "Clusters", icon: IconHexagons, description: "Kubernetes clusters and pinned connections." },
  { path: "/secrets", label: "Secrets", icon: IconKey, description: "Saved provider connections and read-only secret browsing." },
] as const satisfies readonly { path: string; label: string; icon: IconType; description: string }[];

export type DestinationPath = (typeof destinations)[number]["path"];
export const categories = [destinations[0]] as const;
