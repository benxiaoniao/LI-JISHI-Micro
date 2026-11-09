import * as vscode from 'vscode';
import { execFile, spawn } from 'child_process';
import type { ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  buildDebugArgs,
  decideDefinition,
  displayPath,
  findModuleLine,
  formatSource,
  nearestManifest,
  parseDebugEvent,
  parsePackFiles,
  parseRunOutput,
  parseSymbols,
  toggleBreak,
} from './core';
import type { BreakSpec, PackInfo, Site, SymbolInfo } from './core';

const LANGUAGE_ID = 'li';
const OUTPUT_NAME = '砾';
/** micc 的错误行：`1:7 错误[E102] 不支持指数形式的数字字面量（如 1e3）` */
const ERROR_LINE = /^(\d+):(\d+)\s*错误\[([A-Z]+\d+)\]\s*(.*)$/;

let output: vscode.OutputChannel;
let diagnostics: vscode.DiagnosticCollection;
let runDiagnostics: vscode.DiagnosticCollection;
let statusItem: vscode.StatusBarItem;
let breakpointDecoration: vscode.TextEditorDecorationType;
let runningChild: ChildProcess | undefined;
let cardCache: { file: string; mtimeMs: number; card: Langcard } | undefined;

/** 每个文档一个断点集合（多个行号）：文档 URI → 1 起行号数组。 */
const breakpoints = new Map<string, number[]>();
let debugChild: ChildProcess | undefined;
let debugPaused = false;
let debugPrompting = false;
const runUris = new Map<string, vscode.Uri>();
const symbolCache = new Map<string, { mtimeMs: number; symbols: SymbolInfo[] }>();
const packCache = new Map<string, { mtimeMs: number; packs: PackInfo[] }>();

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

function resolveJmrun(root: string): string | undefined {
  const candidates = [
    config<string>('jmrunPath', 'build/jmrun.exe'),
    'build/jmrun.exe',
    'build/jmrun',
  ];
  for (const candidate of candidates) {
    const full = path.isAbsolute(candidate) ? candidate : path.join(root, candidate);
    if (fs.existsSync(full)) {
      return full;
    }
  }
  return undefined;
}

interface TextResult {
  code: number;
  out: string;
  err: string;
  spawnError?: string;
}

/** 跑一个短命命令并收全 stdout/stderr（UTF-8）。 */
function runText(exe: string, args: string[], cwd: string, timeout = 30000): Promise<TextResult> {
  return new Promise(resolve => {
    execFile(
      exe,
      args,
      { cwd, timeout, windowsHide: true, encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 },
      (error, stdout, stderr) => {
        if (error && typeof (error as NodeJS.ErrnoException).code === 'string') {
          resolve({ code: -1, out: '', err: '', spawnError: String(error.message) });
          return;
        }
        const code = error && typeof error.code === 'number' ? error.code : error ? 1 : 0;
        resolve({ code, out: stdout || '', err: stderr || '' });
      }
    );
  });
}

/** 取文件的定义表；按 mtime 缓存，保存后自动失效。 */
async function loadSymbols(micc: string, file: string): Promise<SymbolInfo[]> {
  let mtimeMs = 0;
  try {
    mtimeMs = fs.statSync(file).mtimeMs;
  } catch {
    return [];
  }
  const cached = symbolCache.get(file);
  if (cached && cached.mtimeMs === mtimeMs) {
    return cached.symbols;
  }
  const result = await runText(micc, ['--symbols', file], path.dirname(file), 15000);
  const symbols = result.code === 0 ? parseSymbols(result.out) : [];
  symbolCache.set(file, { mtimeMs, symbols });
  return symbols;
}

/** 取包与依赖的入口文件；按 `砾包.toml` 的 mtime 缓存。 */
async function loadPacks(micc: string, manifestDir: string): Promise<PackInfo[]> {
  let mtimeMs = 0;
  try {
    mtimeMs = fs.statSync(path.join(manifestDir, '砾包.toml')).mtimeMs;
  } catch {
    return [];
  }
  const cached = packCache.get(manifestDir);
  if (cached && cached.mtimeMs === mtimeMs) {
    return cached.packs;
  }
  const result = await runText(micc, ['--pack-files', manifestDir], manifestDir, 15000);
  const packs = result.code === 0 ? parsePackFiles(result.out) : [];
  packCache.set(manifestDir, { mtimeMs, packs });
  return packs;
}

/** 光标处标识符的定义位置；找不到返回 undefined。 */
async function definitionAt(
  document: vscode.TextDocument,
  position: vscode.Position
): Promise<vscode.Location | undefined> {
  const wordRange = document.getWordRangeAtPosition(position, /[\p{L}\p{N}_]+/u);
  const root = workspaceRoot(document.uri);
  if (!wordRange || !root) {
    return undefined;
  }
  const micc = resolveMicc(root);
  if (!micc) {
    return undefined;
  }
  const word = document.getText(wordRange);
  const local = await loadSymbols(micc, document.uri.fsPath);

  // `接收者.名字`：取出点号左边的标识符
  const lineText = document.lineAt(position.line).text;
  const start = wordRange.start.character;
  let receiver: string | undefined;
  if (start > 0 && lineText[start - 1] === '.') {
    const before = lineText.slice(0, start - 1);
    const match = /([\p{L}\p{N}_]+)\s*$/u.exec(before);
    if (match) {
      receiver = match[1];
    }
  }

  const manifest = nearestManifest(document.uri.fsPath, root);
  const packs = manifest ? await loadPacks(micc, manifest) : [];

  // 只为候选包预取一次依赖入口的定义表，其余判断交给纯函数
  const remote = new Map<string, SymbolInfo[]>();
  const candidate = receiver ?? word;
  const pkg = packs.find(p => p.name === candidate);
  if (pkg) {
    remote.set(pkg.entry, await loadSymbols(micc, pkg.entry));
  }

  let moduleLine: number | undefined;
  let langcardFile: string | undefined;
  if (new RegExp(`导入\\s+${escapeRegExp(word)}(\\s|$)`).test(document.getText())) {
    langcardFile = resolveInRoot(root, config<string>('langcardPath', 'spec/langcard.json'));
    try {
      moduleLine = findModuleLine(fs.readFileSync(langcardFile, 'utf8'), word);
    } catch {
      moduleLine = undefined;
    }
  }

  const decision = decideDefinition({
    word,
    receiver,
    file: document.uri.fsPath,
    local,
    packs,
    remote,
    langcardFile,
    moduleLine,
  });
  if (!decision) {
    return undefined;
  }
  return new vscode.Location(
    vscode.Uri.file(decision.file),
    new vscode.Position(Math.max(0, decision.line - 1), Math.max(0, decision.col - 1))
  );
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

class LiDefinitions implements vscode.DefinitionProvider {
  provideDefinition(
    document: vscode.TextDocument,
    position: vscode.Position
  ): vscode.ProviderResult<vscode.Definition> {
    if (document.languageId !== LANGUAGE_ID) {
      return undefined;
    }
    return definitionAt(document, position);
  }
}

/** Shift+Alt+F：缩进归一 + 运算符空格；逻辑全在 core.formatSource，便于命令行测幂等。 */
class LiFormatter implements vscode.DocumentFormattingEditProvider {
  provideDocumentFormattingEdits(document: vscode.TextDocument): vscode.TextEdit[] {
    if (document.languageId !== LANGUAGE_ID) {
      return [];
    }
    const original = document.getText();
    const formatted = formatSource(original);
    if (formatted === original) {
      return [];
    }
    return [vscode.TextEdit.replace(
      new vscode.Range(document.positionAt(0), document.positionAt(original.length)),
      formatted
    )];
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

function refreshBreakpointDecorations(editor: vscode.TextEditor): void {
  if (!breakpointDecoration || editor.document.languageId !== LANGUAGE_ID) {
    return;
  }
  const lines = breakpoints.get(editor.document.uri.toString()) ?? [];
  const ranges = lines.map(line => new vscode.Range(line - 1, 0, line - 1, 0));
  editor.setDecorations(breakpointDecoration, ranges);
}

function toggleBreakpoint(): void {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.languageId !== LANGUAGE_ID) {
    vscode.window.showInformationMessage('砾：断点只支持 .jsh 文件。');
    return;
  }
  const key = editor.document.uri.toString();
  const line = editor.selection.active.line + 1;
  const next = toggleBreak(breakpoints.get(key) ?? [], line);
  if (next.length > 8) {
    vscode.window.showWarningMessage('砾：最多 8 个断点（jmrun 的上限）；本次未添加。');
    return;
  }
  if (next.includes(line)) {
    vscode.window.setStatusBarMessage(`砾：断点设在第 ${line} 行（共 ${next.length} 个）`, 3000);
  } else {
    vscode.window.setStatusBarMessage(`砾：已清除第 ${line} 行断点（剩 ${next.length} 个）`, 2500);
  }
  if (next.length === 0) {
    breakpoints.delete(key);
  } else {
    breakpoints.set(key, next);
  }
  refreshBreakpointDecorations(editor);
}

function siteUri(root: string, file: string): vscode.Uri {
  return vscode.Uri.file(path.isAbsolute(file) ? file : path.join(root, file));
}

/** 把 jmrun 的 `于 文件:行` 变成问题面板里的可点击诊断。 */
function publishRunDiagnostics(document: vscode.TextDocument, root: string, text: string): Site | undefined {
  const parsed = parseRunOutput(text);
  for (const uri of runUris.values()) {
    runDiagnostics.delete(uri);
  }
  runUris.clear();

  if (parsed.hit) {
    const uri = siteUri(root, parsed.hit.file);
    const line = Math.max(0, parsed.hit.line - 1);
    vscode.window.showInformationMessage(`砾：断点命中 ${parsed.hit.file}:${parsed.hit.line}`);
    void vscode.window.showTextDocument(uri, {
      selection: new vscode.Range(line, 0, line, 0),
      preserveFocus: false,
      preview: true,
    });
  }

  const error = parsed.error;
  if (!error) {
    return parsed.hit;
  }
  let target = document.uri;
  let line = 0;
  let note = '';
  if (error.site) {
    target = siteUri(root, error.site.file);
    line = Math.max(0, error.site.line - 1);
  } else {
    note = '（无调试段，只能给到错误码）';
  }
  const diag = new vscode.Diagnostic(
    new vscode.Range(line, 0, line, 1),
    `${error.code ?? '运行期错误'}：${error.message}${note}`,
    vscode.DiagnosticSeverity.Error
  );
  diag.source = 'jmrun';
  if (error.code) {
    diag.code = error.code;
  }
  runDiagnostics.set(target, [diag]);
  runUris.set(target.toString(), target);
  return error.site;
}

/** F5：把当前文件编译成临时 .jmb，再交给 jmrun 跑；有断点就带上。 */
async function runCurrent(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.languageId !== LANGUAGE_ID) {
    vscode.window.showInformationMessage('砾：请先把光标放在 .jsh 文件里再运行。');
    return;
  }
  const document = editor.document;
  const root = workspaceRoot(document.uri);
  const micc = root ? resolveMicc(root) : undefined;
  const jmrun = root ? resolveJmrun(root) : undefined;
  if (!root || !micc || !jmrun) {
    vscode.window.showErrorMessage(
      '砾：运行需要工作区里的 micc 与 jmrun（先运行 tools\\build.ps1，或用 li.miccPath / li.jmrunPath 指定）。'
    );
    return;
  }

  // 调试表记录的是保存时的文件，先落盘避免行号指向临时副本
  await document.save();

  output.clear();
  output.show(true);

  const target = path.join(os.tmpdir(), `li-run-${process.pid}-${Date.now()}.jmb`);
  const compiled = await runMicc(micc, document.uri.fsPath, target, root);
  if (compiled.spawnError || compiled.code !== 0) {
    output.appendLine(compiled.spawnError ? `无法启动 micc：${compiled.spawnError}` : compiled.stderr);
    removeQuietly(target);
    await checkDocument(document, false);
    vscode.window.showErrorMessage('砾：编译失败，详见输出与问题面板。');
    return;
  }

  const lines = breakpoints.get(document.uri.toString()) ?? [];
  if (lines.length > 0) {
    const spec: BreakSpec[] = lines.map(line => ({ file: displayPath(root, document.uri.fsPath), line }));
    await startDebugSession(root, document, jmrun, target, spec);
    return;
  }
  const args = [target];
  output.appendLine(`== 运行 ${path.basename(document.uri.fsPath)}`);
  output.appendLine(`> jmrun ${args.map(a => (a.includes(' ') ? `"${a}"` : a)).join(' ')}`);

  const result = await runText(jmrun, args, root);
  removeQuietly(target);
  if (result.spawnError) {
    output.appendLine(`无法启动 jmrun：${result.spawnError}`);
    vscode.window.showErrorMessage(`砾：无法启动 jmrun（${result.spawnError}）。`);
    return;
  }
  if (result.out) {
    output.append(result.out);
  }
  if (result.err) {
    output.append(result.err);
  }
  output.appendLine(`== 退出码 ${result.code}`);

  const site = publishRunDiagnostics(document, root, result.out + result.err);
  if (site && !parseRunOutput(result.out + result.err).hit) {
    output.appendLine(`源码位置：${site.file}:${site.line}`);
  }
}

function setDebugPaused(paused: boolean, label?: string): void {
  debugPaused = paused;
  if (paused) {
    statusItem.text = `$(debug-pause) 砾：${label ?? '已暂停'}`;
    statusItem.tooltip = '已暂停：F10 下一行 / Ctrl+F10 步过 / F5 继续 / Shift+F5 停止';
    statusItem.show();
  } else {
    statusItem.hide();
  }
}

function sendDebug(cmd: string): void {
  if (!debugChild || !debugChild.stdin) {
    vscode.window.showInformationMessage('砾：当前没有调试会话（F5 且有断点时才会进入调试）。');
    return;
  }
  if (!debugPaused && (cmd === 'c' || cmd === 'n' || cmd === 'o' || cmd === 's')) {
    vscode.window.showInformationMessage('砾：程序未暂停，无需继续/步进。');
    return;
  }
  debugChild.stdin.write(cmd + '\n');
  if (cmd !== 'bl' && cmd !== 'v' && cmd !== 'bt') {
    setDebugPaused(false);
  }
}

async function promptDebugAction(): Promise<void> {
  if (debugPrompting || !debugChild) {
    return;
  }
  if (!config<boolean>('debugPrompt', true)) {
    return;
  }
  debugPrompting = true;
  try {
    const options = [
      { label: '$(play) 继续', description: 'c', cmd: 'c' },
      { label: '$(arrow-right) 下一行', description: 'n', cmd: 'n' },
      { label: '$(debug-step-over) 步过', description: 'o', cmd: 'o' },
      { label: '$(debug-step-into) 单步指令', description: 's', cmd: 's' },
      { label: '$(list-unordered) 断点列表', description: 'bl', cmd: 'bl' },
      { label: '$(stop) 退出', description: 'q', cmd: 'q' },
    ];
    const pick = await vscode.window.showQuickPick(options, {
      placeHolder: '砾调试：选择动作（Esc 保持暂停，之后可用 F10 / F5）',
      ignoreFocusOut: true,
    });
    if (pick) {
      sendDebug(pick.cmd);
    }
  } finally {
    debugPrompting = false;
  }
}

/** 事件行 → 输出通道 / 状态栏 / 编辑器定位；非事件行原样显示。 */
function handleDebugLine(root: string, raw: string, onPause?: () => void): void {
  const ev = parseDebugEvent(raw);
  if (!ev) {
    output.appendLine(raw);
    return;
  }
  if (ev.event === 'pause') {
    const where = ev.file ? `${ev.file}:${ev.line}` : '（无行表）';
    output.appendLine(`== 暂停（${ev.reason}）任务 ${ev.task}「${ev.func}」 ${where}`);
    setDebugPaused(true, `暂停 ${ev.func} ${where}`);
    if (ev.file && ev.line > 0) {
      const uri = siteUri(root, ev.file);
      const row = Math.max(0, ev.line - 1);
      void vscode.window.showTextDocument(uri, {
        selection: new vscode.Range(row, 0, row, 0),
        preserveFocus: true,
        preview: true,
      });
    }
    void promptDebugAction();
    onPause?.();
    return;
  }
  output.appendLine(`== 结束（码 ${ev.code}${ev.quit ? '，用户退出' : ''}）`);
  setDebugPaused(false);
}

/** 有断点时走完整调试：`--dbg --事件流`，交互命令走 stdin。首次暂停（或结束）后 resolve。 */
function startDebugSession(
  root: string,
  document: vscode.TextDocument,
  jmrun: string,
  target: string,
  spec: BreakSpec[]
): Promise<void> {
  return new Promise(resolve => {
    let settled = false;
    const settle = (): void => {
      if (!settled) {
        settled = true;
        resolve();
      }
    };
    const args = buildDebugArgs({ jmb: target, breaks: spec });
    output.appendLine(`== 调试 ${path.basename(document.uri.fsPath)}（断点 ${spec.length} 个）`);
    output.appendLine(`> jmrun ${args.map(a => (a.includes(' ') ? `"${a}"` : a)).join(' ')}`);
    const child = spawn(jmrun, args, { cwd: root, windowsHide: true });
    debugChild = child;
    child.stdout?.setEncoding('utf8');
    child.stderr?.setEncoding('utf8');
    child.stdout?.on('data', (chunk: string | Buffer) => output.append(String(chunk)));

    let buf = '';
    child.stderr?.on('data', (chunk: string | Buffer) => {
      buf += String(chunk);
      let idx = buf.indexOf('\n');
      while (idx >= 0) {
        const line = buf.slice(0, idx).replace(/\r$/, '');
        buf = buf.slice(idx + 1);
        handleDebugLine(root, line, settle);
        idx = buf.indexOf('\n');
      }
    });
    child.on('error', err => {
      output.appendLine(`无法启动 jmrun：${err.message}`);
      vscode.window.showErrorMessage(`砾：无法启动 jmrun（${err.message}）。`);
      debugChild = undefined;
      setDebugPaused(false);
      removeQuietly(target);
      settle();
    });
    child.on('close', code => {
      if (buf) {
        handleDebugLine(root, buf.replace(/\r$/, ''));
        buf = '';
      }
      output.appendLine(`== 退出码 ${code ?? -1}`);
      debugChild = undefined;
      setDebugPaused(false);
      removeQuietly(target);
      settle();
    });
  });
}

function stopDebug(): void {
  if (debugChild) {
    debugChild.kill();
  }
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
  runDiagnostics = vscode.languages.createDiagnosticCollection('li-run');
  breakpointDecoration = vscode.window.createTextEditorDecorationType({
    isWholeLine: true,
    backgroundColor: 'rgba(220, 40, 40, 0.12)',
    borderColor: new vscode.ThemeColor('editorError.foreground'),
    borderStyle: 'solid',
    borderWidth: '0 0 0 3px',
    overviewRulerColor: new vscode.ThemeColor('editorOverviewRuler.errorForeground'),
    overviewRulerLane: vscode.OverviewRulerLane.Left,
  });
  statusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 90);
  statusItem.command = 'li.cancel';

  context.subscriptions.push(
    output,
    diagnostics,
    runDiagnostics,
    breakpointDecoration,
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

    vscode.commands.registerCommand('li.run', () => {
      if (debugChild) {
        sendDebug('c');
        return;
      }
      return runCurrent();
    }),

    vscode.commands.registerCommand('li.toggleBreakpoint', () => toggleBreakpoint()),

    vscode.commands.registerCommand('li.debug.continue', () => sendDebug('c')),
    vscode.commands.registerCommand('li.debug.next', () => sendDebug('n')),
    vscode.commands.registerCommand('li.debug.step', () => sendDebug('s')),
    vscode.commands.registerCommand('li.debug.stepOut', () => sendDebug('o')),
    vscode.commands.registerCommand('li.debug.breaks', () => sendDebug('bl')),
    vscode.commands.registerCommand('li.debug.stop', () => stopDebug()),

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

    vscode.languages.registerDefinitionProvider(LANGUAGE_ID, new LiDefinitions()),

    vscode.languages.registerDocumentFormattingEditProvider(LANGUAGE_ID, new LiFormatter()),

    vscode.window.onDidChangeActiveTextEditor(editor => {
      if (editor) {
        refreshBreakpointDecorations(editor);
      }
    }),

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
  stopDebug();
  if (output) {
    output.dispose();
  }
}
