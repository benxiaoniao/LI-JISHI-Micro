// 在 VS Code 扩展宿主里跑的集成测试：跳转、F5 运行、诊断、断点。
// 由 test/run-it.ps1 通过 `code --extensionDevelopmentPath --extensionTestsPath` 启动。
const vscode = require('vscode');
const path = require('path');

const results = [];

async function check(name, fn) {
  try {
    await fn();
    results.push(`[通过] ${name}`);
  } catch (error) {
    results.push(`[不通过] ${name}\n  ${error && error.message ? error.message : error}`);
    results.failed = true;
  }
}

function workspaceFile(rel) {
  const root = vscode.workspace.workspaceFolders[0].uri;
  return vscode.Uri.joinPath(root, rel);
}

async function open(rel) {
  const uri = workspaceFile(rel);
  const document = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(document, { preview: false });
  return document;
}

async function definitions(rel, line, character) {
  const uri = workspaceFile(rel);
  const document = await vscode.workspace.openTextDocument(uri);
  const list = await vscode.commands.executeCommand(
    'vscode.executeDefinitionProvider',
    uri,
    new vscode.Position(line - 1, character - 1)
  );
  if (!list) {
    return [];
  }
  return Array.isArray(list) ? list : [list];
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

exports.run = async function run() {
  const extension = vscode.extensions.getExtension('jishim.li-vscode');
  await check('扩展能激活', async () => {
    assert(extension, '找不到 jishim.li-vscode');
    await extension.activate();
  });

  await check('命令面板注册了 li.run / li.toggleBreakpoint', async () => {
    const commands = await vscode.commands.getCommands(true);
    assert(commands.includes('li.run'), '缺 li.run');
    assert(commands.includes('li.toggleBreakpoint'), '缺 li.toggleBreakpoint');
  });

  await check('同文件：c.加(3) → 第 7 行方法', async () => {
    const list = await definitions('examples/类.jsh', 27, 3);
    assert(list.length > 0, '没有定义');
    assert(list[0].uri.fsPath.endsWith('类.jsh'), '文件不对：' + list[0].uri.fsPath);
    assert(list[0].range.start.line === 6, '实际行 ' + (list[0].range.start.line + 1));
  });

  await check('同文件：c.值 → 第 2 行字段', async () => {
    const list = await definitions('examples/类.jsh', 60, 3);
    assert(list.length > 0, '没有定义');
    assert(list[0].range.start.line === 1, '实际行 ' + (list[0].range.start.line + 1));
  });

  await check('同文件：查点(-1) → 第 44 行函数', async () => {
    const list = await definitions('examples/类.jsh', 49, 6);
    assert(list.length > 0, '没有定义');
    assert(list[0].range.start.line === 43, '实际行 ' + (list[0].range.start.line + 1));
  });

  await check('不存在符号：打印 不跳转', async () => {
    const list = await definitions('examples/类.jsh', 26, 2);
    assert(list.length === 0, '不应有定义');
  });

  await check('跨包：工具.加一 → 工具包入口第 1 行', async () => {
    const list = await definitions('build/editor-it/应用/应用.jsh', 1, 7);
    assert(list.length > 0, '没有定义');
    assert(list[0].uri.fsPath.endsWith(path.join('工具', '工具.jsh')), '文件不对：' + list[0].uri.fsPath);
    assert(list[0].range.start.line === 0, '实际行 ' + (list[0].range.start.line + 1));
  });

  await check('导入 引脚 → 跳到语言卡模块声明', async () => {
    const list = await definitions('examples/board/点灯.jsh', 1, 4);
    assert(list.length > 0, '没有定义');
    assert(list[0].uri.fsPath.endsWith('langcard.json'), '文件不对：' + list[0].uri.fsPath);
  });

  await check('F5：运行期错误进问题面板并定位到第 3 行', async () => {
    const document = await open('build/editor-it/错误.jsh');
    await vscode.commands.executeCommand('li.run');
    const diags = vscode.languages.getDiagnostics(document.uri);
    const runDiag = diags.find(d => d.source === 'jmrun');
    assert(runDiag, '没有问题面板诊断');
    assert(runDiag.range.start.line === 2, '实际行 ' + (runDiag.range.start.line + 1));
  });

  await check('F9 + F5：断点命中并停在第 32 行', async () => {
    const document = await open('examples/类.jsh');
    const editor = vscode.window.activeTextEditor;
    editor.selection = new vscode.Selection(31, 0, 31, 0);
    await vscode.commands.executeCommand('li.toggleBreakpoint');
    await vscode.commands.executeCommand('li.run');
    const active = vscode.window.activeTextEditor;
    assert(active.document.uri.fsPath.endsWith('类.jsh'), '命中的文件不对');
    assert(active.selection.active.line === 31, '停在第 ' + (active.selection.active.line + 1) + ' 行');
  });

  await check('F9 再按一次可清除断点', async () => {
    const editor = vscode.window.activeTextEditor;
    editor.selection = new vscode.Selection(31, 0, 31, 0);
    await vscode.commands.executeCommand('li.toggleBreakpoint');
  });

  console.log('\n===== 编辑器集成测试 =====');
  for (const line of results) {
    console.log(line);
  }
  const failedCount = results.filter(line => line.startsWith('[不通过]')).length;
  console.log(`${results.length - failedCount} 通过，${failedCount} 不通过`);

  if (failedCount > 0) {
    throw new Error('编辑器集成测试未全部通过');
  }
};
