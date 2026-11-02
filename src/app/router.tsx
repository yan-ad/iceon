import { createHashHistory, createRootRoute, createRoute, createRouter, redirect } from "@tanstack/react-router";
import MainLayout from "./layouts/MainLayout";
import CatalogPage from "./pages/CatalogPage";
import NotFoundPage from "./pages/NotFoundPage";
import { CatalogProvider } from "./catalog/CatalogProvider";
import ClusterWorkspacePage from "./cluster/ClusterWorkspacePage";
import { clusterDestinations, type ClusterSectionId } from "./cluster/navigation";
import { WorkspaceRefreshProvider } from "./cluster/WorkspaceRefresh";
import { CustomResourcesProvider } from "./cluster/CustomResourcesProvider";
import { validDefinitionName } from "./cluster/customResourcesApi";
import { RouteRestoration } from "./RouteRestoration";
import { PermissionProvider } from "./cluster/PermissionProvider";
import SecretsPage from "./pages/SecretsPage";

const rootRoute = createRootRoute({
  component: () => <CatalogProvider><RouteRestoration /><WorkspaceRefreshProvider><PermissionProvider><CustomResourcesProvider><MainLayout /></CustomResourcesProvider></PermissionProvider></WorkspaceRefreshProvider></CatalogProvider>,
  notFoundComponent: NotFoundPage,
});
const homeRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", beforeLoad: () => { throw redirect({ to: "/catalog", replace: true }); } });
const catalogRoute = createRoute({ getParentRoute: () => rootRoute, path: "catalog", validateSearch: (search: Record<string, unknown>): { item?: string } => ({
  item: typeof search.item === "string" && search.item.length <= 256 ? search.item : undefined,
}) });
const catalogIndexRoute = createRoute({ getParentRoute: () => catalogRoute, path: "/", component: CatalogPage });
const generalRoute = createRoute({ getParentRoute: () => catalogRoute, path: "general", beforeLoad: () => { throw redirect({ to: "/catalog/clusters", replace: true }); } });
const allRoute = createRoute({ getParentRoute: () => catalogRoute, path: "all", beforeLoad: () => { throw redirect({ to: "/catalog/clusters", replace: true }); } });
const clustersRoute = createRoute({ getParentRoute: () => catalogRoute, path: "clusters", component: CatalogPage });
const secretsRoute = createRoute({ getParentRoute: () => rootRoute, path: "secrets", component: SecretsPage });
const webLinksRoute = createRoute({ getParentRoute: () => catalogRoute, path: "web-links", beforeLoad: () => { throw redirect({ to: "/catalog/clusters", replace: true }); } });
const clusterWorkspaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "cluster/$clusterId",
  component: ClusterWorkspacePage,
  validateSearch: (search: Record<string, unknown>): { section?: ClusterSectionId | "custom-resource-instances"; definition?: string } => ({
    section: search.section === "custom-resource-instances" ? "custom-resource-instances" : clusterDestinations.find((item) => item.id === search.section)?.id,
    definition: search.section === "custom-resource-instances" && validDefinitionName(search.definition) ? search.definition : undefined,
  }),
});

export const router = createRouter({
  history: createHashHistory(),
  routeTree: rootRoute.addChildren([
    homeRoute,
    catalogRoute.addChildren([catalogIndexRoute, allRoute, generalRoute, clustersRoute, webLinksRoute]),
    clusterWorkspaceRoute,
    secretsRoute,
  ]),
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register { router: typeof router }
}
