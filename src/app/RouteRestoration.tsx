import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { loadRouteHistory, saveRouteHistory, type SavedRoute } from "./preferences/api";
import { usePreferences } from "./preferences/PreferencesProvider";
import { useCatalog } from "./catalog/CatalogProvider";
import { clusterDestinations } from "./cluster/navigation";
import { validDefinitionName } from "./cluster/customResourcesApi";
import { getWindowLayout } from "./layouts/window-layout";

const initialHash = typeof window === "undefined" ? "" : window.location.hash;

function validRoute(value: unknown): value is SavedRoute {
  if (!value || typeof value !== "object") return false;
  const route = value as SavedRoute;
  if (Object.keys(route).some(key => !["path", "section", "definition"].includes(key))) return false;
  if (typeof route.path !== "string") return false;
  if (["/catalog", "/catalog/all", "/catalog/clusters", "/secrets"].includes(route.path)) return !route.section && !route.definition;
  if (!/^\/cluster\/[a-zA-Z0-9_-]{1,128}$/.test(route.path)) return false;
  if (route.section === "custom-resource-instances") return validDefinitionName(route.definition);
  return !route.definition && (!route.section || clusterDestinations.some(item => item.id === route.section));
}

export function RouteRestoration() {
  const navigate = useNavigate();
  const location = useRouterState({ select: s => s.location });
  const { preferences, preferencesReady } = usePreferences();
  const { snapshot, loading } = useCatalog();
  const [settled, setSettled] = useState(false);
  const currentLocation = useRef(location.href);
  currentLocation.current = location.href;
  const queue = useRef<Promise<void>>(Promise.resolve());
  const native = isTauri() && getWindowLayout().platform !== "browser";

  useEffect(() => {
    if (settled || !preferencesReady || loading || !snapshot) return;
    let disposed = false;
    const startLocation = location.href;
    // Only the launch window's empty route is eligible. Explicit navigation wins.
    const startup = (window as unknown as { __ICEON_WINDOW_LAYOUT__?: { restoreRoute?: boolean } }).__ICEON_WINDOW_LAYOUT__;
    if (!native || !preferences.app.restoreLastPage || startup?.restoreRoute === false || !["", "#", "#/"].includes(initialHash) || !["/", "/catalog"].includes(location.pathname)) {
      setSettled(true);
      return;
    }
    void loadRouteHistory().then(async route => {
      if (disposed || currentLocation.current !== startLocation) return;
      if (!validRoute(route)) return;
      if (route.path.startsWith("/cluster/") && !snapshot.clusters.some(c => c.id === route.path.slice(9))) return;
      await navigate({ to: route.path as never, search: route.section ? { section: route.section, ...(route.definition ? { definition: route.definition } : {}) } as never : {} as never, replace: true });
    }).catch(() => {
      console.error("Could not restore the last page.");
    }).finally(() => { if (!disposed) setSettled(true); });
    return () => { disposed = true; };
  }, [settled, preferencesReady, loading, snapshot, native, preferences.app.restoreLastPage, navigate, location.href, location.pathname]);

  useEffect(() => {
    if (!settled || !native || !preferencesReady || !preferences.app.restoreLastPage || !snapshot) return;
    const route: SavedRoute = { path: location.pathname, section: typeof location.search.section === "string" ? location.search.section : undefined, definition: typeof location.search.definition === "string" ? location.search.definition : undefined };
    if (!validRoute(route)) return;
    const timer = window.setTimeout(() => {
      queue.current = queue.current.catch(() => {}).then(() => saveRouteHistory(route)).catch(() => {
        console.error("Could not remember the last page.");
      });
    }, 200);
    return () => window.clearTimeout(timer);
  }, [settled, native, preferencesReady, preferences.app.restoreLastPage, snapshot, location.pathname, location.search]);
  return null;
}
