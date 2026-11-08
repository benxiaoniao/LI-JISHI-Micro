// 纯逻辑：解析 micc / jmrun 的文本输出，不依赖 vscode，便于命令行测试。
import * as fs from 'fs';
import * as path from 'path';

export interface SymbolInfo {
  kind: string;
  name: string;
  line: number;
  col: number;
}

export interface PackInfo {
  name: string;
  version: string;
  entry: string;
}

export interface Site {
  file: string;
  line: number;
}

export interface RunError {
  code?: string;
  message: string;
  codeIndex?: number;
  offset?: number;
  site?: Site;
}

/** `micc --symbols` 的制表符输出 → 定义列表。 */
export function parseSymbols(text: string): SymbolInfo[] {
  const out: SymbolInfo[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/^\uFEFF/, '').trim();
    if (!line) {
      continue;
    }
    const parts = line.split('\t');
    if (parts.length < 4) {
      continue;
    }
    const lineNo = parseInt(parts[2], 10);
    const colNo = parseInt(parts[3], 10);
    if (!Number.isFinite(lineNo) || !Number.isFinite(colNo)) {
      continue;
    }
    out.push({ kind: parts[0], name: parts[1], line: lineNo, col: colNo });
  }
  return out;
}

/** `micc --pack-files` 的输出 → 包与入口文件。 */
export function parsePackFiles(text: string): PackInfo[] {
  const out: PackInfo[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/^\uFEFF/, '').trim();
    if (!line) {
      continue;
    }
    const parts = line.split('\t');
    if (parts.length < 3) {
      continue;
    }
    out.push({ name: parts[0], version: parts[1], entry: parts[2] });
  }
  return out;
}

/** jmrun 输出 → 运行期错误、断点命中位置。 */
export function parseRunOutput(text: string): { error?: RunError; hit?: Site } {
  const result: { error?: RunError; hit?: Site } = {};

  const err = /错误\[([A-Z]\d+)\]\s*(.*?)（代码\s*(\d+)\s*偏移\s*(\d+)）/.exec(text);
  if (err) {
    result.error = {
      code: err[1],
      message: err[2].trim(),
      codeIndex: parseInt(err[3], 10),
      offset: parseInt(err[4], 10),
    };
  }

  const site = /^于\s+(.+):(\d+)\s*$/m.exec(text);
  if (site) {
    const parsed = { file: site[1].trim(), line: parseInt(site[2], 10) };
    if (result.error) {
      result.error.site = parsed;
    } else {
      result.error = { message: '运行期错误（未给出错误码）', site: parsed };
    }
  }

  const hit = /^断点：(.+):(\d+)\s*$/m.exec(text);
  if (hit) {
    result.hit = { file: hit[1].trim(), line: parseInt(hit[2], 10) };
  }

  return result;
}

/** 在定义表里按名字找；同名的优先取靠前的定义。 */
export function findSymbol(symbols: SymbolInfo[], name: string, kinds?: string[]): SymbolInfo | undefined {
  const allowed = kinds ? new Set(kinds) : undefined;
  for (const symbol of symbols) {
    if (symbol.name !== name) {
      continue;
    }
    if (allowed && !allowed.has(symbol.kind)) {
      continue;
    }
    return symbol;
  }
  return undefined;
}

/** 从文件向上找最近一个 `砾包.toml`，返回它所在目录。 */
export function nearestManifest(filePath: string, stopAt?: string): string | undefined {
  let dir = path.dirname(filePath);
  const stop = stopAt ? path.resolve(stopAt) : undefined;
  while (true) {
    if (fs.existsSync(path.join(dir, '砾包.toml'))) {
      return dir;
    }
    const parent = path.dirname(dir);
    if (parent === dir) {
      return undefined;
    }
    if (stop && path.resolve(dir) === stop) {
      return undefined;
    }
    dir = parent;
  }
}

/** 在语言卡文本里找原生模块声明所在行（1 起），找不到返回 undefined。 */
export function findModuleLine(text: string, moduleName: string): number | undefined {
  const escaped = moduleName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp(`"module"\\s*:\\s*"${escaped}"`);
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    if (pattern.test(lines[i])) {
      return i + 1;
    }
  }
  return undefined;
}

/** 把相对工作区的路径原样保留，绝对路径转成相对的 `/` 形式（用来喂给 jmrun --break）。 */
export function displayPath(root: string, filePath: string): string {
  const rel = path.relative(root, filePath);
  if (rel && !rel.startsWith('..') && !path.isAbsolute(rel)) {
    return rel.split(path.sep).join('/');
  }
  return filePath.split(path.sep).join('/');
}

/** 一行里的词法片断：字符串、注释、名字、数字、运算符、标点。 */
interface FmtToken {
  kind: 'word' | 'num' | 'str' | 'op' | 'punct' | 'com';
  text: string;
  /** 行内注释前原有的空格数，格式化时原样保留，避免把对齐打散。 */
  gap?: number;
}

const MULTI_OPS = ['==', '!=', '<=', '>=', '+=', '-=', '*=', '/=', '%=', '**', '//'];
const PUNCT_CHARS = '()[]{},.:：，、；';
const NO_SPACE_BEFORE = ')]}.,:：，、；';

function tokenizeLine(line: string): FmtToken[] {
  const toks: FmtToken[] = [];
  let i = 0;
  while (i < line.length) {
    const ch = line[i];
    if (ch === ' ' || ch === '\t' || ch === '\u3000') {
      i++;
      continue;
    }
    if (ch === '#') {
      let gap = 0;
      let k = i - 1;
      while (k >= 0 && (line[k] === ' ' || line[k] === '\t' || line[k] === '\u3000')) {
        gap++;
        k--;
      }
      toks.push({ kind: 'com', text: line.slice(i), gap: Math.max(1, gap) });
      break;
    }
    if (ch === '"') {
      let j = i + 1;
      while (j < line.length && line[j] !== '"') {
        j += line[j] === '\\' ? 2 : 1;
      }
      j = Math.min(j + 1, line.length);
      toks.push({ kind: 'str', text: line.slice(i, j) });
      i = j;
      continue;
    }
    if (ch === '“' || ch === '‘') {
      const close = ch === '“' ? '”' : '’';
      let j = i + 1;
      while (j < line.length && line[j] !== close) {
        j++;
      }
      j = Math.min(j + 1, line.length);
      toks.push({ kind: 'str', text: line.slice(i, j) });
      i = j;
      continue;
    }
    if (/[0-9]/.test(ch)) {
      let j = i + 1;
      while (j < line.length && /[0-9]/.test(line[j])) {
        j++;
      }
      if (line[j] === '.' && /[0-9]/.test(line[j + 1] ?? '')) {
        j += 2;
        while (j < line.length && /[0-9]/.test(line[j])) {
          j++;
        }
      }
      toks.push({ kind: 'num', text: line.slice(i, j) });
      i = j;
      continue;
    }
    if (/[\p{L}_]/u.test(ch)) {
      let j = i + 1;
      while (j < line.length && /[\p{L}\p{N}_]/u.test(line[j])) {
        j++;
      }
      toks.push({ kind: 'word', text: line.slice(i, j) });
      i = j;
      continue;
    }
    const rest = line.slice(i);
    const op = MULTI_OPS.find(candidate => rest.startsWith(candidate));
    if (op) {
      toks.push({ kind: 'op', text: op });
      i += op.length;
      continue;
    }
    if ('+-*/%<>='.includes(ch)) {
      toks.push({ kind: 'op', text: ch });
      i++;
      continue;
    }
    if (PUNCT_CHARS.includes(ch)) {
      toks.push({ kind: 'punct', text: ch });
      i++;
      continue;
    }
    // 认不出的字符原样保留（宁可不动，也不要吞掉用户代码）
    toks.push({ kind: 'punct', text: ch });
    i++;
  }
  return toks;
}

/** `[` 前面是这些关键字时是数组字面量，要留空格；是普通名字时按索引处理。 */
const LIST_KEYWORDS = new Set(['返回', '令', '在', '导入', '创建', '新建', '抛出', '捕获', '如果', '当', '循环', '遍历', '否则', '类', '函数']);

/** 这些关键字后面跟的是表达式，`-` / `+` 按一元号处理。 */
const EXPR_KEYWORDS = new Set(['返回', '令', '在', '如果', '当', '循环', '遍历', '否则', '抛出', '捕获', '创建', '新建', '打印', '类', '函数', '导入']);

function isValueEnd(tok: FmtToken): boolean {
  if (tok.kind === 'word' && EXPR_KEYWORDS.has(tok.text)) {
    return false;
  }
  return tok.kind === 'word' || tok.kind === 'num' || tok.kind === 'str' ||
    (tok.kind === 'punct' && (tok.text === ')' || tok.text === ']' || tok.text === '}'));
}

function needSpace(prev: FmtToken, prevUnary: boolean, tok: FmtToken): boolean {
  if (tok.kind === 'com') {
    return false; // 注释前的空格在 formatTokens 里按原样保留
  }
  if (tok.kind === 'punct' && NO_SPACE_BEFORE.includes(tok.text)) {
    return false;
  }
  if (tok.kind === 'punct' && tok.text === '(') {
    if (prev.kind === 'op' || prev.text === ',' || prev.text === ':' || prev.text === '：') {
      return true;
    }
    return false;
  }
  if (tok.kind === 'punct' && tok.text === '[') {
    if (prev.kind === 'word') {
      return LIST_KEYWORDS.has(prev.text);
    }
    if (prev.kind === 'op' || prev.text === ',' || prev.text === ':' || prev.text === '：') {
      return true;
    }
    return false;
  }
  if (prev.kind === 'op') {
    return !prevUnary;
  }
  if (prev.kind === 'punct') {
    if (prev.text === '(' || prev.text === '[' || prev.text === '{' || prev.text === '.') {
      return false;
    }
    return true; // 逗号、冒号后面留一个空格
  }
  if (tok.kind === 'op') {
    if (tok.text !== '-' && tok.text !== '+') {
      return true;
    }
    if (isValueEnd(prev)) {
      return true; // 二元减号
    }
    // 一元正负号：前面是关键字/名字时留一个空格（如 `返回 -1`），紧跟括号或运算符时不加
    return prev.kind === 'word';
  }
  return true;
}

function formatTokens(toks: FmtToken[]): string {
  let out = '';
  let prev: FmtToken | undefined;
  let prevUnary = false;
  for (const tok of toks) {
    if (prev) {
      if (tok.kind === 'com') {
        out += ' '.repeat(tok.gap ?? 1);
      } else if (needSpace(prev, prevUnary, tok)) {
        out += ' ';
      }
    }
    out += tok.text;
    prevUnary = tok.kind === 'op' && (tok.text === '-' || tok.text === '+') &&
      !(prev && isValueEnd(prev));
    prev = tok;
  }
  return out;
}

function countIndent(lead: string): number {
  let n = 0;
  for (const ch of lead) {
    n += ch === '\t' ? 4 : ch === '\u3000' ? 2 : 1;
  }
  return n;
}

/**
 * 最小格式化：缩进按 4 空格归一（块层级沿用原缩进结构），运算符两侧各留一个空格，
 * 字符串、注释和空行原样保留。纯函数，`formatSource(formatSource(x)) === formatSource(x)`。
 */
export function formatSource(text: string): string {
  const lines = text.split(/\r?\n/);
  const out: string[] = [];
  const stack = [0];
  for (const raw of lines) {
    const stripped = raw.replace(/[ \t\u3000]+$/, '');
    if (stripped.trim() === '') {
      out.push('');
      continue;
    }
    const lead = /^[ \t\u3000]*/.exec(raw)?.[0] ?? '';
    const orig = countIndent(lead);
    while (stack.length > 1 && orig < stack[stack.length - 1]) {
      stack.pop();
    }
    if (orig > stack[stack.length - 1]) {
      stack.push(orig);
    }
    const level = stack.length - 1;
    out.push('    '.repeat(level) + formatTokens(tokenizeLine(stripped.trim())));
  }
  const result = out.join('\n');
  return result;
}

export interface Decision {
  kind: '本文件' | '包' | '原生模块';
  file: string;
  line: number;
  col: number;
}

/**
 * 跳转决策（纯函数，命令与测试共用同一套逻辑）：
 * - `接收者.名字` 先按包名查依赖入口，再按本文件的方法/字段；
 * - 裸名字按本文件定义；
 * - 裸包名跳依赖入口；
 * - `导入 模块` 跳到语言卡里的模块声明。
 */
export function decideDefinition(input: {
  word: string;
  receiver?: string;
  file: string;
  local: SymbolInfo[];
  packs: PackInfo[];
  remote?: Map<string, SymbolInfo[]>;
  langcardFile?: string;
  moduleLine?: number;
}): Decision | undefined {
  const { word, receiver, file, local, packs, remote } = input;

  if (receiver) {
    const pkg = packs.find(p => p.name === receiver);
    if (pkg && remote) {
      const target = findSymbol(remote.get(pkg.entry) ?? [], word, ['函数', '令', '类']);
      if (target) {
        return { kind: '包', file: pkg.entry, line: target.line, col: target.col };
      }
    }
    const member = findSymbol(local, word, ['方法', '字段']);
    return member ? { kind: '本文件', file, line: member.line, col: member.col } : undefined;
  }

  // `导入 X` 是声明本身，不当作引用目标
  const direct = findSymbol(local, word, ['令', '函数', '类', '方法', '字段']);
  if (direct) {
    return { kind: '本文件', file, line: direct.line, col: direct.col };
  }

  const pkg = packs.find(p => p.name === word);
  if (pkg) {
    return { kind: '包', file: pkg.entry, line: 1, col: 1 };
  }

  if (input.moduleLine && input.langcardFile) {
    return { kind: '原生模块', file: input.langcardFile, line: input.moduleLine, col: 1 };
  }
  return undefined;
}

