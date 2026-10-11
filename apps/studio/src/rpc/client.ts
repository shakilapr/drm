// Typed JSON-RPC client over the stdio transport. The renderer holds no DRM
// logic: it sends domain commands and renders engine responses (REQ-STUDIO-003,
// REQ-STUDIO-011). Topic sample bytes never travel here (REQ-STUDIO-013).
import type { StudioRequest, StudioResponse } from "@drm/studio-protocol";

/** Transport abstraction shared by desktop stdio and (later) remote web. */
export interface StudioTransport {
  request(command: StudioRequest): Promise<StudioResponse>;
}

/** Desktop bridge exposed by the Electron preload script (narrow surface). */
export interface HostBridge {
  request(line: string): Promise<string>;
}

declare global {
  interface Window {
    drm?: HostBridge;
  }
}

/** Desktop stdio transport: one JSON line per request over the host bridge. */
export class StdioTransport implements StudioTransport {
  public async request(command: StudioRequest): Promise<StudioResponse> {
    const bridge = window.drm;
    if (!bridge) {
      throw new Error("studio host bridge unavailable");
    }
    const line = await bridge.request(JSON.stringify(command));
    return JSON.parse(line) as StudioResponse;
  }
}

/** Thin typed client wrapper used by queries and stores. */
export class JsonRpcClient {
  public constructor(private readonly transport: StudioTransport) {}

  public request(command: StudioRequest): Promise<StudioResponse> {
    return this.transport.request(command);
  }

  public handshake(clientVersion: string): Promise<StudioResponse> {
    return this.request({
      method: "hello",
      protocol_version: 1,
      client_version: clientVersion,
    });
  }
}
