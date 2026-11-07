# 砾（LI）VS Code 扩展

`editors/vscode` 是砾语言的**编辑器外壳**，只做五件事：语法高亮、编译/运行诊断、跳转到定义、运行与单断点、语言卡补全。
它不参与编译器、运行时和固件构建：扩展卸载或启动失败，`tools\*.ps1` 与 `micc` 的命令行工作流完全不受影响。

- 语言 ID：`li`，文件后缀 `.jsh`，TextMate scope：`source.li`
- `micc` 仍是唯一的编译入口，`jmrun` 仍是唯一的运行入口；扩展不会自己解析语法或字节码

---

## 1. 安装

### 1.1 本地打包 `.vsix`

在仓库根目录执行：

```powershell
cd editors\vscode
npm install
npm run compile
npx --yes @vscode/vsce package --out dist\li-vscode-0.1.0.vsix
```

`npx` 首次运行需要联网下载 `@vscode/vsce`；之后可用 `npm run package`。
产物在 `editors\vscode\dist\li-vscode-0.1.0.vsix`。

### 1.2 安装到 VS Code

```powershell
code --install-extension editors\vscode\dist\li-vscode-0.1.0.vsix
```

或在 VS Code 里「扩展 → `...` → 从 VSIX 安装」，选同一个文件。安装后重载窗口，打开任意 `.jsh` 即可生效。

> 仅本地使用，不需要发布到扩展市场；`publisher` 字段只是 vsce 的必填项。

---

## 2. 使用

### 2.1 语法高亮

| 类别 | 示例 |
|---|---|
| 关键字 | `令 如果 否则如果 否则 当 循环 次 遍历 在 中断 继续 函数 返回 导入` |
| 逻辑字面量 / 词运算符 | `真 假 空` / `与 或 非` |
| 内建函数 | `打印 文本 整数 小数 长度 范围 类型 总和 最大 最小 绝对值` |
| 原生函数 | `引脚写 引脚读 引脚翻转 串口写 等待毫秒 ADC初始化 ADC读 定时器初始化 定时器计数 定时器延时` |
| 数字 | 整数 `4095`、小数 `4095.0`；`1e3` 这类指数写法标红（对应 E102） |
| 文本 | `"…"`、`'…'`、`“…”`、`‘…’`，转义 `\n \t \\ \" \'` |
| 注释 | `# 到行尾` |
| 原生模块导入 | `导入 引脚` 中的 `导入` 与模块名分别着色 |
| 未支持的关键字 | `类 继承 新建 尝试 捕获 最终 抛出 匹配 情形 枚举 用 断言 超` 标红（语言卡里明确不支持） |

### 2.2 编译诊断

打开或保存 `.jsh` 时自动调用 `micc`，把 stderr 里的 `行:列 错误[码] 说明` 映射成编辑器波浪线，提示文字带错误码（如 `E102：不支持指数形式的数字字面量（如 1e3）`）。

- 输入时也会刷新（防抖 700 ms），可用设置 `li.diagnosticsOnType` 关闭
- 有未保存改动时，先把缓冲区写到系统临时目录再编译，诊断跟着光标走
- 临时文件写在 `%TEMP%`，不落进仓库；编译通过后诊断立即清除
- 找不到 `micc` 时只在文件首行给一条黄色提示，不弹窗、不阻塞编辑

诊断依赖 `micc` 已经构建好：

```powershell
powershell -ExecutionPolicy Bypass -File tools\build.ps1 -SkipTests
```

或用设置 `li.miccPath` 指向任意位置的 `micc.exe`。

### 2.3 跳转到定义

在 `.jsh` 里对标识符按 `F12` 或 `Ctrl+点击`：

| 光标位置 | 行为 |
|---|---|
| 本文件的 `令` / `函数` / `类` / `方法` / `字段` | 跳到定义行（`micc --symbols <文件>`） |
| `接收者.名字`，且 `接收者` 是包名 | 跳到依赖包入口（`micc --pack-files <包目录>`，内部复用 `pack_resolve`） |
| 裸包名（`导入 工具` 中的 `工具`） | 跳到该包入口第 1 行 |
| `导入 引脚` 中的原生模块 | 跳到 `spec/langcard.json` 里该模块的声明行 |
| 内建函数（如 `打印`）或不存在的名字 | 不跳转，保持原样 |

- 跨文件/跨包不另写路径规则：包与入口一律来自 `micc --pack-files`，与 `micc pack` 使用同一套 `砾包.toml` 解析
- 定义表按文件 `mtime` 缓存，保存后自动失效
- 找不到 `micc` 时静默不跳转，不影响编辑

### 2.4 运行与单断点

| 操作 | 实际调用 |
|---|---|
| `F5` / 「砾：运行当前文件」 | `micc <当前文件> -o <临时.jmb>` → `jmrun <临时.jmb>` |
| `F9` / 「砾：行断点开/关」 | 在当前行设置或取消**单断点**（每个文档最多一个，整行淡红底 + 左侧红线） |

- 运行前自动保存当前文件；`micc` 失败时直接走编译诊断，不会调 `jmrun`
- 输出流式进「输出」面板的 **砾** 频道；运行期错误同时进**问题**面板
- 有断点时，`F5` 会附加 `--break <文件>:<行>`；命中后自动打开该文件并定位到停下的行，输出里也会打印 `断点：文件:行`
- 没有调试段（`--strip-debug`、旧文件、损坏段）时，错误只按错误码定位到当前文件首行，不弹错
- 这是**最小验证**：不做变量查看、多断点、继续/单步、调用栈

### 2.5 命令

| 命令面板标题 | 命令 ID | 默认快捷键 | 实际调用 |
|---|---|---|---|
| 砾：构建（micc + jmrun + 回归） | `li.build` | `Ctrl+Shift+B`（仅 `.jsh`） | `tools\build.ps1` |
| 砾：构建固件（STM32） | `li.buildFirmware` | `Ctrl+Alt+B`（仅 `.jsh`） | `tools\build-firmware.ps1 -Example <示例名>` |
| 砾：烧录固件（需确认） | `li.flash` | — | `tools\flash-firmware.ps1 -Apply` |
| 砾：诊断当前文件 | `li.check` | — | `micc <当前文件> -o <临时.jmb>` |
| 砾：运行当前文件 | `li.run` | `F5`（仅 `.jsh`） | `micc` → `jmrun`（有断点时带 `--break`） |
| 砾：行断点开/关（单断点） | `li.toggleBreakpoint` | `F9`（仅 `.jsh`） | 编辑器内状态，不调外部命令 |
| 砾：重新加载语言卡补全 | `li.reloadLangcard` | — | 重新读取 `spec/langcard.json` |
| 砾：取消当前任务 | `li.cancel` | 点状态栏 | 结束当前 PowerShell 子进程 |

- 输出进「输出」面板的 **砾** 频道，实时流式追加
- 编辑器标题栏有「构建」「烧录」两个按钮（仅 `.jsh`）
- 构建固件时，如果当前文件在 `examples\` 下，自动用它的相对路径作 `-Example`（如 `board\ADC采样`），否则用设置 `li.firmwareExample`（默认 `你好`）
- 烧录必须先确认；不会自动执行
- 同一时刻只允许一个任务，运行中状态栏显示进度，点击即取消

### 2.6 补全（语言卡驱动）

补全内容直接读 `spec/langcard.json`，不重复维护词表：

- 关键字：`keywords` 中 `supported: true` 的条目
- 内建函数：`builtins` 的名字与文档
- 原生模块：`导入 ` 后提示模块名（`引脚` / `ADC` / `定时器`）
- 原生函数：只在该文件已 `导入` 对应模块后出现

因为砾的标识符是中文，VS Code 的自动弹出不一定会在中文输入时触发，**手动 `Ctrl+Space` 最稳**。
语言卡改动后执行「砾：重新加载语言卡补全」，或直接保存文件（按 mtime 自动失效缓存）。

---

## 3. 设置

| 设置项 | 默认值 | 说明 |
|---|---|---|
| `li.miccPath` | `compiler/target/release/micc.exe` | 相对工作区根目录；找不到时回退 release/debug 默认路径 |
| `li.jmrunPath` | `build/jmrun.exe` | `F5` 使用的 `jmrun`；找不到时回退 `build/jmrun` |
| `li.powershell` | `powershell.exe` | 执行 `tools\*.ps1` 的 PowerShell |
| `li.firmwareExample` | `你好` | `build-firmware.ps1 -Example` 的默认值 |
| `li.diagnosticsOnType` | `true` | 输入时是否刷新诊断 |
| `li.langcardPath` | `spec/langcard.json` | 补全数据源，也是 `导入 模块` 跳转的目标 |

---

## 4. 验证

两套测试都不需要板子，也不需要扩展坞：

```powershell
cd editors\vscode
npm install          # 首次
npm run compile

# 1) 纯逻辑 + 真实 CLI：符号表、包解析、运行输出、断点判定
node test\verify.js

# 2) VS Code 扩展宿主：真实命令、跳转、诊断、F5/F9
powershell -NoProfile -ExecutionPolicy Bypass -File test\run-it.ps1
```

- `verify.js` 覆盖 19 项：同文件符号/类/方法、跨包 `工具.加一`、裸包名、不存在符号、原生模块语言卡、E007 定位、无调试段退回、断点命中（第 32 行）与未命中（退出码 2）。
- `run-it.ps1` 启动真实 `Code.exe --extensionTestsPath`，覆盖 11 项：扩展激活、命令注册、`c.加`/`c.值`/`查点` 三处同文件跳转、`打印` 不跳转、跨包跳转、`导入 引脚` 跳语言卡、F5 错误进问题面板并定位第 3 行、F9 + F5 断点停第 32 行、再按 F9 清除。
- 集成测试夹具在 `build/editor-it/`（已 gitignore），由 `run-it.ps1` 自动重建。

---

## 5. 边界

- 扩展不修改 `compiler/`、`runtime/`、`firmware/`、`tools/` 的任何文件，只**调用**现有脚本
- 跳转只覆盖 `令/函数/类/方法/字段/包/原生模块`；不做重命名、引用查找、格式化
- 运行是「编译 + 主机跑一遍」，不是调试器：只有单断点，没有变量、调用栈、继续/单步
- 诊断是「编译一次、报告首条错误」，与 `micc` 命令行行为一致
- 不上板、不碰串口；烧录只转发给 `tools\flash-firmware.ps1`
- 扩展自身构建产物（`node_modules/`、`out/`、`dist/`）已在根 `.gitignore` 中排除
