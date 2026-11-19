import * as vscode from "vscode";
import * as cp from "child_process";
import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";
import { assetName } from "./assets";

const REPO = "bolu-atx/gh-ghit";
const TERMINAL_NAME = "ghit";
const exe = process.platform === "win32" ? "ghit.exe" : "ghit";

function onPath(cmd: string): boolean {
  const r = cp.spawnSync(process.platform === "win32" ? "where" : "which", [cmd], { stdio: "ignore" });
  return r.status === 0;
}

function pathOnPath(): string | undefined {
  return onPath("ghit") ? "ghit" : undefined;
}

function downloaded(ctx: vscode.ExtensionContext): string | undefined {
  const p = path.join(ctx.globalStorageUri.fsPath, exe);
  return fs.existsSync(p) ? p : undefined;
}

async function getJson(url: string): Promise<any> {
  const res = await fetch(url, { headers: { "User-Agent": "ghit-vscode", Accept: "application/vnd.github+json" } });
  if (!res.ok) throw new Error(`GET ${url} failed: ${res.status}`);
  return res.json();
}

async function install(ctx: vscode.ExtensionContext): Promise<string | undefined> {
  try {
    const name = assetName(process.platform, process.arch);
    return await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: "Downloading ghit", cancellable: false },
      async (progress) => {
        const rel = await getJson(`https://api.github.com/repos/${REPO}/releases/latest`);
        const assets: { name: string; browser_download_url: string; size: number }[] = rel.assets ?? [];
        const bin = assets.find((a) => a.name === name);
        const sums = assets.find((a) => a.name === "SHA256SUMS");
        if (!bin) throw new Error(`Release ${rel.tag_name} has no asset ${name}.`);
        if (!sums) throw new Error(`Release ${rel.tag_name} has no SHA256SUMS.`);

        const sumsText = await (await fetch(sums.browser_download_url)).text();
        const line = sumsText.split(/\r?\n/).map((l) => l.trim().split(/\s+/)).find((p) => p[1]?.replace(/^\*/, "") === name);
        if (!line) throw new Error(`SHA256SUMS has no entry for ${name}.`);

        const res = await fetch(bin.browser_download_url);
        if (!res.ok || !res.body) throw new Error(`Download failed: ${res.status}`);
        const chunks: Buffer[] = [];
        let got = 0;
        let last = 0;
        for await (const chunk of res.body as any) {
          const b = Buffer.from(chunk);
          chunks.push(b);
          got += b.length;
          const pct = Math.floor((got / bin.size) * 100);
          if (pct > last) {
            progress.report({ increment: pct - last, message: `${pct}%` });
            last = pct;
          }
        }
        const data = Buffer.concat(chunks);
        const actual = crypto.createHash("sha256").update(data).digest("hex");
        if (actual !== line[0].toLowerCase()) throw new Error("SHA256 mismatch; the download was discarded.");

        fs.mkdirSync(ctx.globalStorageUri.fsPath, { recursive: true });
        const dest = path.join(ctx.globalStorageUri.fsPath, exe);
        fs.writeFileSync(dest, data);
        if (process.platform !== "win32") fs.chmodSync(dest, 0o755);
        if (process.platform === "darwin") {
          cp.spawnSync("xattr", ["-d", "com.apple.quarantine", dest], { stdio: "ignore" });
        }
        vscode.window.showInformationMessage(`Installed ghit ${rel.tag_name}.`);
        return dest;
      }
    );
  } catch (e) {
    vscode.window.showErrorMessage(`ghit install failed: ${(e as Error).message}`);
    return undefined;
  }
}

async function resolve(ctx: vscode.ExtensionContext): Promise<string | undefined> {
  const configured = vscode.workspace.getConfiguration("ghit").get<string>("path");
  if (configured) return configured;
  const found = pathOnPath() ?? downloaded(ctx);
  if (found) return found;
  const pick = await vscode.window.showInformationMessage("ghit not found. Download latest release?", "Download");
  return pick ? install(ctx) : undefined;
}

async function open(ctx: vscode.ExtensionContext) {
  const existing = vscode.window.terminals.find((t) => t.name === TERMINAL_NAME && t.exitStatus === undefined);
  if (existing) {
    existing.show();
    return;
  }
  const bin = await resolve(ctx);
  if (!bin) return;
  const editor = vscode.workspace.getConfiguration("ghit").get<string>("location") === "editor";
  const term = vscode.window.createTerminal({
    name: TERMINAL_NAME,
    shellPath: bin,
    cwd: vscode.workspace.workspaceFolders?.[0]?.uri.fsPath,
    location: editor ? vscode.TerminalLocation.Editor : vscode.TerminalLocation.Panel,
  });
  term.show();
}

export function activate(ctx: vscode.ExtensionContext) {
  ctx.subscriptions.push(
    vscode.commands.registerCommand("ghit.open", () => open(ctx)),
    vscode.commands.registerCommand("ghit.install", () => install(ctx))
  );
  const item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 0);
  item.text = "$(github) ghit";
  item.tooltip = "Open ghit";
  item.command = "ghit.open";
  item.show();
  ctx.subscriptions.push(item);

  if (!onPath("gh") && !ctx.globalState.get<boolean>("ghWarned")) {
    ctx.globalState.update("ghWarned", true);
    vscode.window.showWarningMessage("ghit needs the GitHub CLI (gh). Install it from https://cli.github.com.");
  }
}

export function deactivate() {}
