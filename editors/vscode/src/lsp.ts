// 极简 LSP 客户端：只服务砾扩展，不引入 vscode-languageclient。
// 负责 spawn `micc lsp`、Content-Length 帧读写、请求/通知与诊断回调。
import { spawn, ChildProcessWithoutNullStreams } from 'child_process';

export interface LspPosition {
  line: number;
  character: number;
}

export interface LspRange {
  start: LspPosition;
  end: LspPosition;
}

export interface LspDiagnostic {
  range: LspRange;
  severity?: number;
  source?: string;
  code?: string | number;
  message: string;
}

export interface LspLocation {
  uri: string;
  range: LspRange;
}

export interface LspTextEdit {
  range: LspRange;
  newText: string;
}

type Handler = (params: any) => void;

export class LiLspClient {
  private proc?: ChildProcessWithoutNullStreams;
  private buffer = Buffer.alloc(0);
  private nextId = 1;
  private pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  private handlers = new Map<string, Handler[]>();
  private opened = new Set<string>();
  private ready = false;

  constructor(private exe: string, private cwd: string, private langcardPath?: string) {}

  start(): void {
    if (this.proc) {
      return;
    }
    this.proc = spawn(this.exe, ['lsp'], { cwd: this.cwd, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    this.proc.stdout.on('data', chunk => this.onData(chunk));
    this.proc.stderr.on('data', () => {
      // 服务器 stderr 只用于意外信息，不打断编辑
    });
    this.proc.on('exit', () => {
      for (const [, waiter] of this.pending) {
        waiter.reject(new Error('语言服务器已退出'));
      }
      this.pending.clear();
      this.proc = undefined;
      this.ready = false;
    });
    void this.request('initialize', {
      processId: process.pid,
      rootUri: 'file:///' + this.cwd.replace(/\\/g, '/'),
      capabilities: {},
      initializationOptions: this.langcardPath ? { langcardPath: this.langcardPath } : undefined,
    }).then(() => {
      this.ready = true;
      this.notify('initialized', {});
    }).catch(() => {
      this.ready = false;
    });
  }

  onNotification(method: string, handler: Handler): void {
    const list = this.handlers.get(method) ?? [];
    list.push(handler);
    this.handlers.set(method, list);
  }

  request(method: string, params: any): Promise<any> {
    if (!this.proc) {
      this.start();
    }
    if (!this.proc) {
      return Promise.reject(new Error('语言服务器未启动'));
    }
    const id = this.nextId++;
    return new Promise<any>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ jsonrpc: '2.0', id, method, params });
    });
  }

  notify(method: string, params: any): void {
    this.send({ jsonrpc: '2.0', method, params });
  }

  /** 把文档同步到服务器：首次 didOpen，之后 didChange（全文）。 */
  sync(uri: string, text: string, version: number): void {
    this.start();
    if (this.opened.has(uri)) {
      this.notify('textDocument/didChange', {
        textDocument: { uri, version },
        contentChanges: [{ text }],
      });
    } else {
      this.opened.add(uri);
      this.notify('textDocument/didOpen', {
        textDocument: { uri, languageId: 'li', version, text },
      });
    }
  }

  close(uri: string): void {
    if (!this.opened.delete(uri)) {
      return;
    }
    this.notify('textDocument/didClose', { textDocument: { uri } });
  }

  isReady(): boolean {
    return this.ready;
  }

  dispose(): void {
    if (!this.proc) {
      return;
    }
    try {
      this.notify('shutdown', null);
      this.notify('exit', null);
    } catch {
      // 进程可能已退出
    }
    const child = this.proc;
    this.proc = undefined;
    setTimeout(() => {
      try {
        child.kill();
      } catch {
        // 已退出
      }
    }, 500);
  }

  private send(obj: unknown): void {
    if (!this.proc) {
      return;
    }
    const body = Buffer.from(JSON.stringify(obj), 'utf8');
    const head = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, 'ascii');
    try {
      this.proc.stdin.write(Buffer.concat([head, body]));
    } catch {
      // 写失败按进程退出处理
    }
  }

  private onData(chunk: Buffer): void {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (true) {
      const end = this.buffer.indexOf('\r\n\r\n');
      if (end < 0) {
        return;
      }
      const header = this.buffer.slice(0, end).toString('ascii');
      const match = /Content-Length:\s*(\d+)/i.exec(header);
      if (!match) {
        this.buffer = this.buffer.slice(end + 4);
        continue;
      }
      const length = Number(match[1]);
      if (this.buffer.length < end + 4 + length) {
        return;
      }
      const body = this.buffer.slice(end + 4, end + 4 + length).toString('utf8');
      this.buffer = this.buffer.slice(end + 4 + length);
      let msg: any;
      try {
        msg = JSON.parse(body);
      } catch {
        continue;
      }
      this.dispatch(msg);
    }
  }

  private dispatch(msg: any): void {
    if (msg.id !== undefined && msg.id !== null && !msg.method) {
      const waiter = this.pending.get(msg.id);
      if (!waiter) {
        return;
      }
      this.pending.delete(msg.id);
      if (msg.error) {
        waiter.reject(new Error(String(msg.error.message ?? 'JSON-RPC 错误')));
      } else {
        waiter.resolve(msg.result);
      }
      return;
    }
    if (typeof msg.method === 'string') {
      for (const handler of this.handlers.get(msg.method) ?? []) {
        handler(msg.params);
      }
    }
  }
}
