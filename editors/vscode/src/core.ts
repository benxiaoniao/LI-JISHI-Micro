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

