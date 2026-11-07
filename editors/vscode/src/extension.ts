import * as vscode from 'vscode';
import { execFile, spawn } from 'child_process';
import type { ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const LANGUAGE_ID = 'li';
const OUTPUT_NAME = '砾';
/** micc 的错误行：`1:7 错误[E102] 不支持指数形式的数字字面量（如 1e3）` */
const ERROR_LINE = /^(\d+):(\d+)\s*错误\[([A-Z]+\d+)\]\s*(.*)$/;

let output: vscode.OutputChannel;
let diagnostics: vscode.DiagnosticCollection;
let statusItem: vscode.StatusBarItem;
let runningChild: ChildProcess | undefined;
let cardCache: { file: string; mtimeMs: number; card: Langcard } | undefined;

const debounceTimers = new Map<string, NodeJS.Timeout>();
const checkSeq = new Map<string, number>();

interface Langcard {
  keywords?: Array<{ word: string; supported?: boolean }>;
  builtins?: Array<{ name: string; min_args?: number; max_args?: number; doc?: string }>;
  native_modules?: Array<{
    module: string;
    table?: string;
    entries?: Array<{ name: string; min_args?: number; max_args?: number; doc?: string }>;
  }>;
}

function config<T>(key: string, fallback: T): T {
  const value = vscode.workspace.getConfiguration('li').get<T>(key);
  return value === undefined ? fallback : value;
}

function workspaceRoot(uri?: vscode.Uri): string | undefined {
  if (uri) {
    const folder = vscode.workspace.getWorkspaceFolder(uri);
    if (folder) {
      return folder.uri.fsPath;
    }
  }
  const folders = vscode.workspace.workspaceFolders;
  return folders && folders.length > 0 ? folders[0].uri.fsPath : undefined;
}

function resolveMicc(root: string): string | undefined {
  const candidates = [
    config<string>('miccPath', 'compiler/target/release/micc.exe'),
    'compiler/target/release/micc.exe',
    'compiler/target/debug/micc.exe',
  ];
  for (const candidate of candidates) {
    const full = path.isAbsolute(candidate) ? candidate : path.join(root, candidate);
    if (fs.existsSync(full)) {
      return full;
    }
  }
  return undefined;
}

function resolveInRoot(root: string, configured: string): string {
  return path.isAbsolute(configured) ? configured : path.join(root, configured);
}

function zeroRange(line: number, column: number): vscode.Range {
  const ln = Math.max(0, line);
  return new vscode.Range(ln, Math.max(0, column), ln, Math.max(0, column) + 1);
}

function makeDiagnostic(line: number, column: number, message: string, code?: string): vscode.Diagnostic {
  const diag = new vscode.Diagnostic(zeroRange(line, column), message, vscode.DiagnosticSeverity.Error);
  diag.source = 'micc';
  if (code) {
    diag.code = code;
  }
  return diag;
}

interface MiccResult {
  code: number;
  stderr: string;
  spawnError?: string;
}

function runMicc(micc: string, source: string, target: string, cwd: string): Promise<MiccResult> {
  return new Promise(resolve => {
    execFile(
      micc,
      [source, '-o', target],
      { cwd, timeout: 20000, windowsHide: true, encoding: 'utf8', maxBuffer: 1024 * 1024 },
      (error, _stdout, stderr) => {
        if (error && typeof (error as NodeJS.ErrnoException).code === 'string') {
          // ENOENT 之类的启动失败：没有可用的 stderr
          resolve({ code: -1, stderr: '', spawnError: String(error.message) });
          return;
        }
        const code = error && typeof error.code === 'number' ? error.code : error ? 1 : 0;
        resolve({ code, stderr: stderr || '' });
      }
    );
  });
}

function removeQuietly(file: string): void {
  try {
    fs.unlinkSync(file);
  } catch {
    // 临时文件清理失败不影响编辑
  }
}

async function checkDocument(document: vscode.TextDocument, interactive = false): Promise<void> {
  if (!diagnostics || document.languageId !== LANGUAGE_ID) {
    return;
  }

  const root = workspaceRoot(document.uri);
  if (!root) {
    return;
  }

  const micc = resolveMicc(root);
  if (!micc) {
    diagnostics.set(document.uri, [
      new vscode.Diagnostic(
        zeroRange(0, 0),
        '找不到 micc：请先运行 tools\\build.ps1，或用设置 li.miccPath 指定可执行文件路径。',
        vscode.DiagnosticSeverity.Warning
      ),
    ]);
    if (interactive) {
      output.appendLine('[诊断] 未找到 micc，已跳过编译检查。');
      output.show(true);
    }
    return;
  }

  const key = document.uri.toString();
  const seq = (checkSeq.get(key) ?? 0) + 1;
  checkSeq.set(key, seq);

  const dirty = document.isDirty;
  const source = dirty
    ? path.join(os.tmpdir(), `li-src-${process.pid}-${Date.now()}.jsh`)
    : document.uri.fsPath;
  const target = path.join(os.tmpdir(), `li-out-${process.pid}-${Date.now()}.jmb`);

  if (dirty) {
    try {
      fs.writeFileSync(source, document.getText(), 'utf8');
    } catch (error) {
      output.appendLine(`[诊断] 无法写入临时源文件：${String(error)}`);
      return;
    }
  }

  let result: MiccResult;
  try {
    result = await runMicc(micc, source, target, root);
  } finally {
    if (dirty) {
      removeQuietly(source);
    }
    removeQuietly(target);
  }

  // 键盘输入会并发触发多次检查，只接受最后一次的结果
  if (checkSeq.get(key) !== seq) {
    return;
  }

  if (result.spawnError) {
    diagnostics.set(document.uri, [
      makeDiagnostic(0, 0, `无法启动 micc：${result.spawnError}`),
    ]);
    if (interactive) {
      vscode.window.showErrorMessage(`砾：无法启动 micc（${result.spawnError}）`);
    }
    return;
  }

  if (result.code === 0) {
    diagnostics.delete(document.uri);
    if (interactive) {
      output.appendLine('[诊断] 通过，没有编译错误。');
      output.show(true);
    }
    return;
  }

  const found: vscode.Diagnostic[] = [];
  const other: string[] = [];
  for (const raw of result.stderr.split(/\r?\n/)) {
    const line = raw.replace(/^\uFEFF/, '').trim();
    if (!line) {
      continue;
    }
    const match = ERROR_LINE.exec(line);
    if (match) {
      const lineNo = parseInt(match[1], 10) - 1;
      const columnNo = parseInt(match[2], 10) - 1;
      found.push(makeDiagnostic(lineNo, columnNo, `${match[3]}：${match[4]}`, match[3]));
    } else {
      other.push(line);
    }
  }

  if (found.length === 0) {
    found.push(makeDiagnostic(0, 0, other.length > 0 ? other.join('\n') : 'micc 退出码非 0（未给出错误行）'));
  }

  diagnostics.set(document.uri, found);
  if (interactive) {
    output.appendLine(`[诊断] ${found.length} 处错误。`);
    found.forEach(diag => {
      output.appendLine(`  ${diag.range.start.line + 1}:${diag.range.start.character + 1} ${diag.message}`);
    });
    output.show(true);
  }
}

function scheduleCheck(document: vscode.TextDocument, delay: number): void {
  const key = document.uri.toString();
  const previous = debounceTimers.get(key);
  if (previous) {
    clearTimeout(previous);
  }
  debounceTimers.set(
    key,
    setTimeout(() => {
      debounceTimers.delete(key);
      void checkDocument(document);
    }, delay)
  );
}

function stopRunning(): void {
  if (runningChild) {
    runningChild.kill();
  }
}

function setRunning(label: string | undefined): void {
  if (label) {
    statusItem.text = `$(sync~spin) 砾：${label}`;
    statusItem.tooltip = '点击取消当前任务';
    statusItem.show();
  } else {
    statusItem.hide();
  }
}

function runScript(script: string, args: string[], label: string): void {
  const root = workspaceRoot(vscode.window.activeTextEditor?.document.uri);
  if (!root) {
    vscode.window.showErrorMessage('砾：请先打开仓库（工作区）文件夹。');
    return;
  }

  const scriptPath = path.join(root, script);
  if (!fs.existsSync(scriptPath)) {
    vscode.window.showErrorMessage(`砾：找不到脚本 ${script}（工作区 ${root}）。`);
    return;
  }
  if (runningChild) {
    vscode.window.showWarningMessage(`砾：已有任务在运行（${label} 被忽略）。`);
    return;
  }

  output.clear();
  output.show(true);

  const quote = (value: string): string => `'${value.replace(/'/g, "''")}'`;
  const command = `& { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; & ${quote(scriptPath)}${
    args.length > 0 ? ' ' + args.map(quote).join(' ') : ''
  } }`;
  const powershell = config<string>('powershell', 'powershell.exe');

  output.appendLine(`== ${label}`);
  output.appendLine(`> ${script} ${args.join(' ')}`);

  let child: ChildProcess;
  try {
    child = spawn(powershell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-NonInteractive', '-Command', command], {
      cwd: root,
      windowsHide: true,
    });
  } catch (error) {
    output.appendLine(`启动失败：${String(error)}`);
    return;
  }

  runningChild = child;
  setRunning(label);

  child.stdout?.setEncoding('utf8');
  child.stderr?.setEncoding('utf8');
  child.stdout?.on('data', (chunk: string) => output.append(chunk));
  child.stderr?.on('data', (chunk: string) => output.append(chunk));

  child.on('error', (error: Error) => {
    output.appendLine(`启动 PowerShell 失败：${error.message}`);
  });

  child.on('close', (code: number | null) => {
    runningChild = undefined;
    setRunning(undefined);
    if (code === 0) {
      output.appendLine(`== ${label} 完成`);
    } else if (code === null) {
      output.appendLine(`== ${label} 已取消`);
    } else {
      output.appendLine(`== ${label} 失败，退出码 ${code}`);
    }
    // 固件相关命令结束后刷新诊断，顺带确认源码仍可编译
    const document = vscode.window.activeTextEditor?.document;
    if (document && document.languageId === LANGUAGE_ID) {
      void checkDocument(document);
    }
  });
}

function firmwareExample(root: string, document?: vscode.TextDocument): string {
  if (document && document.uri.scheme === 'file') {
    const examplesDir = path.join(root, 'examples');
    const relative = path.relative(examplesDir, document.uri.fsPath);
    if (relative && !relative.startsWith('..') && !path.isAbsolute(relative) && relative.toLowerCase().endsWith('.jsh')) {
      return relative.slice(0, -'.jsh'.length);
    }
  }
  return config<string>('firmwareExample', '你好');
}

function loadLangcard(file: string): Langcard | undefined {
  try {
    const stat = fs.statSync(file);
    if (cardCache && cardCache.file === file && cardCache.mtimeMs === stat.mtimeMs) {
      return cardCache.card;
    }
    const card = JSON.parse(fs.readFileSync(file, 'utf8')) as Langcard;
    cardCache = { file, mtimeMs: stat.mtimeMs, card };
    return card;
  } catch {
    return undefined;
  }
}

/** 当前光标是否落在字符串或注释里（按行内引号配对判断）。 */
function inStringOrComment(document: vscode.TextDocument, position: vscode.Position): boolean {
  const before = document.lineAt(position.line).text.slice(0, position.character);
  let closing = '';
  for (const ch of before) {
    if (closing) {
      if (ch === closing) {
        closing = '';
      }
      continue;
    }
    if (ch === '#') {
      return true;
    }
    if (ch === '"' || ch === "'") {
      closing = ch;
    } else if (ch === '“') {
      closing = '”';
    } else if (ch === '‘') {
      closing = '’';
    }
  }
  return closing !== '';
}

class LangcardCompletions implements vscode.CompletionItemProvider {
  provideCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position
  ): vscode.CompletionItem[] | undefined {
    if (document.languageId !== LANGUAGE_ID || inStringOrComment(document, position)) {
      return undefined;
    }

    const root = workspaceRoot(document.uri);
    if (!root) {
      return undefined;
    }

    const card = loadLangcard(resolveInRoot(root, config<string>('langcardPath', 'spec/langcard.json')));
    if (!card) {
      return undefined;
    }

    const items: vscode.CompletionItem[] = [];
    const imported = new Map<string, string>(); // 小写模块名 -> 原模块名
    const importPattern = /导入\s+(\S+)/g;
    const text = document.getText();
    let match: RegExpExecArray | null;
    while ((match = importPattern.exec(text)) !== null) {
      imported.set(match[1].toLowerCase(), match[1]);
    }

    const linePrefix = document.lineAt(position.line).text.slice(0, position.character);
    if (/^\s*导入\s+\S*$/.test(linePrefix)) {
      for (const module of card.native_modules ?? []) {
        const item = new vscode.CompletionItem(module.module, vscode.CompletionItemKind.Module);
        item.detail = '砾原生模块';
        item.documentation = new vscode.MarkdownString(
          [`模块 \`${module.module}\``, '', ...(module.entries ?? []).map(e => `- \`${e.name}\` ${e.doc ?? ''}`)].join(
            '\n'
          )
        );
        items.push(item);
      }
      return items;
    }

    for (const keyword of card.keywords ?? []) {
      if (keyword.supported === false) {
        continue;
      }
      const item = new vscode.CompletionItem(keyword.word, vscode.CompletionItemKind.Keyword);
      item.detail = '关键字';
      item.sortText = `0-${keyword.word}`;
      items.push(item);
    }

    for (const builtin of card.builtins ?? []) {
      const item = new vscode.CompletionItem(builtin.name, vscode.CompletionItemKind.Function);
      item.detail = `内建函数（${builtin.min_args ?? 0}–${builtin.max_args ?? '?'} 个参数）`;
      if (builtin.doc) {
        item.documentation = new vscode.MarkdownString(builtin.doc);
      }
      item.sortText = `1-${builtin.name}`;
      items.push(item);
    }

    for (const module of card.native_modules ?? []) {
      if (!imported.has(module.module.toLowerCase())) {
        continue;
      }
      for (const entry of module.entries ?? []) {
        const item = new vscode.CompletionItem(entry.name, vscode.CompletionItemKind.Function);
        item.detail = `原生函数 · ${module.module}`;
        if (entry.doc) {
          item.documentation = new vscode.MarkdownString(entry.doc);
        }
        item.sortText = `2-${entry.name}`;
        items.push(item);
      }
    }

    return items;
  }
}

export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel(OUTPUT_NAME);
  diagnostics = vscode.languages.createDiagnosticCollection('li');
  statusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 90);
  statusItem.command = 'li.cancel';

  context.subscriptions.push(
    output,
    diagnostics,
    statusItem,

    vscode.commands.registerCommand('li.check', () => {
      const document = vscode.window.activeTextEditor?.document;
      if (!document) {
        vscode.window.showInformationMessage('砾：请先打开一个 .jsh 文件。');
        return;
      }
      return checkDocument(document, true);
    }),

    vscode.commands.registerCommand('li.build', () => runScript('tools/build.ps1', [], '构建')),

    vscode.commands.registerCommand('li.buildFirmware', () => {
      const root = workspaceRoot(vscode.window.activeTextEditor?.document.uri);
      if (!root) {
        vscode.window.showErrorMessage('砾：请先打开仓库（工作区）文件夹。');
        return;
      }
      const example = firmwareExample(root, vscode.window.activeTextEditor?.document);
      runScript('tools/build-firmware.ps1', ['-Example', example], `构建固件（${example}）`);
    }),

    vscode.commands.registerCommand('li.flash', async () => {
      const confirmed = await vscode.window.showWarningMessage(
        '将把 build\\stm32\\jishim.bin 写入 STM32 的 0x08000000（需要已连接 ST-Link）。继续？',
        { modal: true },
        '烧录'
      );
      if (confirmed !== '烧录') {
        return;
      }
      runScript('tools/flash-firmware.ps1', ['-Apply'], '烧录');
    }),

    vscode.commands.registerCommand('li.cancel', () => {
      if (runningChild) {
        output.appendLine('== 用户取消');
        stopRunning();
      }
    }),

    vscode.commands.registerCommand('li.reloadLangcard', () => {
      cardCache = undefined;
      const root = workspaceRoot(vscode.window.activeTextEditor?.document.uri);
      const file = root ? resolveInRoot(root, config<string>('langcardPath', 'spec/langcard.json')) : '';
      if (file && loadLangcard(file)) {
        vscode.window.showInformationMessage('砾：语言卡补全已重新加载。');
      } else {
        vscode.window.showWarningMessage('砾：没有找到语言卡 spec/langcard.json。');
      }
    }),

    vscode.languages.registerCompletionItemProvider(LANGUAGE_ID, new LangcardCompletions()),

    vscode.workspace.onDidOpenTextDocument(document => {
      if (document.languageId === LANGUAGE_ID) {
        void checkDocument(document);
      }
    }),

    vscode.workspace.onDidSaveTextDocument(document => {
      if (document.languageId === LANGUAGE_ID) {
        void checkDocument(document);
      }
    }),

    vscode.workspace.onDidCloseTextDocument(document => {
      diagnostics.delete(document.uri);
      const timer = debounceTimers.get(document.uri.toString());
      if (timer) {
        clearTimeout(timer);
        debounceTimers.delete(document.uri.toString());
      }
    }),

    vscode.workspace.onDidChangeTextDocument(event => {
      const document = event.document;
      if (document.languageId !== LANGUAGE_ID || !config<boolean>('diagnosticsOnType', true)) {
        return;
      }
      scheduleCheck(document, 700);
    }),

    vscode.workspace.onDidChangeConfiguration(event => {
      if (event.affectsConfiguration('li')) {
        cardCache = undefined;
        for (const document of vscode.workspace.textDocuments) {
          if (document.languageId === LANGUAGE_ID) {
            void checkDocument(document);
          }
        }
      }
    })
  );

  // 启动时已打开的 .jsh
  for (const document of vscode.workspace.textDocuments) {
    if (document.languageId === LANGUAGE_ID) {
      void checkDocument(document);
    }
  }
}

export function deactivate(): void {
  stopRunning();
  if (output) {
    output.dispose();
  }
}
