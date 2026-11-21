# LI038 规格：DAP 调试适配器

> 状态：待批准　目标：用 Debug Adapter Protocol 取代扩展里的自定义调试命令；后端仍复用 `jmrun --dbg --事件流`。
> 约束：零第三方依赖；不改 JMB / ABI / 固件，目标侧 0 B。

## 1. 目标与验收

- `micc dap` 提供 stdio DAP 服务（Content-Length 分帧，JSON 报文）。
- 支持断点、条件断点、`--task` 过滤、本地变量、调用栈、继续/下一行/单步/步过、断开/终止。
- VS Code 扩展注册 `li` 调试类型，F5 在存在断点时启动真实调试会话；调试工具栏、变量面板、调用栈面板可用。
- 旧快捷键保留：F5 继续、F9 断点、Shift+F9 条件断点、F10 下一行、Ctrl+F10 步过、Shift+F5 停止。
- 无断点时 F5 仍走原来的文本运行（保留运行期错误进问题面板的行为）。

验收：
1. `node tools/test-dap.js` 全过（初始化、断点、条件、暂停、栈、变量、单步、继续、停止）。
2. VS Code 扩展宿主集成测试全过，且断点命中后 `activeDebugSession.type === 'li'`，`li.debug.next` 能让暂停行前进。
3. `cargo test`、`test-lsp.js`、`verify.js` 无回退。
4. `tools/build.ps1` 全量回归绿；`类` 固件体积保持 **55 204 B / 17 412 B（+0 B）**。

## 2. 架构

```
VS Code 调试 UI
   │ DAP（stdio / Content-Length）
   ▼
micc dap  ──spawn──▶ jmrun --dbg --事件流
   │                    │ stderr: NDJSON 暂停/结束/栈/变量
   │                    │ stdout: 程序输出
   └── 翻译 ────────────┘
```

- 适配器不实现 VM，只做协议翻译与进程管理。
- `jmrun` 的 `--事件流` 增补两类结构化响应：`栈`、`变量`；现有 `暂停`/`结束` 事件保持不变。
- 人读文本继续输出到 stderr；适配器过滤 `(砾调试)` 提示符，其余转成 `console` 输出事件。

## 3. 后端改动（`runtime/port/host/jm_main.c`，仅宿主）

### 3.1 暂停事件只发一次

`dbg_where()` 当前在 REPL 每轮都调用 `stream_pause()`，会给编辑器重复 `暂停` 事件。改为：

- `dbg_where(int stream)`：只有 `stream != 0` 时才调用 `stream_pause()`。
- `dbg_repl()` 首次进入 `dbg_where(1)`，后续轮次 `dbg_where(0)`。
- 人读文本行不变。

### 3.2 变量事件

`dbg_vars()` 在保留人读输出的同时收集最多 64 项，`g_stream` 时追加一行：

```json
{"事件":"变量","任务":0,"局部":[{"名":"k","值":"3"}]}
```

- 读不到的值写 `"值":"<未赋值>"`；文本过长写 `"值":"<太长>"`。
- 没有局部符号时发空数组，保证适配器能结束等待。

### 3.3 调用栈事件

`dbg_stack()` 收集当前任务最多 16 帧，`g_stream` 时追加一行：

```json
{"事件":"栈","任务":0,"帧":[{"索引":0,"函数":"跑","文件":"examples/类.jsh","行":32}]}
```

- 无行表时 `文件` 为空、`行` 为 0。
- 两个事件都 `fflush(stderr)`。

## 4. 适配器（新增 `compiler/src/dap.rs`）

### 4.1 传输与入口

- `compiler/src/main.rs` 增加 `mod dap;`、用法行与 `if args[0] == "dap" { exit(dap::run()); }`。
- 复用 `lsp.rs` 的 Content-Length 读写：把 `read_message` / `send` 提为 `pub(crate)` 泛型函数（`BufRead` / `Write`），LSP 行为不变。
- `dap::run()`：阻塞读 stdin；每个请求同步处理；stdout 写入用互斥锁保证报文不交错；`seq` 用原子计数。

### 4.2 请求表

| 请求 | 行为 |
|------|------|
| `initialize` | 返回能力（DAP 的响应 body 就是能力对象本身，不再包一层 `capabilities`）：`supportsConfigurationDoneRequest`、`supportsConditionalBreakpoints`、`supportsHitConditionalBreakpoints`、`supportsTerminateRequest` = true；`exceptionBreakpointFilters: []`。 |
| `setBreakpoints` | 记录 `source.path` → `[{line, condition}]`；上限 8，超出项 `verified:false`。 |
| `launch` | 解析 `program`（`.jsh` 则用 micc 编译到临时 `.jmb`）、`source`、`cwd`、`jmrunPath`、`miccPath`、`task`、`breaks`、`cleanup`。缺 jmrun/micc 返回 `success:false`；成功后回 `{}` 并发 `initialized` 事件（VS Code 收到后才会下发断点与 `configurationDone`）。 |
| `configurationDone` | 先回响应，再 spawn `jmrun --dbg --事件流`，按断点表拼 `--break "文件:行 [if 条件]"` 与 `--task N`，`.jmb` 放最后。 |
| `threads` | 单个线程 `{id:1, name:"砾调试"}`。 |
| `stackTrace` | 暂停时向 jmrun 写 `bt`，等 `栈` 事件（2 s）；帧 id = 索引+1，`line` 至少 1，`source.path` 相对路径按 cwd 补全。 |
| `scopes` | 暂停时返回一个局部变量域 `variablesReference:1`；否则空。 |
| `variables` | `variablesReference==1` 且暂停时写 `v`，等 `变量` 事件；`value` 用后端文本。 |
| `continue` | 写 `c`，响应 `{allThreadsContinued:true}`。 |
| `next` / `stepIn` / `stepOut` | 分别写 `n` / `s` / `o`，响应 `{}`。 |
| `pause` | 后端没有异步暂停，返回 `success:false` 与说明。 |
| `disconnect` / `terminate` | 暂停时先写 `q`，随后终止子进程；响应 `{}`；确保只发一次 `terminated`。 |
| `setExceptionBreakpoints` | 响应 `{}`。 |
| 其它 | `success:false`，`message:"不支持：<命令>"`。 |

### 4.3 事件翻译

- stderr 行以 `{` 开头且可解析：
  - `事件=暂停` → `stopped`，`reason`：`断点`→`breakpoint`，`单步/行级/步过`→`step`；`description` 写明任务、函数、文件:行。
  - `事件=结束` → `exited`（`exitCode`=码）→ `terminated`。
  - `事件=栈/变量` → 放入等待队列并唤醒主线程。
  - 其它 → `output`（`category:"console"`）。
- 非 JSON 行 → `output`（`category:"console"`），含 `(砾调试)` 的行丢弃。
- stdout 由独立线程按行转 `output`（`category:"stdout"`）。
- 子进程 close 时若未发过 `terminated`，补 `exited`/`terminated`。

### 4.4 断点合并

- `launch.breaks`（扩展传入）始终保留。
- `setBreakpoints` 按文件替换客户端断点集合。
- 合并后按 `(文件, 行)` 去重，最多 8 条；条件写成 `if <条件>`。
- 运行中新增断点不即时生效（jmrun 只在暂停时读 stdin），在响应的 `message` 里说明“下次会话生效”。

## 5. VS Code 扩展

### 5.1 `package.json`

- 新增 `contributes.debuggers`：
  - `type: "li"`、`label: "砾"`、`languages: ["li"]`。
  - `configurationAttributes.launch`：`program`、`source`、`jmrunPath`、`miccPath`、`task`、`breaks`、`cleanup`。
  - `initialConfigurations` 一条最小配置。
- 新增 `contributes.breakpoints: [{ "language": "li" }]`。
- 删除不再使用的 `li.debugPrompt` 设置（暂停交互改由调试工具栏承担）。
- 命令与快捷键保持不变。

### 5.2 新增 `editors/vscode/src/dap.ts`

- `LiDebugAdapterFactory`：按配置里的 `miccPath`（或 `li.miccPath`）返回 `vscode.DebugAdapterExecutable(micc, ['dap'], { cwd })`。
- `LiDebugConfigurationProvider`：`resolveDebugConfiguration` 补齐当前 `.jsh`、`jmrunPath`、`miccPath`、`task`，不编译（编译仍由 `runCurrent` 负责，保留现有错误提示）。

### 5.3 `extension.ts`

- 注册调试配置提供者与适配器工厂（`vscode.debug.registerDebugConfigurationProvider` / `registerDebugAdapterDescriptorFactory`）。
- 删除旧的自定义调试进程链路：`debugChild`、`sendDebug`、`promptDebugAction`、`handleDebugLine`、`startDebugSession`、`setDebugPaused`。
- `runCurrent()`：编译成功后若有断点，调用 `vscode.debug.startDebugging`，launch 配置带 `program=临时 .jmb`、`source`、`breaks`（含条件）、`task`、`jmrunPath`、`miccPath`、`cleanup:true`；无断点仍走文本运行。
- `li.run`：活动 `li` 调试会话存在时转发 `workbench.action.debug.continue`，否则 `runCurrent()`。
- `li.debug.continue/next/step/stepOut`：活动会话存在时调用 `customRequest('continue'|'next'|'stepIn'|'stepOut')`。
- `li.debug.stop`：活动会话存在时 `vscode.debug.stopDebugging(session)`。
- `li.debug.breaks`：把当前断点与条件写到输出通道。

## 6. 测试

### 6.1 新增 `tools/test-dap.js`（Node，直接驱动 `micc dap`）

夹具写入系统临时目录，覆盖：

1. `initialize` 能力字段 + 收到 `initialized`。
2. `setBreakpoints` 条件断点返回 `verified:true`。
3. `launch`（传 `.jsh`，走适配器编译）+ `configurationDone`。
4. 条件成立时 `stopped` 行号正确、`reason` 为 `breakpoint`。
5. `threads` / `stackTrace` / `variables` 均有内容。
6. `next` 后再次 `stopped`，行号变化。
7. `continue` 后 `exited` + `terminated`。
8. 条件不成立时不停，直接结束。
9. 暂停中 `disconnect` 能结束会话。
10. 断点超过 8 个时至少一条 `verified:false`。

### 6.2 扩展集成测试 `editors/vscode/test/suite/index.js`

- 删除 `li.debugPrompt` 相关两段。
- 断点命中后断言 `vscode.debug.activeDebugSession.type === 'li'`。
- 执行 `li.debug.next` 后轮询 `session.customRequest('stackTrace', {threadId:1})`，断言暂停行前进。
- 保留 F9 清除、`li.debug.stop` 用例。

### 6.3 接入回归

- `tools/build.ps1`：在 `test-lsp.js` 之后运行 `node tools/test-dap.js`，无 node 时 SKIP。
- 保留 `test-debug.ps1` 全部 54 项。

## 7. 文档

- `docs/路径图.md`：LI038 标记 ✅，写明实测值；§10.7 的 H 段进度改为“LI037–LI038 已完成”。
- `README.md`、`editors/vscode/README.md`、`editors/vscode/CHANGELOG.md`：补 DAP 调试、变量/调用栈面板、`micc dap`。
- 新增 `docs/LI038/spec.md`（本文件）与 `docs/LI038/tasks.md`。

## 8. 验证命令

```powershell
cargo build --release --manifest-path compiler/Cargo.toml
cargo test --release --manifest-path compiler/Cargo.toml
node tools/test-lsp.js
node tools/test-dap.js
node editors/vscode/test/verify.js
cd editors/vscode; npm run compile
powershell -NoProfile -ExecutionPolicy Bypass -File test/run-it.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build-firmware.ps1 -Example 类
```

## 9. 已知限制

- 运行中新增/删除断点不即时生效，下次会话生效。
- 变量面板只列当前帧局部变量；全局变量需用 jmrun REPL 的 `g <名字>`，DAP 暂不暴露。
- `pause` 请求不支持（后端没有异步暂停入口）。
- 行号按 1 基处理（VS Code 默认）。
