export type RouteKey =
  | "overview"
  | "inventory"
  | "templates"
  | "artifacts"
  | "harbor"
  | "operations"
  | "pki"
  | "login";

export interface RouteEntry {
  key: RouteKey;
  path: string;
  label: string;
  requiresAuth: boolean;
  showInSidebar: boolean;
}

export const ROUTE_REGISTRY: readonly RouteEntry[] = [
  {
    key: "overview",
    path: "/",
    label: "概览",
    requiresAuth: true,
    showInSidebar: true,
  },
  {
    key: "inventory",
    path: "/inventory",
    label: "集群配置",
    requiresAuth: true,
    showInSidebar: true,
  },
  {
    key: "templates",
    path: "/templates",
    label: "模板预览",
    requiresAuth: true,
    showInSidebar: true,
  },
  {
    key: "artifacts",
    path: "/artifacts",
    label: "离线资源",
    requiresAuth: true,
    showInSidebar: true,
  },
  {
    key: "harbor",
    path: "/harbor",
    label: "私库管理",
    requiresAuth: true,
    showInSidebar: true,
  },
  {
    key: "operations",
    path: "/operations",
    label: "编排执行",
    requiresAuth: true,
    showInSidebar: true,
  },
  {
    key: "pki",
    path: "/pki",
    label: "证书备份",
    requiresAuth: true,
    showInSidebar: true,
  },
  {
    key: "login",
    path: "/login",
    label: "登录",
    requiresAuth: false,
    showInSidebar: false,
  },
] as const;

export const SIDEBAR_ROUTES: readonly RouteEntry[] = ROUTE_REGISTRY.filter((r) => r.showInSidebar);

export function resolveRouteByPath(pathname: string): RouteEntry {
  const normalized = pathname.length > 1 ? pathname.replace(/\/+$/, "") : pathname;
  const match = ROUTE_REGISTRY.find((entry) => entry.path === normalized);
  return match ?? ROUTE_REGISTRY[0]!;
}
