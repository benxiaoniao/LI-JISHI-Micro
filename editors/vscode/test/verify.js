// 编辑器侧验证：直接调用扩展编译产物 out/core.js + 真实 micc/jmrun。
// 运行：node test/verify.js
const assert = require('assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');
const core = require('../out/core');

const root = path.resolve(__dirname, '..', '..', '..');
const micc = path.join(root, 'compiler', 'target', 'release', 'micc.exe');
const jmrun = path.join(root, 'build', 'jmrun.exe');

let passed = 0;
let failed = 0;
function check(name, fn) {
  try {
    fn();
    passed++;
    console.log('[通过] ' + name);
  } catch (error) {
    failed++;
    console.log('[不通过] ' + name + '\n  ' + error.message);
  }
}

function run(exe, args) {
  const result = spawnSync(exe, args, { cwd: root, encoding: 'utf8', windowsHide: true });
  return { code: result.status, out: result.stdout || '', err: result.stderr || '' };
}

function writeUtf8(file, text, bom = false) {
  fs.writeFileSync(file, (bom ? '\uFEFF' : '') + text, 'utf8');
}

if (!fs.existsSync(micc) || !fs.existsSync(jmrun)) {
  console.error('先运行 tools\\build.ps1 生成 micc 与 jmrun');
  process.exit(2);
}

const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'li-editor-'));
try {
  // ---------- 1. 同文件：函数/类/方法跳转 ----------
  const symbols = core.parseSymbols(run(micc, ['--symbols', path.join(root, 'examples', '类.jsh')]).out);
  check('同文件：类 计数器 在第 1 行', () => {
    const target = core.findSymbol(symbols, '计数器');
    assert(target && target.kind === '类' && target.line === 1);
  });
  check('同文件：方法 加 在第 7 行', () => {
    const target = core.findSymbol(symbols, '加', ['方法']);
    assert(target && target.line === 7);
  });
  check('同文件：无同名定义返回 undefined', () => {
    assert.strictEqual(core.findSymbol(symbols, '不存在的名字'), undefined);
  });
  check('decideDefinition：裸名字跳到本文件', () => {
    const decision = core.decideDefinition({
      word: '加',
      file: path.join(root, 'examples', '类.jsh'),
      local: symbols,
      packs: [],
    });
    assert(decision && decision.kind === '本文件' && decision.line === 7);
  });

  // ---------- 2. 跨包：工具.加一 ----------
  const pkgRoot = path.join(sandbox, '跨包');
  const toolDir = path.join(pkgRoot, '工具');
  const appDir = path.join(pkgRoot, '应用');
  fs.mkdirSync(toolDir, { recursive: true });
  fs.mkdirSync(appDir, { recursive: true });
  writeUtf8(
    path.join(toolDir, '砾包.toml'),
    '[包]\n名字 = "工具"\n版本 = "1.0.0"\n入口 = "工具.jsh"\n\n[导出]\n名字 = ["加一"]\n',
    true
  );
  writeUtf8(path.join(toolDir, '工具.jsh'), '函数 加一(数)：\n    返回 数 + 1\n\n令 基 = 41\n');
  writeUtf8(
    path.join(appDir, '砾包.toml'),
    '[包]\n名字 = "应用"\n版本 = "1.0.0"\n入口 = "应用.jsh"\n\n[依赖]\n工具 = { 路径 = "../工具", 版本 = "^1.0.0" }\n',
    true
  );
  writeUtf8(path.join(appDir, '应用.jsh'), '打印(工具.加一(1))\n');

  const appFile = path.join(appDir, '应用.jsh');
  const packs = core.parsePackFiles(run(micc, ['--pack-files', appDir]).out);
  check('跨包：--pack-files 解析出 工具 与 应用', () => {
    assert.strictEqual(packs.length, 2);
    assert(packs.some(p => p.name === '工具') && packs.some(p => p.name === '应用'));
  });
  check('跨包：最近 manifest 定位到 应用', () => {
    const manifest = core.nearestManifest(appFile, root);
    assert(manifest && path.resolve(manifest) === path.resolve(appDir));
  });

  const toolEntry = packs.find(p => p.name === '工具').entry;
  const toolSymbols = core.parseSymbols(run(micc, ['--symbols', toolEntry]).out);
  const remote = new Map([[toolEntry, toolSymbols]]);
  check('跨包：工具.加一 跳到工具包入口第 1 行', () => {
    const decision = core.decideDefinition({
      word: '加一',
      receiver: '工具',
      file: appFile,
      local: core.parseSymbols(run(micc, ['--symbols', appFile]).out),
      packs,
      remote,
    });
    assert(decision, '没有解析到定义');
    assert.strictEqual(decision.kind, '包');
    assert.strictEqual(path.resolve(decision.file), path.resolve(toolEntry));
    assert.strictEqual(decision.line, 1);
  });
  check('跨包：裸包名跳到依赖入口', () => {
    const decision = core.decideDefinition({
      word: '工具',
      file: appFile,
      local: [],
      packs,
    });
    assert(decision && decision.kind === '包' && path.resolve(decision.file) === path.resolve(toolEntry));
  });
  check('跨包：不存在的导出返回 undefined', () => {
    const decision = core.decideDefinition({
      word: '没有这个',
      receiver: '工具',
      file: appFile,
      local: [],
      packs,
      remote,
    });
    assert.strictEqual(decision, undefined);
  });

  // ---------- 3. 导入的原生模块 → 语言卡 ----------
  const cardFile = path.join(root, 'spec', 'langcard.json');
  const cardText = fs.readFileSync(cardFile, 'utf8');
  const moduleLine = core.findModuleLine(cardText, '引脚');
  check('原生模块：语言卡里能找到 引脚 的声明行', () => {
    assert(moduleLine && moduleLine > 1);
  });
  check('原生模块：decideDefinition 返回语言卡位置', () => {
    const decision = core.decideDefinition({
      word: '引脚',
      file: path.join(root, 'examples', '点灯.jsh'),
      local: [],
      packs: [],
      langcardFile: cardFile,
      moduleLine,
    });
    assert(decision && decision.kind === '原生模块' && decision.file === cardFile);
  });
  check('原生模块：没有导入时不给定义', () => {
    assert.strictEqual(core.findModuleLine(cardText, '不存在的模块'), undefined);
  });

  // ---------- 4. 运行期错误定位 ----------
  const badSrc = path.join(sandbox, '错误.jsh');
  writeUtf8(badSrc, '令 甲 = 1\n令 乙 = 0\n打印(甲 // 乙)\n');
  const badJmb = path.join(sandbox, '错误.jmb');
  run(micc, [badSrc, '-o', badJmb]);
  const badRun = run(jmrun, [badJmb]);
  const badParsed = core.parseRunOutput(badRun.out + badRun.err);
  check('运行期错误：解析出 E007', () => {
    assert(badParsed.error && badParsed.error.code === 'E007');
  });
  check('运行期错误：定位到错误.jsh 第 3 行', () => {
    assert(badParsed.error && badParsed.error.site);
    assert(badParsed.error.site.line === 3, '实际 ' + JSON.stringify(badParsed.error.site));
    assert(path.resolve(root, badParsed.error.site.file) === path.resolve(badSrc));
  });

  const stripJmb = path.join(sandbox, '错误-剥离.jmb');
  run(micc, [badSrc, '-o', stripJmb, '--strip-debug']);
  const stripRun = run(jmrun, [stripJmb]);
  const stripParsed = core.parseRunOutput(stripRun.out + stripRun.err);
  check('无调试段：仍有 E007，但没有源码行', () => {
    assert(stripParsed.error && stripParsed.error.code === 'E007');
    assert.strictEqual(stripParsed.error.site, undefined);
    assert.strictEqual(stripRun.code, 1);
  });

  // ---------- 5. 断点命中与未命中 ----------
  const classJmb = path.join(sandbox, '类.jmb');
  const compiled = run(micc, [path.join('examples', '类.jsh'), '-o', classJmb]);
  check('断点夹具编译成功', () => assert.strictEqual(compiled.code, 0));
  const hit = run(jmrun, [classJmb, '--break', 'examples/类.jsh:32']);
  const hitParsed = core.parseRunOutput(hit.out + hit.err);
  check('断点命中：停在第 32 行', () => {
    assert(hitParsed.hit, '没有命中：' + hit.out + hit.err);
    assert.strictEqual(hitParsed.hit.line, 32);
    assert.strictEqual(hit.code, 0);
  });
  const miss = run(jmrun, [classJmb, '--break', 'examples/类.jsh:999']);
  const missParsed = core.parseRunOutput(miss.out + miss.err);
  check('断点未命中：退出码 2，提示断点无法落地', () => {
    assert.strictEqual(missParsed.hit, undefined);
    assert.strictEqual(miss.code, 2);
    assert((miss.out + miss.err).includes('断点无法落地'));
  });

  // ---------- 6. 相对路径 ----------
  check('displayPath：工作区内转相对路径', () => {
    const text = core.displayPath(root, path.join(root, 'examples', '类.jsh'));
    assert.strictEqual(text, 'examples/类.jsh');
  });
} finally {
  fs.rmSync(sandbox, { recursive: true, force: true });
}

console.log(`\n${passed} 通过，${failed} 不通过`);
process.exit(failed === 0 ? 0 : 1);
