// Electron main process: owns window lifecycle and the `drm studio --stdio`
// child process. It exposes only a narrow `request(line)` bridge to the
// sandboxed renderer (ui-arch §Desktop Shell). Node/Electron access is
// confined to this main process; the renderer has none (REQ-STUDIO-001,
// REQ-STUDIO-019). The packaged app bundles the `drm` executable under
// `resources/` (electron-builder `extraResources`).
import { spawn } from "node:child_process";
import { join } from "node:path";

import { BrowserWindow, app, ipcMain } from "electron";

/** Resolves the `drm` executable: bundled under `resources/` when packaged. */
function drmExecutable(): string {
  if (!app.isPackaged) {
    return "drm";
  }
  const bundled = join(process.resourcesPath, "drm");
  return process.platform === "win32" ? `${bundled}.exe` : bundled;
}

class StudioHostProcess {
  private readonly child = spawn(drmExecutable(), ["studio", "--stdio"], {
    stdio: ["pipe", "pipe", "inherit"],
  });

  private buffer = "";
  private readonly pending: Array<(line: string) => void> = [];

  public constructor() {
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (chunk: string) => {
      this.buffer += chunk;
      let index = this.buffer.indexOf("\n");
      while (index >= 0) {
        const line = this.buffer.slice(0, index);
        this.buffer = this.buffer.slice(index + 1);
        const resolve = this.pending.shift();
        resolve?.(line);
        index = this.buffer.indexOf("\n");
      }
    });
  }

  public request(line: string): Promise<string> {
    this.child.stdin.write(`${line}\n`);
    return new Promise((resolve) => {
      this.pending.push(resolve);
    });
  }
}

const host = new StudioHostProcess();

ipcMain.handle("studio.request", (_event, line: string) => host.request(line));

function createWindow(): void {
  const window = new BrowserWindow({
    width: 1280,
    height: 800,
    webPreferences: {
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
    },
  });
  void window.loadFile("dist/renderer/index.html");
}

app.whenReady().then(() => {
  createWindow();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});
