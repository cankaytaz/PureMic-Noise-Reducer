import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import "./index.css";

// Catch unhandled promise rejections
window.addEventListener("unhandledrejection", (e) => {
  const reason = e.reason;
  const message = reason?.message || reason || "Unknown error";
  const stack = reason?.stack || "";
  
  // Specific handling for "Connection Refused" (os error 61)
  // This is often a harmless side-effect of the browser hitting the OAuth port after it closes.
  if (String(message).includes("os error 61")) {
    console.warn("ℹ️ Background Connection Ignored (os error 61). This is likely a browser favicon/retry request to a closed OAuth port.");
    return;
  }

  console.group("🔴 Unhandled Promise Rejection");
  console.error(reason);
  console.log("Stack Trace:", stack);
  console.groupEnd();

  // For other fatal errors, show full screen to prevent silent UI hangs
  document.body.innerHTML = `
    <div id="crash-screen" style="background:#09090b;color:#f4f4f5;height:100vh;padding:40px;font-family:monospace;display:flex;flex-direction:column;gap:20px">
      <h1 style="color:#ef4444;font-size:18px;margin:0">Unhandled Exception</h1>
      <div style="background:#18181b;padding:20px;border-radius:8px;border:1px solid #27272a;overflow:auto">
        <pre style="margin:0;color:#ef4444;font-weight:bold">${message}</pre>
        <pre style="margin-top:20px;font-size:11px;color:#a1a1aa;white-space:pre-wrap">${stack}</pre>
      </div>
      <button onclick="location.reload()" style="background:#ef4444;color:white;border:none;padding:8px 16px;border-radius:4px;cursor:pointer;width:fit-content">Reload App</button>
    </div>
  `;
});

// Helper to clear error markers from other components
(window as any).clearGlobalErrors = () => {
  const bars = document.querySelectorAll('[id^="error-bar-"]');
  bars.forEach(bar => bar.remove());
};

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
);
