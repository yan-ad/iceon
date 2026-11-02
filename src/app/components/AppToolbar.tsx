import { useCallback, useEffect, useState } from "react";
import { useNavigate, useParams, useRouterState, useSearch } from "@tanstack/react-router";
import { Icon } from "@astryxdesign/core/Icon";
import { IconButton } from "@astryxdesign/core/IconButton";
import { Button } from "@astryxdesign/core/Button";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Text } from "@astryxdesign/core/Text";
import { DropdownMenu, type DropdownMenuOption } from "@astryxdesign/core/DropdownMenu";
import { HStack, StackItem } from "@astryxdesign/core/Stack";
import {
  IconLayoutSidebarLeftExpand,
  IconArrowLeft,
  IconArrowRight,
  IconSearch,
  IconDots,
  IconRefresh,
  IconSettings,
  IconBrandGithub,
} from "@tabler/icons-react";
import { useNavigationHistory } from "../layouts/use-navigation-history";
import { WindowToolbar } from "../layouts/WindowFrame";
import { getWindowLayout } from "../layouts/window-layout";
import { CatalogRefreshButton } from "../catalog/CatalogForms";
import { useWorkspaceRefresh } from "../cluster/WorkspaceRefresh";
import { RestartUpdateButton } from "../updates/RestartUpdateButton";
import { usePreferences } from "../preferences/PreferencesProvider";
import { StatusBadge } from "../cluster/StatusBadge";
import { useCatalog } from "../catalog/CatalogProvider";
import { RecordIcon } from "../catalog/RecordIcon";
import { clusterDestinations } from "../cluster/navigation";
import { hasProjectEngine, openProjectRepository, projectOpenError } from "../project/api";

function ClusterSwitcher({ clusterId }: { clusterId: string }) {
  const { snapshot, loading, error } = useCatalog();
  const navigate = useNavigate();
  const { section } = useSearch({ strict: false }) as { section?: string };
  const [query, setQuery] = useState("");
  const cluster = snapshot?.clusters.find(item => item.id === clusterId);
  const clusters = snapshot?.clusters.filter(item => item.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())) ?? [];
  const safeSection = clusterDestinations.find(item => item.id === section)?.id ?? "overview";
  return <DropdownMenu presentation="popover" hasChevron={false} menuWidth="max-content"
    onOpenChange={() => setQuery("")}
    renderTrigger={props => <Button {...props} className="toolbar-cluster-switcher" label="Switch cluster" aria-label="Switch cluster"
      tooltip={`Switch cluster · ${cluster?.name ?? "Cluster unavailable"}`} variant="ghost"
      icon={cluster ? <RecordIcon kind="Clusters" color={cluster.color} /> : <Icon icon={IconLayoutSidebarLeftExpand} />}>
      <Text className="toolbar-cluster-name" maxLines={1}>{cluster?.name ?? "Switch cluster"}</Text>
    </Button>}
    header={<TextInput label="Find cluster" isLabelHidden value={query} onChange={setQuery} placeholder="Find cluster" size="sm" />}
    items={clusters.length ? clusters.map(item => ({ id: item.id, label: item.name,
      isDisabled: item.id === clusterId,
      onClick: () => {
        if (snapshot?.clusters.some(saved => saved.id === item.id)) void navigate({ to: "/cluster/$clusterId", params: { clusterId: item.id }, search: { section: safeSection } });
      },
    })) : [{ label: loading ? "Loading clusters…" : error ? "Cluster metadata unavailable" : query ? "No matching clusters" : "No saved clusters", isDisabled: true }]} />;
}

export function AppToolbar({ onSearch }: { onSearch: () => void }) {
  const history = useNavigationHistory();
  const [repositoryBusy, setRepositoryBusy] = useState(false);
  const [repositoryError, setRepositoryError] = useState<string | null>(null);
  const projectAvailable = hasProjectEngine();
  async function openRepository() {
    if (repositoryBusy) return;
    setRepositoryBusy(true);
    setRepositoryError(null);
    try { await openProjectRepository(); }
    catch { setRepositoryError(projectOpenError); }
    finally { setRepositoryBusy(false); }
  }
  const layout = getWindowLayout();
  const draggable = layout.seamless && layout.platform !== "browser";
  const isCatalog = useRouterState({ select: (state) => state.location.pathname.startsWith("/catalog") });
  const isSecrets = useRouterState({ select: (state) => state.location.pathname === "/secrets" });
  const workspaceRefresh = useWorkspaceRefresh();
  const { openPreferences } = usePreferences();
  const { clusterId } = useParams({ strict: false });
  const refresh = workspaceRefresh.action?.clusterId === clusterId ? workspaceRefresh.action : null;
  const refreshWorkspace = useCallback(() => {
    if (!refresh?.enabled || !clusterId) return;
    workspaceRefresh.refreshClusterCaches(clusterId);
    refresh.refresh();
  }, [refresh, clusterId, workspaceRefresh.refreshClusterCaches]);
  const catalog = useCatalog();
  const clusterActions = workspaceRefresh.clusterActions?.clusterId === clusterId ? workspaceRefresh.clusterActions : null;
  const pageActions = workspaceRefresh.pageActions && workspaceRefresh.pageActions.clusterId === clusterId && workspaceRefresh.pageActions.section === refresh?.section ? workspaceRefresh.pageActions : null;
  const status = clusterActions?.status;
  const statusColor = status === "Connected" ? "success" : status === "Connection error" ? "error" : status === "Connecting…" ? "warning" : "secondary";
  const items: DropdownMenuOption[] = [
    ...(status ? [{ label: status, isDisabled: true }, { type: "divider" as const }] : []),
    ...(pageActions ? [
      { label: "Previous page", icon: IconArrowLeft, isDisabled: !pageActions.previous.enabled, onClick: pageActions.previous.run },
      { label: "Next page", icon: IconArrowRight, isDisabled: !pageActions.next.enabled, onClick: pageActions.next.run },
      ...(pageActions.live ? [{ label: pageActions.live.checked ? "Pause live sync" : "Enable live sync", isDisabled: !pageActions.live.enabled, onClick: pageActions.live.run }] : []),
    ] : []),
    ...(clusterActions ? [{ type: "divider" as const }, ...clusterActions.items.map(item => ({ label: item.label, isDisabled: item.enabled === false, onClick: item.run }))] : []),
  ];
  const compactItems = [
    { label: "Search", icon: IconSearch, onClick: onSearch },
    { label: "Preferences", icon: IconSettings, onClick: openPreferences },
  ];
  useEffect(() => {
    function onRefresh(event: KeyboardEvent) {
      if (event.defaultPrevented) return;
      if ((event.metaKey !== event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "r" && !event.isComposing && (refresh || isCatalog)) {
        event.preventDefault();
        if (event.repeat) return;
        if (isCatalog) {
          if (catalog.available && !catalog.busy && !catalog.loading) void catalog.run("catalog_list");
        } else refreshWorkspace();
      }
    }
    window.addEventListener("keydown", onRefresh);
    return () => window.removeEventListener("keydown", onRefresh);
  }, [refresh, refreshWorkspace, isCatalog, catalog.available, catalog.busy, catalog.loading, catalog.run]);

  return (
    <>
    <WindowToolbar {...layout}>
      <HStack
        gap={1}
        paddingInline={1}
        vAlign="center"
        className="toolbar-history"
        style={{ flexShrink: 0 }}
      >
        <IconButton
          label="Go back"
          tooltip="Go back"
          variant="ghost"
          icon={<Icon icon={IconArrowLeft} />}
          isDisabled={!history.canGoBack}
          onClick={history.back}
        />
        <IconButton
          label="Go forward"
          tooltip="Go forward"
          variant="ghost"
          icon={<Icon icon={IconArrowRight} />}
          isDisabled={!history.canGoForward}
          onClick={history.forward}
        />
        {isCatalog && <CatalogRefreshButton />}
        {refresh && <IconButton label={`Refresh ${refresh.section}`} tooltip={`Refresh ${refresh.section}`}
          variant="ghost" icon={<Icon icon={IconRefresh} />} isDisabled={!refresh.enabled} onClick={refreshWorkspace} />}
      </HStack>
      {clusterId && <ClusterSwitcher clusterId={clusterId} />}
      {isSecrets && <Text weight="semibold">Secrets</Text>}
      <StackItem
        size="fill"
        aria-hidden="true"
        data-tauri-drag-region={draggable || undefined}
        className="toolbar-drag-space"
        style={{ alignSelf: "stretch", minWidth: 0 }}
      />
      <RestartUpdateButton />
      <IconButton
        label="Preferences"
        className="toolbar-secondary-action"
        tooltip="Preferences (Cmd+,)"
        variant="ghost"
        icon={<Icon icon={IconSettings} />}
        onClick={openPreferences}
      />
      <IconButton
        label="Search"
        className="toolbar-secondary-action"
        tooltip="Search"
        variant="ghost"
        icon={<Icon icon={IconSearch} />}
        onClick={onSearch}
        aria-keyshortcuts="Control+k Meta+k"
      />
      {status && <HStack className="cluster-connection-status"><StatusBadge status={status} variant={status === "Connection error" ? "error" : undefined} /></HStack>}
      <IconButton label="Open Iceon on GitHub" className="toolbar-github-action"
        tooltip={projectAvailable ? "Open Iceon on GitHub" : "GitHub repository opens in the desktop app only"}
        aria-description={!projectAvailable ? "GitHub repository opens in the desktop app only" : undefined}
        variant="ghost" icon={<Icon icon={IconBrandGithub} />}
        isDisabled={!projectAvailable || repositoryBusy} onClick={() => void openRepository()} />
      {clusterId && <DropdownMenu presentation="popover" alignment="end" hasChevron={false}
        renderTrigger={props => <IconButton {...props} label="Cluster actions" tooltip={status ? `Cluster actions · ${status}` : "Cluster actions"} aria-description={status ? `Connection status: ${status}` : undefined} icon={<Icon icon={IconDots} color={statusColor} />} variant="ghost" />}
        items={[...items, { type: "section", id: "compact-tools", items: compactItems }]} />}
      {!clusterId && <HStack className="toolbar-compact-actions"><DropdownMenu presentation="popover" alignment="end" hasChevron={false}
        renderTrigger={props => <IconButton {...props} label="Toolbar actions" tooltip="Toolbar actions" icon={<Icon icon={IconDots} />} variant="ghost" />}
        items={compactItems} /></HStack>}
    </WindowToolbar>
    {repositoryError && <Text role="alert">{repositoryError}</Text>}
    </>
  );
}
