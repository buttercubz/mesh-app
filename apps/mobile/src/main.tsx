// App entry (WP-18). Order matters: i18n catalogs, then features (screen registration),
// then the bootstrap, then the first render. No StrictMode (see app/App.tsx).
import "./index.css";
import "./app/i18n";
import "./app/features";

import { createRoot } from "react-dom/client";

import { App } from "./app/App";
import { boot } from "./app/boot";

boot();

// Layout audit hook (e2e/layout.spec.ts): dev server + `?simDebug=1` only. The whole
// branch, and with it the module, is dropped from production bundles.
if (import.meta.env.DEV && new URLSearchParams(window.location.search).get("simDebug") === "1") {
  void import("./app/layoutHarness").then((m) => m.installLayoutHarness());
}

const root = document.getElementById("root");
if (root === null) throw new Error("index.html has no #root element");
createRoot(root).render(<App />);
