import React from "react";

interface State {
  error: string | null;
}

export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  State
> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error: `${error.name}: ${error.message}\n${error.stack}` };
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 20, color: "#ff6b6b", fontFamily: "monospace", fontSize: 12, whiteSpace: "pre-wrap" }}>
          <h2 style={{ color: "#ff4444" }}>UI Error</h2>
          <p>{this.state.error}</p>
        </div>
      );
    }
    return this.props.children;
  }
}
