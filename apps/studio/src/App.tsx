// Minimal boot UI: prove the pipe by rendering a workspace snapshot. All
// semantics come from the Rust engine (REQ-STUDIO-003).
import { useEffect, useState } from "react";

import { JsonRpcClient, StdioTransport } from "./rpc/client";

const client = new JsonRpcClient(new StdioTransport());

/** Boot screen: opens the workspace and shows the canonical model render. */
export function App(): React.JSX.Element {
  const [model, setModel] = useState<string>("(loading)");

  useEffect(() => {
    let cancelled = false;
    const boot = async (): Promise<void> => {
      await client.handshake("1.0.0");
      const opened = await client.request({
        method: "open_workspace",
        path: ".",
      });
      if (!cancelled && opened.result === "snapshot") {
        setModel(opened.model);
      }
    };
    void boot();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main>
      <h1>DRM Studio</h1>
      <pre data-testid="workspace-model">{model}</pre>
    </main>
  );
}
