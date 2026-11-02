import type { ComponentProps } from "react";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { IconPlus } from "@tabler/icons-react";

type Props = Omit<ComponentProps<typeof IconButton>, "icon" | "size" | "variant" | "elevation">;

export function AddButton({ style, tooltip, label, ...props }: Props) {
  return <IconButton {...props} label={label} tooltip={tooltip ?? label}
    icon={<Icon icon={IconPlus} />} size="lg" variant="primary" elevation="med"
    style={{ ...style, borderRadius: "var(--radius-container)",
      width: "calc(var(--spacing-10) + var(--spacing-1))", height: "calc(var(--spacing-10) + var(--spacing-1))",
      minWidth: "calc(var(--spacing-10) + var(--spacing-1))", minHeight: "calc(var(--spacing-10) + var(--spacing-1))" }} />;
}
