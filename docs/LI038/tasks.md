# LI038 任务清单

> 依赖顺序执行；每项完成后勾选并记录实测值。

- [x] T1 后端事件：`jm_main.c` 暂停事件只发一次；`v` / `bt` 增补 `变量` / `栈` NDJSON（人读输出不变）。
- [x] T2 Rust 适配器：新增 `compiler/src/dap.rs`，`main.rs` 接入 `micc dap`；复用 LSP 的 Content-Length 读写。
- [x] T3 适配器能力：initialize/initialized、setBreakpoints（条件/上限）、launch（`.jsh` 编译）、configurationDone、threads、stackTrace、scopes、variables、continue/next/stepIn/stepOut、disconnect/terminate。
- [x] T4 适配器事件：暂停→stopped、结束→exited/terminated、stdout→output、栈/变量请求响应同步。
- [x] T5 扩展：`package.json` 增加 `debuggers` / `breakpoints`，删除 `li.debugPrompt`；新增 `src/dap.ts` 工厂与配置提供者。
- [x] T6 扩展接线：`runCurrent` 有断点时 `startDebugging`；旧快捷键转发到 DAP；删除旧自定义调试链路。
- [x] T7 测试：新增 `tools/test-dap.js`；更新 VS Code 集成测试（DAP 会话、单步）；`build.ps1` 接入。
- [x] T8 文档：路径图 LI038 ✅、README、编辑器 README/CHANGELOG。
- [x] T9 全量验证：cargo test、test-lsp、test-dap、verify.js、Run-It、build.ps1、固件体积（期望 +0 B）。

## 验收记录

| 项 | 期望 | 实测 |
|----|------|------|
| cargo test | 全过 | 4/4 |
| test-dap.js | 全过 | 21/21 |
| test-lsp.js | 9/9 | 9/9 |
| verify.js | 44/44 | 44/44 |
| 扩展集成 | 全过（含 DAP 单步） | 14/14 |
| build.ps1 | 全绿 | 全绿（test 62/62、errors 104、debug 54/54、arduino 14/14 等） |
| 固件 `类` | 55 204 B / 17 412 B | 55 204 B / 17 412 B（+0 B） |
