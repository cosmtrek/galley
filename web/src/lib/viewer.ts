// Full-screen image viewer with zoom and pan. Framework-free so the share page can load it as a
// standalone script; the workbench opens it from its article click handler.
import { PATHS, type IconName } from "./icons";

const PAD = 16;
const STEP = 1.25;
const MAX = 8;
const TAP_SLOP = 6;
const DOUBLE_TAP_MS = 300;

type Pt = { x: number; y: number };

/** Linked images keep following their link instead of opening the viewer. */
export function zoomable(t: EventTarget | null): t is HTMLImageElement {
  return t instanceof HTMLImageElement && !t.closest("a");
}

function icon(name: IconName) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS(ns, "path");
  path.setAttribute("d", PATHS[name]);
  svg.append(path);
  return svg;
}

function control<K extends "button" | "a">(tag: K, label: string, name?: IconName): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  el.className = "zoom-btn";
  el.title = label;
  el.setAttribute("aria-label", label);
  if (el instanceof HTMLButtonElement) el.type = "button";
  if (name) el.append(icon(name));
  return el;
}

export function openViewer(source: HTMLImageElement) {
  const dlg = document.createElement("dialog");
  dlg.className = "zoom";
  dlg.setAttribute("aria-label", source.alt || "图片");
  const stage = document.createElement("div");
  stage.className = "zoom-stage";
  const img = document.createElement("img");
  img.className = "zoom-img";
  img.alt = source.alt;
  img.draggable = false;
  img.src = source.currentSrc || source.src;
  stage.append(img);

  const bar = document.createElement("div");
  bar.className = "zoom-bar";
  const outBtn = control("button", "缩小（-）", "zoom-out");
  const pct = control("button", "适应窗口（0）");
  pct.classList.add("zoom-pct");
  const inBtn = control("button", "放大（+）", "zoom-in");
  const closeBtn = control("button", "关闭（Esc）", "x");
  bar.append(outBtn, pct, inBtn);
  // Browsers refuse to navigate to data: URLs, so inline diagrams get no "open original".
  if (!img.src.startsWith("data:")) {
    const open = control("a", "在新标签页打开原图", "external");
    open.href = img.src;
    open.target = "_blank";
    open.rel = "noopener noreferrer";
    bar.append(open);
  }
  bar.append(closeBtn);
  dlg.append(stage, bar);

  // Vector images stay sharp at any size, so they may be fitted above their nominal size.
  const vector = /^data:image\/svg|\.svg([?#]|$)/i.test(img.src);
  let nw = 0;
  let nh = 0;
  let fit = 1;
  let s = 1;
  let x = 0;
  let y = 0;

  // The area above the toolbar; images are centered in it and may be panned under the toolbar.
  const view = () => ({ w: stage.clientWidth, h: stage.clientHeight - bar.offsetHeight - 2 * PAD });
  const lo = () => Math.min(fit, 1);
  const hi = () => Math.max(MAX, fit * 2);

  function clampPan() {
    const v = view();
    const w = nw * s;
    const h = nh * s;
    x = w <= v.w ? (v.w - w) / 2 : Math.min(0, Math.max(v.w - w, x));
    y = h <= v.h ? (v.h - h) / 2 : Math.min(0, Math.max(v.h - h, y));
  }

  function render() {
    const v = view();
    img.style.width = `${nw * s}px`;
    img.style.height = `${nh * s}px`;
    img.style.transform = `translate(${x}px, ${y}px)`;
    img.style.visibility = "visible";
    pct.textContent = `${Math.round(s * 100)}%`;
    outBtn.disabled = s <= lo() * 1.001;
    inBtn.disabled = s >= hi() * 0.999;
    stage.classList.toggle("pannable", nw * s > v.w + 0.5 || nh * s > v.h + 0.5);
  }

  function zoomTo(next: number, cx: number, cy: number) {
    const ns = Math.min(hi(), Math.max(lo(), next));
    x = cx - ((cx - x) * ns) / s;
    y = cy - ((cy - y) * ns) / s;
    s = ns;
    clampPan();
    render();
  }

  function reset() {
    const v = view();
    const k = Math.min((v.w - 2 * PAD) / nw, (v.h - PAD) / nh);
    fit = Math.max(0.01, vector ? k : Math.min(1, k));
    s = fit;
    clampPan();
    render();
  }

  const atFit = () => Math.abs(s - fit) < fit * 0.01;
  const center = (): [number, number] => [view().w / 2, view().h / 2];
  const toggle = (cx: number, cy: number) => (atFit() ? zoomTo(fit < 0.99 && !vector ? 1 : fit * 2, cx, cy) : reset());

  function ready() {
    const r = source.getBoundingClientRect();
    // SVGs without width/height report no natural size; fall back to how big they were on the page.
    nw = img.naturalWidth || r.width;
    nh = img.naturalHeight || r.height;
    if (nw && nh) reset();
  }

  const local = (e: { clientX: number; clientY: number }): Pt => {
    const r = stage.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  // ----- pointer: drag to pan, pinch to zoom, double tap to toggle, tap outside to close -----

  const pts = new Map<number, Pt>();
  let downAt: Pt = { x: 0, y: 0 };
  let moved = false;
  // Pointer capture retargets later events to the stage, so remember where the press started.
  let downOnImg = false;
  let lastTap = { t: 0, x: 0, y: 0 };

  stage.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    stage.setPointerCapture(e.pointerId);
    pts.set(e.pointerId, local(e));
    if (pts.size === 1) {
      downAt = local(e);
      downOnImg = e.target === img;
      moved = false;
    } else moved = true;
  });

  stage.addEventListener("pointermove", (e) => {
    const prev = pts.get(e.pointerId);
    if (!prev) return;
    const p = local(e);
    if (pts.size === 1) {
      if (!moved && Math.hypot(p.x - downAt.x, p.y - downAt.y) < TAP_SLOP) return;
      moved = true;
      stage.classList.add("dragging");
      pts.set(e.pointerId, p);
      x += p.x - prev.x;
      y += p.y - prev.y;
      clampPan();
      render();
    } else if (pts.size === 2) {
      const [a0, b0] = [...pts.values()];
      pts.set(e.pointerId, p);
      const [a1, b1] = [...pts.values()];
      const d0 = Math.hypot(a0.x - b0.x, a0.y - b0.y);
      const d1 = Math.hypot(a1.x - b1.x, a1.y - b1.y);
      const mx = (a1.x + b1.x) / 2;
      const my = (a1.y + b1.y) / 2;
      x += mx - (a0.x + b0.x) / 2;
      y += my - (a0.y + b0.y) / 2;
      if (d0 > 0) zoomTo((s * d1) / d0, mx, my);
    }
  });

  function onUp(e: PointerEvent) {
    if (!pts.delete(e.pointerId)) return;
    if (!pts.size) stage.classList.remove("dragging");
    if (e.type === "pointercancel" || moved || pts.size) return;
    if (!downOnImg) {
      dlg.close();
      return;
    }
    const p = local(e);
    const now = performance.now();
    if (now - lastTap.t < DOUBLE_TAP_MS && Math.hypot(p.x - lastTap.x, p.y - lastTap.y) < 24) {
      lastTap.t = 0;
      toggle(p.x, p.y);
    } else lastTap = { t: now, ...p };
  }
  stage.addEventListener("pointerup", onUp);
  stage.addEventListener("pointercancel", onUp);

  stage.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      const dy = e.deltaY * (e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1);
      const p = local(e);
      // Trackpad pinches arrive as ctrl+wheel with small deltas.
      zoomTo(s * Math.exp(-dy * (e.ctrlKey ? 0.01 : 0.002)), p.x, p.y);
    },
    { passive: false },
  );

  // ----- toolbar and keys -----

  outBtn.addEventListener("click", () => zoomTo(s / STEP, ...center()));
  inBtn.addEventListener("click", () => zoomTo(s * STEP, ...center()));
  pct.addEventListener("click", reset);
  closeBtn.addEventListener("click", () => dlg.close());

  dlg.addEventListener("keydown", (e) => {
    // The page behind has its own shortcuts (the workbench's M, Escape for popovers).
    e.stopPropagation();
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "+" || e.key === "=") zoomTo(s * STEP, ...center());
    else if (e.key === "-" || e.key === "_") zoomTo(s / STEP, ...center());
    else if (e.key === "0") reset();
    else return;
    e.preventDefault();
  });

  const onResize = () => {
    if (!nw) return;
    if (atFit()) reset();
    else {
      clampPan();
      render();
    }
  };
  window.addEventListener("resize", onResize);
  dlg.addEventListener("close", () => {
    window.removeEventListener("resize", onResize);
    dlg.remove();
  });

  img.style.visibility = "hidden";
  document.body.append(dlg);
  dlg.showModal();
  // Focus the dialog itself rather than the first toolbar button, which would open with a focus ring.
  dlg.tabIndex = -1;
  dlg.focus({ preventScroll: true });
  if (img.complete && img.naturalWidth) ready();
  else img.addEventListener("load", ready, { once: true });
}
