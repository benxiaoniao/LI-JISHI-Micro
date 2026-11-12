# LI012 单继承 · 任务清单

> 状态：已批准，2026-11-11 实施并实测完成。

1. - [x] **词法 / AST / 解析**
   - `lexer.rs`：`Kw::Inherit`，`继承` 转正。
   - `ast.rs`：`ClassDecl.parent: Option<String>`。
   - `parser.rs`：类名后可选 `继承 IDENT`。

2. - [x] **codegen 继承主体**
   - `ClassObj.parent`；父类解析（未定义/自继承/成环/父初始化带参 → E140）。
   - 按深度扁平化字段并做跨类成员冲突检查（E140 / E105）。
   - `compile_class_init` 先跑父构造、字段槽位加前缀偏移。
   - `class_method_idx` 沿父链查找；方法名先统一登记，支持前后声明互调。

3. - [x] **JMB v5 写入与加载**
   - `writer.rs`：版本 5、类记录 10 B、`parent_idx`。
   - `jm_loader.c`：版本 5、10 B 步长、父下标与防环校验。
   - `jm_vm.c`：类记录步长、方法链查找、`CALL_INIT` 祖先校验。
   - `jm_builtin.c`：实例 repr 步长。

4. - [x] **错误码**
   - `error.rs` 新增 E140，修订 E103 文案；`langcard.rs` 扫描白名单补 E140。

5. - [x] **示例与回归**
   - 新增 `examples/继承.jsh` + 实测生成的 `tests/expected/继承.txt`。
   - `tools/test-errors.ps1` 增加 E140 用例（父类未定义、自继承、成环、重定义父类字段、父构造带参）。

6. - [x] **打包与预算**
   - `pack_merge.rs`：父类名随包改名；`class_field_counts` 累加父链字段。

7. - [x] **全量实测**
   - `tools/build.ps1 -SkipTests`。
   - `test` / `test-errors` / `test-debug` / `test-packages` / `test-native-import` 全部 0 失败。
   - `micc --check-langcard`。
   - `tools/build-firmware.ps1 -Example 类`：53 900 / 17 404。

8. - [x] **文档同步**
   - `micc --langcard` 重新生成 `docs/语言卡.md`、`spec/langcard.json`。
   - `docs/微字节码规格.md`、`docs/路径图.md`、`README.md` 按实测数字更新。

9. - [x] **收尾**
   - 汇总实测值与剩余预算；全部改动未提交。
