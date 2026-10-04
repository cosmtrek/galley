// A tiny pathname router; the workbench has only a handful of pages.

export type Route =
  | { name: "reports" }
  | { name: "workbench"; id: string }
  | { name: "verify"; id: string; round: string | null }
  | { name: "history"; id: string }
  | { name: "publish"; id: string }
  | { name: "notfound" };

function parse(pathname: string, search: string): Route {
  const p = pathname.replace(/\/+$/, "");
  if (p === "/app" || p === "") return { name: "reports" };
  const m = p.match(/^\/app\/r\/([^/]+)(?:\/(verify|history|publish))?$/);
  if (!m) return { name: "notfound" };
  const id = decodeURIComponent(m[1]);
  switch (m[2]) {
    case "verify":
      return { name: "verify", id, round: new URLSearchParams(search).get("round") };
    case "history":
      return { name: "history", id };
    case "publish":
      return { name: "publish", id };
    default:
      return { name: "workbench", id };
  }
}

export const router = $state({ route: parse(location.pathname, location.search), hash: location.hash });

export function navigate(href: string) {
  history.pushState({}, "", href);
  sync();
  window.scrollTo(0, 0);
}

function sync() {
  router.route = parse(location.pathname, location.search);
  router.hash = location.hash;
}

window.addEventListener("popstate", sync);

document.addEventListener("click", (e) => {
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  const a = (e.target as Element | null)?.closest?.("a");
  if (!a || a.target || a.hasAttribute("download")) return;
  const href = a.getAttribute("href");
  if (!href || !href.startsWith("/app")) return;
  e.preventDefault();
  navigate(href);
});
