// micc dap 回归：通过 stdio 走 DAP，覆盖初始化、断点、条件、暂停、栈、变量、单步、继续、停止。
// 与 VS Code 解耦：`node tools/test-dap.js` 可直接复现。
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

const root = path.resolve(__dirname, '..');
const exe = path.join(root, 'compiler/target/release/micc' + (process.platform === 'win32' ? '.exe' : ''));
const jmrun = path.join(root, 'build/jmrun' + (process.platform === 'win32' ? '.exe' : ''));
if (!fs.existsSync(exe)) {
  console.error('先运行 tools/build.ps1（缺 ' + exe + '）');
  process.exit(2);
}
if (!fs.existsSync(jmrun)) {
  console.error('先运行 tools/build.ps1（缺 ' + jmrun + '）');
  process.exit(2);
}

const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'li-dap-'));
const fixture = path.join(sandbox, 'dap-case.jsh');
fs.writeFileSync(
  fixture,
  ['函数 跑():', '    令 k = 0', '    当 k < 5:', '        k += 1', '        打印("k =", k)', '    令 名 = "甲"', '    打印(名)', '跑()'].join('\n') + '\n',
  'utf8'
);

const breakLine = 5;

function spawnAdapter() {
  const proc = spawn(exe, ['dap'], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
  let buffer = Buffer.alloc(0);
  let nextId = 1;
  let closed = false;
  const pending = new Map();
  const events = [];
  const waiters = [];

  proc.stderr.on('data', () => {});

  function send(obj) {
    const body = Buffer.from(JSON.stringify(obj), 'utf8');
    proc.stdin.write(Buffer.concat([
      Buffer.from('Content-Length: ' + body.length + '\r\n\r\n', 'ascii'),
      body,
    ]));
  }

  function dispatch(msg) {
    if (msg.type === 'response') {
      const waiter = pending.get(msg.request_seq);
      if (waiter) {
        pending.delete(msg.request_seq);
        waiter.resolve(msg);
      }
      return;
    }
    if (msg.type === 'event') {
      const index = waiters.findIndex(w => w.name === msg.event);
      if (index >= 0) {
        const waiter = waiters.splice(index, 1)[0];
        waiter.resolve(msg);
      } else {
        events.push(msg);
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
        console.error('无法解析 DAP 响应：' + body);
      }
    }
  });

  const closePromise = new Promise(resolve => {
    proc.on('close', code => {
      closed = true;
      for (const waiter of pending.values()) {
        waiter.reject(new Error('适配器已退出'));
      }
      pending.clear();
      for (const waiter of waiters.splice(0)) {
        waiter.reject(new Error('适配器已退出，等待事件失败：' + waiter.name));
      }
      resolve(code);
    });
  });

  function request(command, args) {
    const seq = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(seq, { resolve, reject });
      send({ seq, type: 'request', command, arguments: args || {} });
    });
  }

  function waitEvent(name, timeout = 15000) {
    const index = events.findIndex(event => event.event === name);
    if (index >= 0) {
      return Promise.resolve(events.splice(index, 1)[0]);
    }
    if (closed) {
      return Promise.reject(new Error('适配器已退出，等不到事件：' + name));
    }
    return new Promise((resolve, reject) => {
      const waiter = { name, resolve, reject };
      waiters.push(waiter);
      const timer = setTimeout(() => {
        const at = waiters.indexOf(waiter);
        if (at >= 0) {
          waiters.splice(at, 1);
        }
        reject(new Error('等待事件超时：' + name));
      }, timeout);
      const originalResolve = waiter.resolve;
      waiter.resolve = value => {
        clearTimeout(timer);
        originalResolve(value);
      };
    });
  }

  function hasEvent(name) {
    return events.some(event => event.event === name);
  }

  function countEvent(name) {
    return events.filter(event => event.event === name).length;
  }

  async function dispose() {
    if (!closed) {
      proc.stdin.end();
    }
    await Promise.race([
      closePromise,
      new Promise(resolve => setTimeout(resolve, 3000)),
    ]);
    if (!closed) {
      proc.kill();
      await closePromise;
    }
  }

  return { request, waitEvent, hasEvent, countEvent, dispose, proc, closePromise };
}

async function initialize(client) {
  return client.request('initialize', {
    clientID: 'li-dap-test',
    adapterID: 'li',
    linesStartAt1: true,
    columnsStartAt1: true,
    pathFormat: 'path',
  });
}

async function startWithBreakpoint(client, condition) {
  await client.request('launch', {
    program: fixture,
    cwd: root,
    miccPath: exe,
    jmrunPath: jmrun,
  });
  const initialized = await client.waitEvent('initialized', 5000);
  const set = await client.request('setBreakpoints', {
    source: { path: fixture },
    breakpoints: [{ line: breakLine, condition }],
  });
  await client.request('configurationDone', {});
  return { set, initialized };
}

let total = 0;
let failed = 0;

function check(name, ok, detail) {
  total++;
  if (ok) {
    console.log('[通过] ' + name);
  } else {
    failed++;
    console.log('[不通过] ' + name);
    if (detail) {
      console.log(detail);
    }
  }
}

async function main() {
  // 1. 初始化能力
  {
    const client = spawnAdapter();
    try {
      const response = await initialize(client);
      const caps = response.body || {};
      check('initialize 返回条件断点能力', caps.supportsConditionalBreakpoints === true, JSON.stringify(caps));
      check('initialize 返回 configurationDone 能力', caps.supportsConfigurationDoneRequest === true, JSON.stringify(caps));
    } finally {
      await client.dispose();
    }
  }

  // 2-7. 条件断点 → 暂停 → 栈/变量 → 下一行 → 继续
  {
    const client = spawnAdapter();
    try {
      await initialize(client);
      const started = await startWithBreakpoint(client, 'k == 3');
      check('launch 后收到 initialized 事件', !!started.initialized, JSON.stringify(started.initialized));
      const set = started.set;
      const bp = set.body && set.body.breakpoints && set.body.breakpoints[0];
      check('setBreakpoints 返回 verified', !!(bp && bp.verified), JSON.stringify(set.body));

      const stopped = await client.waitEvent('stopped');
      check('条件断点命中（reason=breakpoint）', stopped.body && stopped.body.reason === 'breakpoint', JSON.stringify(stopped.body));

      const threads = await client.request('threads', {});
      check('threads 返回单个线程', !!(threads.body && threads.body.threads && threads.body.threads.length === 1), JSON.stringify(threads.body));

      const stack = await client.request('stackTrace', { threadId: 1, startFrame: 0, levels: 20 });
      const frame = stack.body && stack.body.stackFrames && stack.body.stackFrames[0];
      check('stackTrace 停在第 5 行', !!(frame && frame.line === breakLine), JSON.stringify(stack.body));
      check('stackTrace 函数名包含「跑」', !!(frame && String(frame.name).includes('跑')), JSON.stringify(frame));
      check('stackTrace 源文件路径正确', !!(frame && frame.source && String(frame.source.path).endsWith('dap-case.jsh')), JSON.stringify(frame && frame.source));

      const scopes = await client.request('scopes', { frameId: frame ? frame.id : 1 });
      const scope = scopes.body && scopes.body.scopes && scopes.body.scopes[0];
      check('scopes 返回局部变量域', !!(scope && scope.variablesReference === 1), JSON.stringify(scopes.body));

      const variables = await client.request('variables', { variablesReference: scope ? scope.variablesReference : 1 });
      const list = (variables.body && variables.body.variables) || [];
      check('variables 读到 k = 3', list.some(item => item.name === 'k' && item.value === '3'), JSON.stringify(list));
      check('一次暂停只发一个 stopped 事件', client.countEvent('stopped') === 0, '多出 ' + client.countEvent('stopped'));

      await client.request('next', { threadId: 1 });
      const stepped = await client.waitEvent('stopped');
      check('next 后再次暂停（reason=step）', stepped.body && stepped.body.reason === 'step', JSON.stringify(stepped.body));
      const stack2 = await client.request('stackTrace', { threadId: 1, startFrame: 0, levels: 20 });
      const frame2 = stack2.body && stack2.body.stackFrames && stack2.body.stackFrames[0];
      check('next 后行号变化', !!(frame2 && frame2.line !== breakLine), JSON.stringify(stack2.body));
      check('next 后没有多余 stopped', client.countEvent('stopped') === 0, '多出 ' + client.countEvent('stopped'));

      await client.request('continue', { threadId: 1 });
      const exited = await client.waitEvent('exited', 20000);
      await client.waitEvent('terminated', 20000);
      check('continue 后正常退出（exitCode=0）', exited.body && exited.body.exitCode === 0, JSON.stringify(exited.body));
    } finally {
      await client.dispose();
    }
  }

  // 8. 条件不成立：不停，直接结束
  {
    const client = spawnAdapter();
    try {
      await initialize(client);
      await startWithBreakpoint(client, 'k == 99');
      const terminated = await client.waitEvent('terminated', 20000);
      check('条件不成立直接结束', !!terminated, JSON.stringify(terminated.body));
      check('条件不成立没有 stopped', !client.hasEvent('stopped'));
    } finally {
      await client.dispose();
    }
  }

  // 9. 暂停中断开
  {
    const client = spawnAdapter();
    try {
      await initialize(client);
      await startWithBreakpoint(client, 'k == 3');
      await client.waitEvent('stopped');
      const response = await client.request('disconnect', {});
      check('disconnect 返回成功', response.success === true, JSON.stringify(response));
      await client.waitEvent('terminated', 10000);
      check('disconnect 后适配器退出', (await Promise.race([
        client.closePromise,
        new Promise(resolve => setTimeout(() => resolve('timeout'), 5000)),
      ])) !== 'timeout');
    } finally {
      await client.dispose();
    }
  }

  // 10. 断点上限
  {
    const client = spawnAdapter();
    try {
      await initialize(client);
      const breakpoints = [];
      for (let i = 0; i < 9; i++) {
        breakpoints.push({ line: i + 1 });
      }
      const response = await client.request('setBreakpoints', {
        source: { path: fixture },
        breakpoints,
      });
      const list = (response.body && response.body.breakpoints) || [];
      check('第 9 个断点标记未验证', !!(list[8] && list[8].verified === false), JSON.stringify(list));
    } finally {
      await client.dispose();
    }
  }

  console.log('\n===== DAP 用例 =====');
  console.log(`${total - failed} 通过，${failed} 不通过`);
}

main()
  .catch(error => {
    failed++;
    console.error(error && error.stack ? error.stack : error);
  })
  .finally(() => {
    try {
      fs.rmSync(sandbox, { recursive: true, force: true });
    } catch (e) {
      // 临时目录清理失败不影响结论
    }
    process.exit(failed > 0 ? 1 : 0);
  });
