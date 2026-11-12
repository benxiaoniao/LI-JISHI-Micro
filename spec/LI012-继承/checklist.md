# LI012 单继承 · 验收清单

- [x] `tools/build.ps1 -SkipTests` 重建成功，全部示例（含 `继承.jsh`）编译通过
- [x] `test.ps1`：48/48，新增 `继承` 基准逐行一致
- [x] `test-errors.ps1`：80/80，新增 5 条 E140 + 1 条 E103
- [x] `test-debug.ps1`：54/54
- [x] `test-packages.ps1`：54/54
- [x] `test-native-import.ps1`：37/37
- [x] `micc --check-langcard`：通过，22 个示例可编译
- [x] 固件 `类` 示例：53 900 B Flash（82.24%）/ 17 404 B RAM；Flash 余量 11 636 B，相对 LI011 基线 +216 B
- [x] `docs/语言卡.md`、`spec/langcard.json` 由 `micc --langcard` 重新生成
- [x] `docs/微字节码规格.md`（JMB v5 / 类记录 10 B / parent_idx）、`docs/路径图.md`、`README.md` 同步
- [x] 所有改动保持未提交
