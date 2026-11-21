// 砾的 DAP 接线：扩展只负责给出 micc/jmrun 路径，协议翻译全在 `micc dap` 里。
import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';

export const DEBUG_TYPE = 'li';

const MICC_CANDIDATES = [
  'compiler/target/release/micc.exe',
  'compiler/target/release/micc',
  'compiler/target/debug/micc.exe',
  'compiler/target/debug/micc',
];
const JMRUN_CANDIDATES = ['build/jmrun.exe', 'build/jmrun'];

function existsInRoot(root: string, candidates: string[]): string | undefined {
  for (const candidate of candidates) {
    const full = path.isAbsolute(candidate) ? candidate : path.join(root, candidate);
    if (fs.existsSync(full)) {
      return full;
    }
  }
  return undefined;
}

/** 补全 launch 配置：F5 / 调试面板启动时都走这里。 */
export class LiDebugConfigurationProvider implements vscode.DebugConfigurationProvider {
  resolveDebugConfiguration(
    folder: vscode.WorkspaceFolder | undefined,
    configuration: vscode.DebugConfiguration
  ): vscode.ProviderResult<vscode.DebugConfiguration> {
    const root = folder?.uri.fsPath ?? vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (!root) {
      vscode.window.showErrorMessage('砾：请先打开仓库（工作区）文件夹。');
      return undefined;
    }
    if (!configuration.program) {
      const document = vscode.window.activeTextEditor?.document;
      if (!document || document.languageId !== 'li') {
        vscode.window.showInformationMessage('砾：请先把光标放在 .jsh 文件里再调试。');
        return undefined;
      }
      configuration.program = document.uri.fsPath;
    }
    configuration.cwd = configuration.cwd ?? root;
    if (!configuration.miccPath) {
      const configured = vscode.workspace.getConfiguration('li').get<string>('miccPath');
      configuration.miccPath = configured
        ? existsInRoot(root, [configured]) ?? existsInRoot(root, MICC_CANDIDATES)
        : existsInRoot(root, MICC_CANDIDATES);
    }
    if (!configuration.jmrunPath) {
      const configured = vscode.workspace.getConfiguration('li').get<string>('jmrunPath');
      configuration.jmrunPath = configured
        ? existsInRoot(root, [configured]) ?? existsInRoot(root, JMRUN_CANDIDATES)
        : existsInRoot(root, JMRUN_CANDIDATES);
    }
    if (!configuration.miccPath || !configuration.jmrunPath) {
      vscode.window.showErrorMessage(
        '砾：调试需要工作区里的 micc 与 jmrun（先运行 tools\\build.ps1，或用 li.miccPath / li.jmrunPath 指定）。'
      );
      return undefined;
    }
    return configuration;
  }
}

/** 启动 `micc dap`，由它去 spawn `jmrun --dbg --事件流`。 */
export class LiDebugAdapterFactory implements vscode.DebugAdapterDescriptorFactory {
  createDebugAdapterDescriptor(
    session: vscode.DebugSession
  ): vscode.ProviderResult<vscode.DebugAdapterDescriptor> {
    const micc = session.configuration.miccPath;
    if (typeof micc !== 'string' || !micc) {
      vscode.window.showErrorMessage('砾：调试适配器缺少 micc 路径。');
      return undefined;
    }
    const cwd =
      typeof session.configuration.cwd === 'string'
        ? session.configuration.cwd
        : session.workspaceFolder?.uri.fsPath;
    return new vscode.DebugAdapterExecutable(micc, ['dap'], cwd ? { cwd } : undefined);
  }
}
