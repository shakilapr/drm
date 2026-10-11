// Preload: exposes a narrow domain bridge only. No `exec`, no raw `fs`, no
// `ipcRenderer` leak (ui-arch §Desktop Shell). REQ: REQ-STUDIO-001.
import { contextBridge, ipcRenderer } from "electron";

contextBridge.exposeInMainWorld("drm", {
  request: (line: string): Promise<string> =>
    ipcRenderer.invoke("studio.request", line),
});
