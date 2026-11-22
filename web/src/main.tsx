import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyThemeToDocument, loadStoredTheme } from "./lib/theme";
import "./styles/app.css";

applyThemeToDocument(loadStoredTheme());

const rootElement = document.getElementById("root");
if (rootElement) {
  createRoot(rootElement).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
