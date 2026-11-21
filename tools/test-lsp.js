// micc lsp 回归：通过 stdio 走 JSON-RPC，覆盖诊断/补全/定义/格式化/退出。
// 与编辑器 verify.js 等价：不依赖 VS Code，`node tools/test-lsp.js` 可直接复现。
const { spawn, execFileSync } = require('child_process');
const path = require('path');
const fs = require('fs');

const root = path.resolve(__dirname, '..');
const exe = path.join(root, 'compiler/target/release/micc' + (process.platform === 'win32' ? '.exe' : ''));
if (!fs.existsSync(exe)) {
  console.error('先运行 tools/build.ps1（缺 ' + exe + '）');
  process.exit(2);
}

const proc = spawn(exe, ['lsp'], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
let stderr = '';
proc.stderr.on('data', chunk => { stderr += chunk.toString('utf8'); });

let buffer = Buffer.alloc(0);
let nextId = 1;
const pending = new Map();
const diagWaiters = new Map();
const lastDiag = new Map();

function send(obj) {
  const body = Buffer.from(JSON.stringify(obj), 'utf8');
  proc.stdin.write(Buffer.concat([
    Buffer.from('Content-Length: ' + body.length + '\r\n\r\n', 'ascii'),
    body,
  ]));
}

function notify(method, params) {
  send({ jsonrpc: '2.0', method, params });
}

function request(method, params) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    send({ jsonrpc: '2.0', id, method, params });
  });
}

function dispatch(msg) {
  if (msg.id !== undefined && msg.id !== null) {
    const waiter = pending.get(msg.id);
    if (waiter) {
      pending.delete(msg.id);
      if (msg.error) {
        waiter.reject(new Error(msg.error.message || 'JSON-RPC 错误'));
      } else {
        waiter.resolve(msg.result);
      }
    }
    return;
  }
  if (msg.method === 'textDocument/publishDiagnostics') {
    const uri = msg.params.uri;
    lastDiag.set(uri, msg.params.diagnostics || []);
    const waiter = diagWaiters.get(uri);
    if (waiter) {
      diagWaiters.delete(uri);
      waiter(lastDiag.get(uri));
    }
  }
}

proc.stdout.on('data', chunk => {
  buffer = Buffer.concat([buffer, chunk]);
  while (true) {
    const end = buffer.indexOf('\r\n\r\n');
    if (end < 0) return;
    const header = buffer.slice(0, end).toString('ascii');
    const match = /Content-Length:\s*(\d+)/i.exec(header);
    if (!match) {
      buffer = buffer.slice(end + 4);
      continue;
    }
    const length = Number(match[1]);
    if (buffer.length < end + 4 + length) return;
    const body = buffer.slice(end + 4, end + 4 + length).toString('utf8');
    buffer = buffer.slice(end + 4 + length);
    try {
      dispatch(JSON.parse(body));
    } catch (e) {
      console.error('无法解析响应：' + body);
    }
  }
});

let pass = 0;
let fail = 0;
function check(name, ok, detail) {
  if (ok) {
    pass++;
    console.log('[通过] ' + name);
  } else {
    fail++;
    console.log('[不通过] ' + name);
    if (detail) console.log(detail);
  }
}

function waitDiagnostics(uri) {
  return new Promise(resolve => {
    diagWaiters.set(uri, resolve);
  });
}

async function main() {
  const rootUri = 'file:///' + root.replace(/\\/g, '/');
  const init = await request('initialize', {
    processId: process.pid,
    rootUri,
    capabilities: {},
  });
  const caps = init.capabilities || {};
  check(
    'initialize：声明 textDocumentSync / 定义 / 格式化',
    caps.textDocumentSync === 1 && caps.definitionProvider === true &&
      caps.documentFormattingProvider === true && !!caps.completionProvider,
    JSON.stringify(init)
  );
  notify('initialized', {});

  const docUri = rootUri + '/build/lsp-测试.jsh';
  let waiting = waitDiagnostics(docUri);
  notify('textDocument/didOpen', {
    textDocument: { uri: docUri, languageId: 'li', version: 1, text: '令 = 1\n' },
  });
  let diags = await waiting;
  const code = diags.length > 0 ? String(diags[0].code) : '';
  check(
    'didOpen 坏源码：返回编译诊断（含错误码）',
    diags.length > 0 && /^E\d+$/.test(code) && diags[0].range.start.line >= 0,
    JSON.stringify(diags)
  );

  waiting = waitDiagnostics(docUri);
  notify('textDocument/didChange', {
    textDocument: { uri: docUri, version: 2 },
    contentChanges: [{ text: '令 x = 1\n打印(x)\n' }],
  });
  diags = await waiting;
  check('didChange 好源码：诊断清空', diags.length === 0, JSON.stringify(diags));

  const comp = await request('textDocument/completion', {
    textDocument: { uri: docUri },
    position: { line: 0, character: 0 },
  });
  const labels = (comp.items || []).map(item => item.label);
  check(
    'completion：含关键字「令」与内建「打印」',
    labels.includes('令') && labels.includes('打印') && labels.includes('如果'),
    labels.join(', ')
  );

  const defUri = rootUri + '/build/lsp-定义.jsh';
  waiting = waitDiagnostics(defUri);
  notify('textDocument/didOpen', {
    textDocument: {
      uri: defUri,
      languageId: 'li',
      version: 1,
      text: '函数 甲():\n    返回 1\n打印(甲())\n',
    },
  });
  await waiting;
  const loc = await request('textDocument/definition', {
    textDocument: { uri: defUri },
    position: { line: 2, character: 3 },
  });
  check(
    'definition：甲() → 第 1 行定义',
    !!loc && loc.uri === defUri && loc.range.start.line === 0,
    JSON.stringify(loc)
  );

  waiting = waitDiagnostics(defUri);
  notify('textDocument/didChange', {
    textDocument: { uri: defUri, version: 2 },
    contentChanges: [{ text: '函数 甲():\n  令 x=1+2\n' }],
  });
  await waiting;
  const edits = await request('textDocument/formatting', {
    textDocument: { uri: defUri },
    options: { tabSize: 4, insertSpaces: true },
  });
  const expect = '函数 甲():\n    令 x = 1 + 2\n';
  check(
    'formatting：缩进与运算符空格归一',
    Array.isArray(edits) && edits.length === 1 && edits[0].newText === expect,
    JSON.stringify(edits)
  );

  const fmtFile = path.join(root, 'build/lsp-format.jsh');
  fs.mkdirSync(path.dirname(fmtFile), { recursive: true });
  fs.writeFileSync(fmtFile, '函数 甲():\n  令 x=1+2\n', 'utf8');
  const fmtOut = execFileSync(exe, ['--format', fmtFile], { encoding: 'utf8' });
  check('--format：与 LSP 格式化结果一致', fmtOut === expect, JSON.stringify(fmtOut));

  const shut = await request('shutdown', null);
  check('shutdown：返回 null', shut === null, JSON.stringify(shut));
  notify('exit', null);
  const exitCode = await new Promise(resolve => {
    proc.on('exit', code2 => resolve(code2));
    setTimeout(() => {
      try { proc.kill(); } catch (e) { /* 已退出 */ }
      resolve(-1);
    }, 5000);
  });
  check('exit：进程正常退出（0）', exitCode === 0, '退出码 ' + exitCode + (stderr ? '；stderr：' + stderr : ''));

  console.log('LSP 用例：' + pass + ' 通过，' + fail + ' 不通过');
  process.exit(fail > 0 ? 1 : 0);
}

main().catch(err => {
  console.error('[不通过] ' + (err && err.message ? err.message : err));
  if (stderr) console.error(stderr);
  try { proc.kill(); } catch (e) { /* 已退出 */ }
  process.exit(1);
});
