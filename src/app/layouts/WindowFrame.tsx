import { useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { HStack, StackItem, VStack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { IconMinus, IconSquare, IconX } from "@tabler/icons-react";
import type { WindowLayout } from "./window-layout";

const chromeHeight = "calc(var(--spacing-10) + var(--spacing-1))";

export function WindowFrame({
  seamless,
  platform,
  children,
}: WindowLayout & { children: ReactNode }) {
  const hasChrome = seamless && platform !== "browser";

  return (
    <VStack
      gap={0}
      height="100%"
      data-seamless={hasChrome || undefined}
      style={{
        boxSizing: "border-box",
        overscrollBehavior: "none",
        paddingTop: "env(safe-area-inset-top, 0rem)",
        paddingRight: "env(safe-area-inset-right, 0rem)",
        paddingBottom: "env(safe-area-inset-bottom, 0rem)",
        paddingLeft: "env(safe-area-inset-left, 0rem)",
      }}
    >
      <StackItem size="fill" style={{ overflow: "hidden" }}>
        {children}
      </StackItem>
    </VStack>
  );
}

export function WindowToolbar({
  seamless,
  platform,
  children,
}: WindowLayout & { children: ReactNode }) {
  const [error, setError] = useState<string | null>(null);
  const hasChrome = seamless && platform !== "browser";

  async function runWindowAction(
    action: "minimize" | "toggleMaximize" | "close",
  ) {
    try {
      await getCurrentWindow()[action]();
      setError(null);
    } catch {
      setError(
        "Window action failed. Try again or use your operating system's window menu.",
      );
    }
  }

  return (
    <VStack gap={0}>
      <HStack
        as="nav"
        aria-label="App toolbar"
        gap={0}
        vAlign="center"
        data-testid={hasChrome ? "window-safe-area" : undefined}
        style={{ minHeight: chromeHeight, flexShrink: 0, userSelect: "none",
          paddingRight: hasChrome && platform === "macos" ? "var(--spacing-3)" : undefined }}
      >
        {hasChrome && platform === "macos" && (
          <StackItem
            aria-hidden="true"
            data-tauri-drag-region
            style={{
              width: "calc(var(--spacing-10) * 2)",
              alignSelf: "stretch",
              flexShrink: 0,
            }}
          />
        )}
        {children}
        {hasChrome && platform !== "macos" && (
          <HStack gap={0} aria-label="Window controls">
            {(
              [
                ["minimize", "Minimize window", IconMinus],
                ["toggleMaximize", "Maximize or restore window", IconSquare],
                ["close", "Close window", IconX],
              ] as const
            ).map(([action, label, icon]) => (
              <IconButton
                key={action}
                label={label}
                tooltip={label}
                variant="ghost"
                icon={<Icon icon={icon} />}
                style={{ minWidth: chromeHeight, minHeight: chromeHeight }}
                onClick={() => void runWindowAction(action)}
              />
            ))}
          </HStack>
        )}
      </HStack>
      {error && <Text role="alert">{error}</Text>}
    </VStack>
  );
}
