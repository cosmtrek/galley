// The only script on the public share page (built to dist/share.js by vite.share.config.ts).
// It must stay self-contained: the page's CSP admits this one file by nonce and nothing else.
import { openViewer, zoomable } from "./lib/viewer";

const article = document.querySelector<HTMLElement>("article.report");
if (article) {
  article.classList.add("zoomable");
  article.addEventListener("click", (e) => {
    if (zoomable(e.target)) openViewer(e.target);
  });
}
