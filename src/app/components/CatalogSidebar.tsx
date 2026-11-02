import { useRouterState } from "@tanstack/react-router";
import { Divider } from "@astryxdesign/core/Divider";
import { SideNav, SideNavItem, SideNavSection } from "@astryxdesign/core/SideNav";
import { Text } from "@astryxdesign/core/Text";
import { IconMenu2, IconPin } from "@tabler/icons-react";
import { destinations } from "../navigation";
import { useCatalog } from "../catalog/CatalogProvider";
import { RecordIcon } from "../catalog/RecordIcon";
import { pinKey } from "../catalog/api";
import { usePinnedItems } from "../catalog/usePinnedItems";
import { NavLink } from "./NavLink";
import { useAppShellMobile } from "@astryxdesign/core/AppShell";
import { clusterDestinations } from "../cluster/navigation";

export function CatalogSidebar({ onNavigate, id = "catalog-sidebar" }: { onNavigate: () => void; id?: string }) {
  const mobile = useAppShellMobile();
  const pathname = useRouterState({ select: state => state.location.pathname });
  const { snapshot, loading } = useCatalog();
  const activeClusterId = useRouterState({ select: state => state.matches.find(match => "clusterId" in match.params)?.params as { clusterId?: string } | undefined });
  const activeSection = useRouterState({ select: state => (state.location.search as { section?: string }).section ?? "overview" });
  const pins = usePinnedItems();

  return <>
    <SideNav aria-label="Catalog navigation" id={id} className="catalog-sidebar-motion" data-collapsed="true"
      collapsible={{ isCollapsed: true, hasButton: false }}
      style={{ width: "var(--spacing-12)", minWidth: "var(--spacing-12)", minHeight: 0, flexShrink: 0 }}>
      {mobile.isMobile && id === "catalog-sidebar-narrow" && <SideNavSection title="Navigation">
        <SideNavItem label="Open sidebar" icon={IconMenu2} aria-expanded={mobile.isMobileNavOpen}
          aria-controls={mobile.mobileNavId} onClick={mobile.openMobileNav} />
      </SideNavSection>}
      <SideNavSection title="Navigation">
        {destinations.map(category => <NavLink key={category.path} to={category.path} label={category.label} icon={category.icon}
           isSelected={pathname === category.path || (category.path === "/catalog/clusters" && pathname === "/catalog")} onClick={onNavigate} />)}
      </SideNavSection>
      <Divider />
      <SideNavSection title="Pinned items">
        {!pins.items.length && <SideNavItem label={loading ? "Loading pinned items…" : "No pinned items"} icon={IconPin} isDisabled />}
        {pins.items.map(target => {
          const cluster = snapshot?.clusters.find(item => item.id === target.id);
          const destination = target.section ? clusterDestinations.find(item => item.id === target.section) : null;
          const handlers = pins.handlers(target);
          if (cluster) return <NavLink key={pinKey(target)} {...handlers}
            to="/cluster/$clusterId" params={{ clusterId: cluster.id }} search={target.section ? { section: target.section } : {}}
            label={destination ? `${cluster.name} · ${destination.label}` : cluster.name}
            icon={<RecordIcon kind="Clusters" color={cluster.color} icon={destination?.icon} />}
            isSelected={activeClusterId?.clusterId === cluster.id && activeSection === (target.section ?? "overview")}
            onClick={event => { if (handlers.onClick(event)) onNavigate(); }} />;
          return null;
        })}
      </SideNavSection>
    </SideNav>
    {pins.error && <Text role="alert" className="pin-menu-error">{pins.error}</Text>}
  </>;
}
