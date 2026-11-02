import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";

type RefreshAction = { clusterId: string; section: string; enabled: boolean; refresh: () => void };
type Registration = RefreshAction & { owner: symbol };
type PageRefreshAction = { scope: string; label: string; enabled: boolean; refresh: () => void };
type MenuAction = { label: string; enabled?: boolean; run: () => void };
type ClusterActions = { clusterId: string; section: string; status?: string; items: MenuAction[] };
type PageAction = { enabled: boolean; run: () => void };
type PageActions = { clusterId: string; section: string; previous: PageAction; next: PageAction; live?: PageAction & { checked: boolean } };
const Context = createContext<{
  action: RefreshAction | null; register: (action: RefreshAction) => () => void;
  pageRefresh: PageRefreshAction | null; registerPageRefresh: (action: PageRefreshAction) => () => void;
  clusterActions: ClusterActions | null; registerCluster: (action: ClusterActions) => () => void;
  pageActions: PageActions | null; registerPage: (action: PageActions) => () => void;
  registerClusterRefresh: (clusterId: string, refresh: () => void) => () => void;
  refreshClusterCaches: (clusterId: string) => void;
} | null>(null);

export function WorkspaceRefreshProvider({ children }: { children: ReactNode }) {
  const clusterRefresh = useRef(new Map<symbol, { clusterId: string; refresh: () => void }>());
  const registerClusterRefresh = useCallback((clusterId: string, refresh: () => void) => {
    const owner = Symbol();
    clusterRefresh.current.set(owner, { clusterId, refresh });
    return () => { clusterRefresh.current.delete(owner); };
  }, []);
  const refreshClusterCaches = useCallback((clusterId: string) => {
    for (const entry of clusterRefresh.current.values()) if (entry.clusterId === clusterId) entry.refresh();
  }, []);
  const [action, setAction] = useState<Registration | null>(null);
  const [pageRefreshes, setPageRefreshes] = useState<(PageRefreshAction & { owner: symbol })[]>([]);
  const registerPageRefresh = useCallback((next: PageRefreshAction) => {
    const owner = Symbol();
    setPageRefreshes(previous => [...previous, { ...next, owner }]);
    return () => setPageRefreshes(previous => previous.filter(entry => entry.owner !== owner));
  }, []);
  const [clusterActions, setClusterActions] = useState<(ClusterActions & { owner: symbol }) | null>(null);
  const [pageActions, setPageActions] = useState<(PageActions & { owner: symbol }) | null>(null);
  const register = useCallback((next: RefreshAction) => {
    const owner = Symbol();
    setAction({ ...next, owner });
    return () => setAction((previous) => previous?.owner === owner ? null : previous);
  }, []);
  const registerCluster = useCallback((next: ClusterActions) => {
    const owner = Symbol(); setClusterActions({ ...next, owner });
    return () => setClusterActions(previous => previous?.owner === owner ? null : previous);
  }, []);
  const registerPage = useCallback((next: PageActions) => {
    const owner = Symbol(); setPageActions({ ...next, owner });
    return () => setPageActions(previous => previous?.owner === owner ? null : previous);
  }, []);
  const pageRefresh = pageRefreshes[pageRefreshes.length - 1] ?? null;
  return <Context value={{ action, register, pageRefresh, registerPageRefresh, clusterActions, registerCluster, pageActions, registerPage, registerClusterRefresh, refreshClusterCaches }}>{children}</Context>;
}

export function useRegisterPageRefresh(action: PageRefreshAction, active = true) {
  const { registerPageRefresh } = useWorkspaceRefresh();
  const callback = useRef(action.refresh);
  callback.current = action.refresh;
  useEffect(() => active ? registerPageRefresh({ ...action, refresh: () => callback.current() }) : undefined,
    [registerPageRefresh, action.scope, action.label, action.enabled, active]);
}

export function useRegisterClusterRefresh(clusterId: string, refresh: () => void) {
  const { registerClusterRefresh } = useWorkspaceRefresh();
  useEffect(() => registerClusterRefresh(clusterId, refresh), [registerClusterRefresh, clusterId, refresh]);
}

export function useRegisterClusterActions(action: ClusterActions) {
  const { registerCluster } = useWorkspaceRefresh();
  const current = useRef(action); current.current = action;
  const state = JSON.stringify(action.items.map(({ label, enabled }) => [label, enabled]));
  useEffect(() => registerCluster({ ...action, items: action.items.map((item, index) => ({ ...item, run: () => current.current.items[index]?.run() })) }),
    [registerCluster, action.clusterId, action.section, action.status, state]);
}

export function useRegisterWorkspacePageActions(action: PageActions) {
  const { registerPage } = useWorkspaceRefresh();
  const current = useRef(action); current.current = action;
  useEffect(() => registerPage({ ...action,
    previous: { ...action.previous, run: () => current.current.previous.run() },
    next: { ...action.next, run: () => current.current.next.run() },
    live: action.live ? { ...action.live, run: () => current.current.live?.run() } : undefined,
  }), [registerPage, action.clusterId, action.section, action.previous.enabled, action.next.enabled, action.live?.enabled, action.live?.checked]);
}
export function useWorkspaceRefresh() {
  const context = useContext(Context);
  if (!context) throw new Error("WorkspaceRefreshProvider is required");
  return context;
}
export function useRegisterWorkspaceRefresh(action: RefreshAction, active = true) {
  const { register } = useWorkspaceRefresh();
  const callback = useRef(action.refresh);
  callback.current = action.refresh;
  useEffect(() => active ? register({ ...action, refresh: () => callback.current() }) : undefined, [register, action.clusterId, action.section, action.enabled, active]);
}
