# 包（H4 最小可用切分）回归：源码包 + 单 .jmb。
# 所有夹具都建在 $env:TEMP 下的临时目录，跑完删除，不污染仓库。
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
$micc = "compiler\target\release\micc.exe"
$jmrun = "build\jmrun.exe"

Push-Location $root
try {
    if (-not (Test-Path $micc)) { throw "先运行 tools\build.ps1" }
    if (-not (Test-Path $jmrun)) { throw "先运行 tools\build.ps1（缺 build\jmrun.exe）" }

    $sandbox = Join-Path $env:TEMP ("li-pack-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Path $sandbox | Out-Null
    $script:fail = 0
    $script:total = 0
    $script:skip = 0

    # 写一个包：砾包.toml + 主.jsh。故意带 UTF-8 BOM，顺带覆盖 BOM 处理。
    function New-Pkg([string]$dir, [string]$toml, [string]$src) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $enc = New-Object System.Text.UTF8Encoding($true)
        [IO.File]::WriteAllText((Join-Path $dir "砾包.toml"), $toml, $enc)
        [IO.File]::WriteAllText((Join-Path $dir "主.jsh"), $src, $enc)
    }

    function Invoke-Pack([string]$dir, [switch]$Offline) {
        if ($Offline) {
            $o = (& $micc pack --目录 $dir --离线 2>&1 | Out-String)
        }
        else {
            $o = (& $micc pack --目录 $dir 2>&1 | Out-String)
        }
        return @{ code = $LASTEXITCODE; out = $o }
    }

    function Check([string]$name, [bool]$ok, [string]$detail) {
        $script:total++
        if ($ok) {
            Write-Host ("[通过] " + $name)
        }
        else {
            Write-Host ("[不通过] " + $name)
            if ($detail) { Write-Host $detail }
            $script:fail++
        }
    }

    function Check-Error([string]$name, $r, [string]$code, [string]$needle) {
        $ok = ($r.code -ne 0) -and ($r.out -match [regex]::Escape($code)) -and $r.out.Contains($needle)
        Check $name $ok ("期望 " + $code + " 且包含「" + $needle + "」，实际退出码 " + $r.code + "：`n" + $r.out)
    }

    function Run-Jmb([string]$jmb) {
        if (-not (Test-Path $jmb)) { return "<缺少 jmb>" }
        $o = (& $jmrun $jmb 2>&1 | Out-String).TrimEnd()
        return ($o -replace "`r`n", "`n")
    }

    # ---------- 1. 单包 ----------
    $d = Join-Path $sandbox "单包"
    $toml = @"
[包]
名字 = "单包"
版本 = "1.0.0"
"@
    $src = @"
打印("单包", 1 + 2)
"@
    New-Pkg $d $toml $src
    $r = Invoke-Pack $d
    $jmb = Join-Path $d "单包.jmb"
    $out = Run-Jmb $jmb
    Check "单包：打包并运行" (($r.code -eq 0) -and ($out -eq "单包 3") -and (Test-Path $jmb)) ($r.out + "`n实际输出：" + $out)

    # ---------- 2. 双包依赖 ----------
    $root2 = Join-Path $sandbox "双包"
    $tool = Join-Path $root2 "工具"
    $app = Join-Path $root2 "应用"
    $toml = @"
[包]
名字 = "工具"
版本 = "0.1.0"

[导出]
名字 = ["加一", "问候"]
"@
    $src = @"
函数 加一(数)：
    返回 数 + 1

函数 问候(名)：
    返回 "你好，" + 名

令 内部 = 99
"@
    New-Pkg $tool $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^0.1.0" }
"@
    $src = @"
令 结果 = 工具.加一(41)
打印(工具.问候("砾"))
打印(结果)
"@
    New-Pkg $app $toml $src
    $r = Invoke-Pack $app
    $out = Run-Jmb (Join-Path $app "应用.jmb")
    Check "双包依赖：源码级合并并运行" (($r.code -eq 0) -and ($out -eq "你好，砾`n42") -and $r.out.Contains("共 1 个")) ($r.out + "`n实际输出：" + $out)

    # ---------- 3. 同名字共存（不同包同名，不串） ----------
    $root3 = Join-Path $sandbox "同名"
    $toml = @"
[包]
名字 = "甲"
版本 = "1.0.0"

[导出]
名字 = ["名字"]
"@
    $src = @"
函数 名字()：
    返回 "甲"
"@
    New-Pkg (Join-Path $root3 "甲") $toml $src
    $toml = @"
[包]
名字 = "乙"
版本 = "1.0.0"

[导出]
名字 = ["名字"]
"@
    $src = @"
函数 名字()：
    返回 "乙"
"@
    New-Pkg (Join-Path $root3 "乙") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
甲 = { 路径 = "../甲", 版本 = "*" }
乙 = { 路径 = "../乙", 版本 = "*" }
"@
    $src = @"
打印(甲.名字(), 乙.名字())
"@
    New-Pkg (Join-Path $root3 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root3 "应用")
    $out = Run-Jmb (Join-Path $root3 "应用\应用.jmb")
    Check "同名函数在不同包内共存" (($r.code -eq 0) -and ($out -eq "甲 乙")) ($r.out + "`n实际输出：" + $out)

    # ---------- 3b. 跨包类型导出（LI016）：类/枚举显式写 [导出] 类型 ----------
    $rootTypes = Join-Path $sandbox "类型导出"
    $typeLib = Join-Path $rootTypes "类型库"
    $toml = @"
[包]
名字 = "类型库"
版本 = "1.0.0"

[导出]
名字 = ["描述"]
类型 = ["计数器", "状态"]
"@
    $libSrc = @"
类 计数器:
    令 值 = 0
    函数 初始化(起点):
        本.值 = 起点
    函数 加(n):
        本.值 = 本.值 + n
        返回 本.值
    函数 读():
        返回 本.值

枚举 状态:
    空闲
    运行

函数 描述():
    返回 "库"
"@
    New-Pkg $typeLib $toml $libSrc
    $typeApp = Join-Path $rootTypes "应用"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
类型库 = { 路径 = "../类型库", 版本 = "^1.0.0" }
"@
    $src = @"
函数 使用(c: 类型库.计数器) -> 整数:
    返回 c.读()

令 c = 新建 类型库.计数器(10)
打印("新建:", 使用(c))
打印("加:", c.加(5))

令 s = 类型库.状态.运行
打印("枚举:", s)

匹配 s:
    情形 类型库.状态.空闲:
        打印("匹配:空闲")
    情形 类型库.状态.运行:
        打印("匹配:运行")
    情形 其他:
        打印("匹配:其他")
"@
    New-Pkg $typeApp $toml $src
    $o = (& $micc pack --目录 $typeApp --strict-type 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $out = Run-Jmb (Join-Path $typeApp "应用.jmb")
    $expect = "新建: 10`n加: 15`n枚举: 1`n匹配:运行"
    Check "跨包类型：新建类 + 枚举 + 跨包类型标注（--strict-type）" (($code -eq 0) -and ($out -eq $expect)) ($o + "`n实际输出：" + $out)

    # LI017：默认公开 —— 没写 [导出] 也能跨包用类/枚举/函数
    $pubLib = Join-Path $rootTypes "公开库"
    $toml = @"
[包]
名字 = "公开库"
版本 = "1.0.0"
"@
    New-Pkg $pubLib $toml $libSrc
    $pubApp = Join-Path $rootTypes "公开应用"
    $toml = @"
[包]
名字 = "公开应用"
版本 = "1.0.0"

[依赖]
公开库 = { 路径 = "../公开库", 版本 = "^1.0.0" }
"@
    $src = @"
令 c = 新建 公开库.计数器(3)
打印("默认公开:", c.读(), 公开库.状态.运行, 公开库.描述())
"@
    New-Pkg $pubApp $toml $src
    $r = Invoke-Pack $pubApp
    $out = Run-Jmb (Join-Path $pubApp "公开应用.jmb")
    Check "默认公开：没写 [导出] 的类/枚举/函数可跨包" (($r.code -eq 0) -and ($out -eq "默认公开: 3 1 库")) ($r.out + "`n实际输出：" + $out)

    # LI017：私有类/枚举/函数/令 一律不能跨包
    $privLib = Join-Path $rootTypes "私有库"
    $toml = @"
[包]
名字 = "私有库"
版本 = "1.0.0"
"@
    $privSrc = @"
类 公开类:
    令 值 = 0

私有 类 隐藏类:
    令 值 = 0

私有 枚举 隐藏状态:
    甲
    乙

私有 函数 内部():
    返回 1

私有 令 秘密 = 42
"@
    New-Pkg $privLib $toml $privSrc
    $privCases = @(
        @{ name = "私有类"; src = "令 c = 新建 私有库.隐藏类()" },
        @{ name = "私有枚举"; src = "打印(私有库.隐藏状态.甲)" },
        @{ name = "私有函数"; src = "打印(私有库.内部())" },
        @{ name = "私有令"; src = "打印(私有库.秘密)" }
    )
    foreach ($case in $privCases) {
        $d = Join-Path $rootTypes ("私有应用-" + $case.name)
        $toml = @"
[包]
名字 = "私有应用"
版本 = "1.0.0"

[依赖]
私有库 = { 路径 = "../私有库", 版本 = "^1.0.0" }
"@
        New-Pkg $d $toml $case.src
        $r = Invoke-Pack $d
        Check-Error ($case.name + "不能跨包：E128") $r "E128" "私有声明"
    }

    # 私有声明写进 [导出]：E144
    $conflict = Join-Path $rootTypes "冲突库"
    $toml = @"
[包]
名字 = "冲突库"
版本 = "1.0.0"

[导出]
名字 = ["内部"]
类型 = ["隐藏类"]
"@
    New-Pkg $conflict $toml $privSrc
    $conflictApp = Join-Path $rootTypes "冲突应用"
    $toml = @"
[包]
名字 = "冲突应用"
版本 = "1.0.0"

[依赖]
冲突库 = { 路径 = "../冲突库", 版本 = "^1.0.0" }
"@
    New-Pkg $conflictApp $toml '打印("x")'
    $r = Invoke-Pack $conflictApp
    Check-Error "私有声明写进 [导出]：E144" $r "E144" "私有"

    # 接口清单：默认公开的类型/函数入清单，私有声明不入
    $o = (& $micc interface --目录 $pubLib 2>&1 | Out-String)
    $pubIf = Join-Path $pubLib "接口.toml"
    $pubText = if (Test-Path $pubIf) { [IO.File]::ReadAllText($pubIf) } else { "" }
    Check "interface：默认公开的类/枚举/函数都列出" (($LASTEXITCODE -eq 0) -and $pubText.Contains("[类.计数器]") -and $pubText.Contains("[枚举.状态]") -and $pubText.Contains("描述 = 0")) ($o + "`n" + $pubText)

    $o = (& $micc interface --目录 $privLib 2>&1 | Out-String)
    $privIf = Join-Path $privLib "接口.toml"
    $privText = if (Test-Path $privIf) { [IO.File]::ReadAllText($privIf) } else { "" }
    $privOk = ($LASTEXITCODE -eq 0) -and $privText.Contains("[类.公开类]") -and
              (-not $privText.Contains("隐藏类")) -and (-not $privText.Contains("隐藏状态")) -and
              (-not $privText.Contains("内部")) -and (-not $privText.Contains("秘密"))
    Check "interface：私有类/枚举/函数不入清单" $privOk ($o + "`n" + $privText)

    # LI018：标准库模块（导入 文本）不需要写进 [包] 原生
    $stdLib = Join-Path $rootTypes "标准库包"
    $toml = @"
[包]
名字 = "标准库包"
版本 = "1.0.0"
"@
    New-Pkg $stdLib $toml @"
导入 文本

函数 首字(s):
    返回 子串(s, 0, 1)
"@
    $stdApp = Join-Path $rootTypes "标准库应用"
    $toml = @"
[包]
名字 = "标准库应用"
版本 = "1.0.0"

[依赖]
标准库包 = { 路径 = "../标准库包", 版本 = "^1.0.0" }
"@
    New-Pkg $stdApp $toml '打印(标准库包.首字("砾石"))'
    $r = Invoke-Pack $stdApp
    $out = Run-Jmb (Join-Path $stdApp "标准库应用.jmb")
    Check "标准库模块：导入 文本 不需要 [包] 原生" (($r.code -eq 0) -and ($out -eq "砾")) ($r.out + "`n实际输出：" + $out)

    # 类名不能当值用
    $valApp = Join-Path $rootTypes "值引用应用"
    $toml = @"
[包]
名字 = "值引用应用"
版本 = "1.0.0"

[依赖]
类型库 = { 路径 = "../类型库", 版本 = "^1.0.0" }
"@
    New-Pkg $valApp $toml "令 c = 类型库.计数器"
    $r = Invoke-Pack $valApp
    Check-Error "类名当值用：E128 提示改用 新建" $r "E128" "不能当值用"

    # 接口清单：类 / 字段 / 方法 / 枚举；归档 verify 对照类表
    $o = (& $micc interface --目录 $typeLib 2>&1 | Out-String)
    $ifPath = Join-Path $typeLib "接口.toml"
    $ifText = if (Test-Path $ifPath) { [IO.File]::ReadAllText($ifPath) } else { "" }
    $ifOk = ($LASTEXITCODE -eq 0) -and $ifText.Contains("[类.计数器]") -and
            $ifText.Contains('字段 = ["值"]') -and
            $ifText.Contains('方法 = ["初始化/1", "加/1", "读/0"]') -and
            $ifText.Contains("[枚举.状态]") -and $ifText.Contains('成员 = ["空闲", "运行"]')
    Check "interface：表达类/字段/方法与枚举" $ifOk ($o + "`n" + $ifText)

    & $micc pack --目录 $typeLib --归档 | Out-Null
    $typeZip = Join-Path $typeLib "类型库-1.0.0.zip"
    $v = (& $micc verify $typeZip 2>&1 | Out-String)
    Check "verify 归档：接口里的类/方法对照类表通过" (($LASTEXITCODE -eq 0) -and $v.Contains("校验通过")) $v

    $badIface = $ifText -replace "加/1", "加/2"
    [IO.File]::WriteAllText($ifPath, $badIface, (New-Object System.Text.UTF8Encoding($false)))
    $v = (& $micc verify $typeLib 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "接口方法参数个数不符：E137" @{ code = $code; out = $v } "E137" "参数个数不符"

    & $micc interface --目录 $typeLib | Out-Null
    $v = (& $micc verify $typeLib 2>&1 | Out-String)
    Check "重新生成接口后类校验恢复通过" (($LASTEXITCODE -eq 0) -and $v.Contains("校验通过")) $v

    # ---------- 4. 菱形依赖（基础只装一份） ----------
    $root4 = Join-Path $sandbox "菱形"
    $toml = @"
[包]
名字 = "基础"
版本 = "1.2.3"

[导出]
名字 = ["值"]
"@
    $src = @"
函数 值()：
    返回 7
"@
    New-Pkg (Join-Path $root4 "基础") $toml $src
    foreach ($side in @("左", "右")) {
        $toml = @"
[包]
名字 = "$side"
版本 = "1.0.0"

[依赖]
基础 = { 路径 = "../基础", 版本 = "^1.2.0" }

[导出]
名字 = ["取"]
"@
        $src = @"
函数 取()：
    返回 基础.值()
"@
        New-Pkg (Join-Path $root4 $side) $toml $src
    }
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
左 = { 路径 = "../左", 版本 = "^1.0.0" }
右 = { 路径 = "../右", 版本 = "^1.0.0" }
"@
    $src = @"
打印(左.取(), 右.取())
"@
    New-Pkg (Join-Path $root4 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root4 "应用")
    $out = Run-Jmb (Join-Path $root4 "应用\应用.jmb")
    Check "菱形依赖：基础只解析一份（共 3 个）" (($r.code -eq 0) -and $r.out.Contains("共 3 个") -and ($out -eq "7 7")) ($r.out + "`n实际输出：" + $out)

    # ---------- 5. 名字表接近上限 ----------
    $root5 = Join-Path $sandbox "名字表接近"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
"@
    $sb = New-Object System.Text.StringBuilder
    for ($i = 1; $i -le 240; $i++) {
        [void]$sb.AppendLine("令 g" + $i.ToString("000") + " = 1")
    }
    New-Pkg (Join-Path $root5 "工具") $toml $sb.ToString()
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    $src = @"
打印("接近上限")
"@
    New-Pkg (Join-Path $root5 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root5 "应用")
    $m = [regex]::Match($r.out, "名字表：(\d+) / 255")
    $n = if ($m.Success) { [int]$m.Groups[1].Value } else { 0 }
    Check ("名字表接近上限：" + $n + " / 255 仍可打包") (($r.code -eq 0) -and ($n -ge 200) -and ($n -le 255)) $r.out

    # ---------- 6. 名字表超过 255（E124 并指出占用最多的包） ----------
    $root6 = Join-Path $sandbox "名字表超限"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
"@
    $sb = New-Object System.Text.StringBuilder
    for ($i = 1; $i -le 260; $i++) {
        [void]$sb.AppendLine("令 g" + $i.ToString("000") + " = 1")
    }
    New-Pkg (Join-Path $root6 "工具") $toml $sb.ToString()
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    New-Pkg (Join-Path $root6 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root6 "应用")
    Check-Error "名字表超限：E124 + 占用最多的包" $r "E124" "占用最多的是：工具"

    # ---------- 7. native ABI 不一致（拒绝） ----------
    $root7 = Join-Path $sandbox "ABI"
    $tomlBad = @"
[包]
名字 = "工具"
版本 = "1.0.0"
原生ABI = 3
"@
    $src = @"
令 值 = 1
"@
    New-Pkg (Join-Path $root7 "工具") $tomlBad $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    New-Pkg (Join-Path $root7 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root7 "应用")
    Check-Error "native ABI 不一致：E129 拒绝" $r "E129" "ABI"

    # ---------- 8. native 需求并集（依赖声明的模块对根包生效） ----------
    $root8 = Join-Path $sandbox "原生并集"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
原生 = ["引脚"]
"@
    $src = @"
导入 引脚

函数 亮()：
    引脚写(13, 0)
"@
    New-Pkg (Join-Path $root8 "工具") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    $src = @"
打印("native 并集")
"@
    New-Pkg (Join-Path $root8 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root8 "应用")
    Check "native 并集：依赖声明的模块被合并" (($r.code -eq 0) -and $r.out.Contains("原生并集：引脚")) $r.out

    # ---------- 9. 依赖包顶层出现可执行语句（E127） ----------
    $root9 = Join-Path $sandbox "顶层语句"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
"@
    $src = @"
打印("我是可执行语句")
"@
    New-Pkg (Join-Path $root9 "工具") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    $src = @"
打印("根包")
"@
    New-Pkg (Join-Path $root9 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root9 "应用")
    Check-Error "依赖包顶层可执行语句：E127" $r "E127" "依赖包"

    # ---------- 10. 引用未导出名字（E128） ----------
    $root10 = Join-Path $sandbox "未导出"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"

[导出]
名字 = ["加一"]
"@
    $src = @"
函数 加一(数)：
    返回 数 + 1

令 内部 = 99
"@
    New-Pkg (Join-Path $root10 "工具") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    $src = @"
打印(工具.内部)
"@
    New-Pkg (Join-Path $root10 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root10 "应用")
    Check-Error "引用未导出名字：E128" $r "E128" "没有导出"

    # ---------- 11. 堆预算超限（E130） ----------
    $root11 = Join-Path $sandbox "堆预算"
    $zeros = (0..254 | ForEach-Object { "0" }) -join ", "
    $sb = New-Object System.Text.StringBuilder
    foreach ($v in @("a", "b", "c", "d", "e")) {
        [void]$sb.AppendLine("令 $v = [$zeros]")
    }
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"
"@
    New-Pkg $root11 $toml $sb.ToString()
    $r = Invoke-Pack $root11
    Check-Error "堆预算超限：E130" $r "E130" "堆下界"

    # ---------- 12. .jmb 体积超限（E130） ----------
    $root12 = Join-Path $sandbox "字节码上限"
    $sb = New-Object System.Text.StringBuilder
    for ($i = 0; $i -lt 60; $i++) {
        [void]$sb.AppendLine('打印("abcdefghij")')
    }
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"
字节码上限 = 100
"@
    New-Pkg $root12 $toml $sb.ToString()
    $r = Invoke-Pack $root12
    Check-Error "字节码上限超限：E130" $r "E130" "字节码上限"

    # ---------- 13. 离线缺包（没装过，E126） ----------
    $root13 = Join-Path $sandbox "离线缺包"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"
"@
    $src = @"
令 值 = 1
"@
    New-Pkg (Join-Path $root13 "工具") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 路径 = "../工具", 版本 = "^1.0.0" }
"@
    New-Pkg (Join-Path $root13 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root13 "应用") -Offline
    Check-Error "离线缺包：E126" $r "E126" "锁文件"

    # ---------- 14. 版本冲突：单版本约束 + 来源链 ----------
    $root14 = Join-Path $sandbox "版本冲突"
    $toml = @"
[包]
名字 = "基础"
版本 = "0.1.0"
"@
    $src = @"
函数 值()：
    返回 1
"@
    New-Pkg (Join-Path $root14 "基础") $toml $src
    $toml = @"
[包]
名字 = "左"
版本 = "1.0.0"

[依赖]
基础 = { 路径 = "../基础", 版本 = "^0.1.0" }
"@
    New-Pkg (Join-Path $root14 "左") $toml $src
    $toml = @"
[包]
名字 = "右"
版本 = "1.0.0"

[依赖]
基础 = { 路径 = "../基础", 版本 = "^0.2.0" }
"@
    New-Pkg (Join-Path $root14 "右") $toml $src
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
左 = { 路径 = "../左", 版本 = "^1.0.0" }
右 = { 路径 = "../右", 版本 = "^1.0.0" }
"@
    New-Pkg (Join-Path $root14 "应用") $toml $src
    $r = Invoke-Pack (Join-Path $root14 "应用")
    Check-Error "版本冲突：E126 + 来源链" $r "E126" "来源链"

    # ---------- 15. install + 离线打包（只读锁与缓存） ----------
    $installDir = Join-Path $root2 "应用"
    $io = (& $micc install --目录 $installDir 2>&1 | Out-String)
    $installed = ($LASTEXITCODE -eq 0) -and (Test-Path (Join-Path $installDir ".砾\锁.toml")) -and (Test-Path (Join-Path $installDir ".砾\包\工具-0.1.0\主.jsh"))
    $r = Invoke-Pack $installDir -Offline
    $out = Run-Jmb (Join-Path $installDir "应用.jmb")
    Check "install + 离线打包可运行" ($installed -and ($r.code -eq 0) -and ($r.out.Contains("离线")) -and ($out -eq "你好，砾`n42")) ($io + $r.out + "`n实际输出：" + $out)

    # ---------- 16. 离线缓存被改坏：校验拒绝 ----------
    $cached = Join-Path $installDir ".砾\包\工具-0.1.0\主.jsh"
    [IO.File]::WriteAllText($cached, "函数 加一(数)：`r`n    返回 数 + 2`r`n", (New-Object System.Text.UTF8Encoding($true)))
    $r = Invoke-Pack $installDir -Offline
    Check-Error "离线缓存被改坏：E126 校验失败" $r "E126" "校验"

    # ---------- 17. 仓库内示例包（examples/packages） ----------
    $exampleDir = Join-Path $root "examples\packages\应用"
    $exampleJmb = Join-Path $sandbox "示例.jmb"
    $o = (& $micc pack --目录 $exampleDir -o $exampleJmb 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $out = Run-Jmb $exampleJmb
    Check "仓库示例包 examples/packages：打包并运行" (($code -eq 0) -and ($out -eq "你好，砾`n42")) ($o + "`n实际输出：" + $out)

    # ---------- 18. 静态索引：按 ^ 约束挑版本（1.2.0），install 后在线/离线都可打包 ----------
    $idxRoot = Join-Path $sandbox "索引"
    $artifact = Join-Path $idxRoot "工件"
    foreach ($v in @("1.0.0", "1.2.0", "2.0.0")) {
        $toml = @"
[包]
名字 = "工具"
版本 = "$v"

[导出]
名字 = ["版本"]
"@
        $src = @"
函数 版本()：
    返回 "$v"
"@
        New-Pkg (Join-Path $artifact "工具-$v") $toml $src
    }
    function Get-PkgHash([string]$dir) {
        $o = (& $micc hash --目录 $dir 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw ("micc hash 失败：" + $o) }
        return $o
    }
    $h100 = Get-PkgHash (Join-Path $artifact "工具-1.0.0")
    $h120 = Get-PkgHash (Join-Path $artifact "工具-1.2.0")
    $h200 = Get-PkgHash (Join-Path $artifact "工具-2.0.0")
    $idxFile = Join-Path $idxRoot "砾索引.toml"
    $idxText = @"
# 静态索引夹具：来源相对本文件
索引版本 = 1

[包]
"工具@1.0.0" = { 来源 = "file:工件/工具-1.0.0", 校验 = "$h100" }
"工具@1.2.0" = { 来源 = "file:工件/工具-1.2.0", 校验 = "$h120" }
"工具@2.0.0" = { 来源 = "file:工件/工具-2.0.0", 校验 = "$h200" }
"@
    [IO.File]::WriteAllText($idxFile, $idxText, (New-Object System.Text.UTF8Encoding($false)))

    $app18 = Join-Path $idxRoot "应用"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "^1.0.0" }
"@
    New-Pkg $app18 $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $app18 --索引 $idxFile 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    $lockText = [IO.File]::ReadAllText((Join-Path $app18 ".砾\锁.toml"))
    $rOff = Invoke-Pack $app18 -Offline
    $outOff = Run-Jmb (Join-Path $app18 "应用.jmb")
    $rOn = Invoke-Pack $app18
    $outOn = Run-Jmb (Join-Path $app18 "应用.jmb")
    Check "索引：^1.0.0 选中 1.2.0，install 后在线/离线都可运行" (($installCode -eq 0) -and $lockText.Contains('版本 = "1.2.0"') -and ($rOff.code -eq 0) -and ($rOn.code -eq 0) -and ($outOff -eq "1.2.0") -and ($outOn -eq "1.2.0")) ($io + $rOff.out + $rOn.out + "`n离线输出：" + $outOff + "`n在线输出：" + $outOn)

    # ---------- 19. 静态索引：* 取最高版本（2.0.0） ----------
    $app19 = Join-Path $idxRoot "应用最大"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "*" }
"@
    New-Pkg $app19 $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $app19 --索引 $idxFile 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    $lockText = [IO.File]::ReadAllText((Join-Path $app19 ".砾\锁.toml"))
    $r = Invoke-Pack $app19 -Offline
    $out = Run-Jmb (Join-Path $app19 "应用.jmb")
    Check "索引：* 选中最高版本 2.0.0" (($installCode -eq 0) -and $lockText.Contains('版本 = "2.0.0"') -and ($r.code -eq 0) -and ($out -eq "2.0.0")) ($io + $r.out + "`n实际输出：" + $out)

    # ---------- 20. 静态索引：没有满足约束的版本（E134） ----------
    $app20 = Join-Path $idxRoot "应用缺版本"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "^1.3.0" }
"@
    New-Pkg $app20 $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $app20 --索引 $idxFile 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    Check-Error "索引：没有满足 ^1.3.0 的版本 → E134" @{ code = $installCode; out = $io } "E134" "没有满足"

    # ---------- 21. 静态索引：索引里没有这个包（E133） ----------
    $app21 = Join-Path $idxRoot "应用缺包"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
无此包 = { 版本 = "*" }
"@
    New-Pkg $app21 $toml '打印(1)'
    $io = (& $micc install --目录 $app21 --索引 $idxFile 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    Check-Error "索引：包不存在 → E133" @{ code = $installCode; out = $io } "E133" "索引里没有包"

    # ---------- 22. 静态索引：内容哈希不符（E136） ----------
    $badIdx = Join-Path $idxRoot "砾索引-坏校验.toml"
    $badText = @"
索引版本 = 1

[包]
"工具@1.0.0" = { 来源 = "file:工件/工具-1.0.0", 校验 = "fnv1a64:0000000000000000" }
"@
    [IO.File]::WriteAllText($badIdx, $badText, (New-Object System.Text.UTF8Encoding($false)))
    $app22 = Join-Path $idxRoot "应用坏校验"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "1.0.0" }
"@
    New-Pkg $app22 $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $app22 --索引 $badIdx 2>&1 | Out-String)
    $installCode = $LASTEXITCODE
    Check-Error "索引：哈希不符 → E136" @{ code = $installCode; out = $io } "E136" "不一致"

    # ---------- 23. HTTP 静态索引：真 curl 下载 + tar 解包 + 哈希校验 + 打包运行 ----------
    # 本机起 python -m http.server；起不来（没有 python / 没端口 / 2 秒内没就绪）就显式跳过，
    # 不拿本地目录冒充 HTTP 证据。
    $pyExe = Get-Command python -ErrorAction SilentlyContinue
    if ($null -eq $pyExe) {
        Write-Host "[跳过] HTTP 索引：本机没有 python，无法起静态服务"
        $script:skip++
    }
    else {
        $httpRoot = Join-Path $sandbox "http"
        $pkgSrc = Join-Path $httpRoot "工具-1.0.0"
        $toml = @"
[包]
名字 = "工具"
版本 = "1.0.0"

[导出]
名字 = ["版本"]
"@
        $srcHttp = @"
函数 版本()：
    返回 "1.0.0"
"@
        New-Pkg $pkgSrc $toml $srcHttp
        $h = Get-PkgHash $pkgSrc
        Compress-Archive -Path (Join-Path $pkgSrc "*") -DestinationPath (Join-Path $httpRoot "tool-1.0.0.zip") -Force
        $zipBytes = (Get-Item (Join-Path $httpRoot "tool-1.0.0.zip")).Length

        $port = 0
        try {
            $probe = New-Object System.Net.Sockets.TcpListener -ArgumentList @([Net.IPAddress]::Loopback, 0)
            $probe.Start()
            $port = $probe.LocalEndpoint.Port
            $probe.Stop()
        }
        catch {
            Write-Host "[跳过] HTTP 索引：挑不到空闲端口"
            $script:skip++
        }

        if ($port -gt 0) {
            $idxHttp = @"
索引版本 = 1

[包]
"工具@1.0.0" = { 来源 = "http://127.0.0.1:$port/tool-1.0.0.zip", 校验 = "$h" }
"@
            [IO.File]::WriteAllText((Join-Path $httpRoot "index.toml"), $idxHttp, (New-Object System.Text.UTF8Encoding($false)))

            $srv = $null
            try {
                $srv = Start-Process -FilePath $pyExe.Source -ArgumentList @("-m", "http.server", "$port", "--bind", "127.0.0.1", "--directory", $httpRoot) -WindowStyle Hidden -PassThru
                $ready = $false
                for ($i = 0; $i -lt 20; $i++) {
                    Start-Sleep -Milliseconds 250
                    try {
                        $c = New-Object System.Net.Sockets.TcpClient
                        $c.Connect("127.0.0.1", $port)
                        $c.Close()
                        $ready = $true
                        break
                    }
                    catch { }
                }
                if (-not $ready) {
                    Write-Host "[跳过] HTTP 索引：静态服务 2 秒内没就绪"
                    $script:skip++
                }
                else {
                    $appHttp = Join-Path $httpRoot "应用"
                    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "1.0.0" }
"@
                    New-Pkg $appHttp $toml '打印(工具.版本())'
                    $savedNoProxy = $env:NO_PROXY
                    $env:NO_PROXY = "127.0.0.1,localhost"
                    $io = (& $micc install --目录 $appHttp --索引 ("http://127.0.0.1:" + $port + "/index.toml") 2>&1 | Out-String)
                    $installCode = $LASTEXITCODE
                    $env:NO_PROXY = $savedNoProxy

                    $lockFile = Join-Path $appHttp ".砾\锁.toml"
                    $lockText = if (Test-Path $lockFile) { [IO.File]::ReadAllText($lockFile) } else { "<缺少锁文件>" }
                    $cached = Join-Path $appHttp ".砾\包\工具-1.0.0\主.jsh"
                    $r = Invoke-Pack $appHttp -Offline
                    $out = Run-Jmb (Join-Path $appHttp "应用.jmb")
                    Write-Host ("[信息] HTTP 索引 1 个条目；归档 " + $zipBytes + " 字节；锁文件：" + ($lockText -replace "`r`n", " | "))
                    Check "HTTP 索引：curl 下载 + tar 解包 + 哈希校验 + 离线打包运行" (($installCode -eq 0) -and ($lockText.Contains('版本 = "1.0.0"')) -and (Test-Path $cached) -and ($r.code -eq 0) -and ($out -eq "1.0.0")) ($io + $r.out + "`n缓存存在：" + (Test-Path $cached) + "`n实际输出：" + $out + "`n锁文件：" + $lockText)
                }
            }
            finally {
                if ($srv -and -not $srv.HasExited) { Stop-Process -Id $srv.Id -Force }
            }
        }
    }

    # ---------- 19. H4b：归档（--归档）+ 二进制包 + micc verify ----------
    $arcDir = Join-Path $sandbox "归档包"
    $toml = @"
[包]
名字 = "归档包"
版本 = "1.0.0"
"@
    New-Pkg $arcDir $toml '打印("归档", 1 + 1)'
    $arcZip = Join-Path $arcDir "归档包-1.0.0.zip"
    $o = (& $micc pack --目录 $arcDir --归档 2>&1 | Out-String)
    $arcOk = ($LASTEXITCODE -eq 0) -and (Test-Path $arcZip) -and
             (Test-Path (Join-Path $arcDir "清单.toml")) -and
             (Test-Path (Join-Path $arcDir "归档包.jmb"))
    Check "归档：产出 zip + 清单.toml + 产物" $arcOk $o

    $v = (& $micc verify $arcZip 2>&1 | Out-String)
    Check "verify 归档：校验通过" (($LASTEXITCODE -eq 0) -and $v.Contains("校验通过") -and $v.Contains("fnv1a64:")) $v

    $plain = Join-Path $sandbox "归档对照.jmb"
    & $micc pack --目录 $arcDir -o $plain | Out-Null
    $same = $false
    if ((Test-Path $plain) -and (Test-Path (Join-Path $arcDir "归档包.jmb"))) {
        $a = [IO.File]::ReadAllBytes($plain)
        $b = [IO.File]::ReadAllBytes((Join-Path $arcDir "归档包.jmb"))
        $same = ($a.Length -eq $b.Length) -and ([Convert]::ToBase64String($a) -eq [Convert]::ToBase64String($b))
    }
    Check "归档内产物与不带 --归档 的 .jmb 逐字节一致" $same ""

    # 篡改产物：校验必须拦住
    $pj = Join-Path $arcDir "归档包.jmb"
    $bytes = [IO.File]::ReadAllBytes($pj)
    $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 0xFF
    [IO.File]::WriteAllBytes($pj, $bytes)
    $o = (& $micc verify $arcDir 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "篡改产物：verify 目录报 E137 校验不符" @{ code = $code; out = $o } "E137" "产物校验不符"

    # 重新归档恢复，再删清单
    & $micc pack --目录 $arcDir --归档 | Out-Null
    Remove-Item (Join-Path $arcDir "清单.toml") -Force
    $o = (& $micc verify $arcDir 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "缺清单：verify 目录报 E137" @{ code = $code; out = $o } "E137" "没有"

    # 清单被改（大小不符）
    & $micc pack --目录 $arcDir --归档 | Out-Null
    $listPath = Join-Path $arcDir "清单.toml"
    $listText = [IO.File]::ReadAllText($listPath)
    $listText = $listText -replace "产物大小 = \d+", "产物大小 = 1"
    [IO.File]::WriteAllText($listPath, $listText, (New-Object System.Text.UTF8Encoding($false)))
    $o = (& $micc verify $arcDir 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "清单被改：verify 报 E137 大小不符" @{ code = $code; out = $o } "E137" "产物大小不符"

    # 二进制包：产物字段 + 预编译 .jmb，没有源码
    $binDir = Join-Path $sandbox "二进制包"
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    $toml = @"
[包]
名字 = "二进制包"
版本 = "0.2.0"
产物 = "二进制包.jmb"

[导出]
名字 = ["加一"]
"@
    [IO.File]::WriteAllText((Join-Path $binDir "砾包.toml"), $toml, (New-Object System.Text.UTF8Encoding($true)))
    Copy-Item $plain (Join-Path $binDir "二进制包.jmb") -Force
    $o = (& $micc pack --目录 $binDir 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "二进制包不带 --归档：明确拒绝" @{ code = $code; out = $o } "E125" "二进制包"

    $o = (& $micc pack --目录 $binDir --归档 2>&1 | Out-String)
    $binZip = Join-Path $binDir "二进制包-0.2.0.zip"
    Check "二进制包 --归档：产出分发件" (($LASTEXITCODE -eq 0) -and (Test-Path $binZip)) $o
    $v = (& $micc verify $binZip 2>&1 | Out-String)
    Check "verify 二进制包：通过且标注二进制包" (($LASTEXITCODE -eq 0) -and $v.Contains("二进制包") -and $v.Contains("校验通过")) $v

    # ---------- 20. H4b：接口清单（micc interface）与导出校验 ----------
    $h4Root = Join-Path $sandbox "H4b接口"
    $libDir = Join-Path $h4Root "库"
    $toml = @"
[包]
名字 = "库"
版本 = "1.2.0"

[导出]
名字 = ["加一", "问候"]
"@
    New-Pkg $libDir $toml @"
函数 加一(数)：
    返回 数 + 1

函数 问候(名)：
    返回 "你好，" + 名
"@
    $o = (& $micc interface --目录 $libDir 2>&1 | Out-String)
    $ifPath = Join-Path $libDir "接口.toml"
    $ifText = if (Test-Path $ifPath) { [IO.File]::ReadAllText($ifPath) } else { "" }
    Check "interface：生成接口清单（函数 + 参数个数）" `
        (($LASTEXITCODE -eq 0) -and $ifText.Contains("加一 = 1") -and $ifText.Contains("问候 = 1") -and
         $ifText.Contains('包 = "库"')) ($o + "`n" + $ifText)

    & $micc pack --目录 $libDir --归档 | Out-Null
    $libZip = Join-Path $libDir "库-1.2.0.zip"
    $v = (& $micc verify $libZip 2>&1 | Out-String)
    Check "归档带上接口：verify 归档通过" (($LASTEXITCODE -eq 0) -and $v.Contains("校验通过")) $v

    # 接口说错参数个数：目录侧必须拦住（归档里仍是旧接口，不受影响）
    $badIface = $ifText -replace "加一 = 1", "加一 = 2"
    [IO.File]::WriteAllText($ifPath, $badIface, (New-Object System.Text.UTF8Encoding($false)))
    $o = (& $micc verify $libDir 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "接口参数个数不符：E137" @{ code = $code; out = $o } "E137" "参数个数不符"
    $v = (& $micc verify $libZip 2>&1 | Out-String)
    Check "归档内接口未改：zip 仍通过" (($LASTEXITCODE -eq 0) -and $v.Contains("校验通过")) $v

    # 接口列了产物里没有的函数
    [IO.File]::WriteAllText($ifPath, ($ifText -replace "问候 = 1", "不存在 = 1"), (New-Object System.Text.UTF8Encoding($false)))
    $o = (& $micc verify $libDir 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "接口列了不存在的函数：E137" @{ code = $code; out = $o } "E137" "没有这个代码对象"

    # 接口 ABI 不符
    [IO.File]::WriteAllText($ifPath, ($ifText -replace "原生ABI = 2", "原生ABI = 3"), (New-Object System.Text.UTF8Encoding($false)))
    $o = (& $micc verify $libDir 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "接口 ABI 不符：E137" @{ code = $code; out = $o } "E137" "原生ABI"

    # 复原后目录校验重新通过
    & $micc interface --目录 $libDir | Out-Null
    $v = (& $micc verify $libDir 2>&1 | Out-String)
    Check "重新生成接口后目录校验恢复通过" (($LASTEXITCODE -eq 0) -and $v.Contains("校验通过")) $v

    # ---------- 21. H4b：源码归档（pack --源码）+ 本地归档作为 install 来源 ----------
    $srcRoot = Join-Path $sandbox "H4b源码归档"
    $lib2 = Join-Path $srcRoot "库"
    $toml = @"
[包]
名字 = "库"
版本 = "1.0.0"

[导出]
名字 = ["版本"]
"@
    New-Pkg $lib2 $toml @"
函数 版本()：
    返回 "1.0.0"
"@
    # 先取目录哈希（归档里只有 砾包.toml + .jsh，解包后必须与之一致）
    $hSrc = Get-PkgHash $lib2
    $o = (& $micc pack --目录 $lib2 --源码 2>&1 | Out-String)
    $srcZip = Join-Path $lib2 "库-1.0.0-源码.zip"
    Check "源码归档：产出 <包名>-<版本>-源码.zip" (($LASTEXITCODE -eq 0) -and (Test-Path $srcZip)) $o

    $o = (& $micc verify $srcZip 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "源码归档不是产物归档：verify 明确提示" @{ code = $code; out = $o } "E137" "源码归档"

    $idx2 = Join-Path $srcRoot "砾索引.toml"
    $idx2Text = @"
索引版本 = 1

[包]
"库@1.0.0" = { 来源 = "file:库/库-1.0.0-源码.zip", 校验 = "$hSrc" }
"@
    [IO.File]::WriteAllText($idx2, $idx2Text, (New-Object System.Text.UTF8Encoding($false)))
    $app21 = Join-Path $srcRoot "应用"
    $toml = @"
[包]
名字 = "应用"
版本 = "1.0.0"

[依赖]
库 = { 版本 = "^1.0.0" }
"@
    New-Pkg $app21 $toml '打印(库.版本())'
    $io = (& $micc install --目录 $app21 --索引 $idx2 2>&1 | Out-String)
    $installed21 = ($LASTEXITCODE -eq 0) -and (Test-Path (Join-Path $app21 ".砾\包\库-1.0.0\主.jsh"))
    Check "install：索引来源指向本地源码归档（zip）" $installed21 $io

    $r = Invoke-Pack $app21 -Offline
    $out = Run-Jmb (Join-Path $app21 "应用.jmb")
    Check "本地归档安装后：离线打包运行" (($r.code -eq 0) -and ($out -eq "1.0.0")) ($r.out + "`n实际输出：" + $out)

    # 索引里的 校验 写错：解包成功但目录哈希不符，install 必须拒绝
    $badIdx = Join-Path $srcRoot "砾索引-坏校验.toml"
    [IO.File]::WriteAllText($badIdx, ($idx2Text -replace $hSrc, "fnv1a64:0000000000000000"), (New-Object System.Text.UTF8Encoding($false)))
    $app22 = Join-Path $srcRoot "应用2"
    New-Pkg $app22 $toml '打印(库.版本())'
    $o = (& $micc install --目录 $app22 --索引 $badIdx 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "归档内容与索引校验不符：install 报 E136" @{ code = $code; out = $o } "E136" "校验"

    # 归档本身被改坏：解包阶段就要拒绝
    $bytes = [IO.File]::ReadAllBytes($srcZip)
    $bytes[$bytes.Length - 30] = $bytes[$bytes.Length - 30] -bxor 0xFF
    [IO.File]::WriteAllBytes($srcZip, $bytes)
    $app23 = Join-Path $srcRoot "应用3"
    New-Pkg $app23 $toml '打印(库.版本())'
    $o = (& $micc install --目录 $app23 --索引 $idx2 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "本地归档损坏：install 报 E135 解包失败" @{ code = $code; out = $o } "E135" "解包"

    # ---------- 22. P2/N7：publish 写目录式仓库 + 静态索引 ----------
    $p2Root = Join-Path $sandbox "P2发布"
    $p2lib = Join-Path $p2Root "源包"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.2.0"

[导出]
名字 = ["版本"]
"@
    New-Pkg $p2lib $toml @"
函数 版本()：
    返回 "1.2.0"
"@
    $p2repo = Join-Path $p2Root "仓库"
    $o = (& $micc publish --目录 $p2lib --仓库 $p2repo 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $p2idx = Join-Path $p2repo "砾索引.toml"
    $p2zip = Join-Path $p2repo "工具\工具-1.2.0.zip"
    $idxText = if (Test-Path $p2idx) { [IO.File]::ReadAllText($p2idx, [Text.Encoding]::UTF8) } else { "" }
    $hPub = Get-PkgHash $p2lib
    Check "publish：产出归档与索引条目" (($code -eq 0) -and (Test-Path $p2zip) -and $idxText.Contains('"工具@1.2.0"') -and $idxText.Contains($hPub) -and $idxText.Contains('来源 = "工具/工具-1.2.0.zip"')) $o

    $o = (& $micc search 工具 --索引 $p2idx 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check "search：按名字命中并打印版本/来源/校验" (($code -eq 0) -and $o.Contains("工具") -and $o.Contains("1.2.0") -and $o.Contains("工具/工具-1.2.0.zip") -and $o.Contains("fnv1a64:")) $o

    $o = (& $micc search 查无此包 --索引 $p2idx 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check "search：无匹配退出码 1" (($code -eq 1) -and $o.Contains("没有匹配的包")) ("退出码 " + $code + "：`n" + $o)

    # publish 的条目能被现有 install 直接消费
    $p2app = Join-Path $p2Root "引用方"
    $toml = @"
[包]
名字 = "引用方"
版本 = "1.0.0"

[依赖]
工具 = { 版本 = "^1.0.0" }
"@
    New-Pkg $p2app $toml '打印(工具.版本())'
    $io = (& $micc install --目录 $p2app --索引 $p2idx 2>&1 | Out-String)
    $r = Invoke-Pack $p2app -Offline
    $out = Run-Jmb (Join-Path $p2app "引用方.jmb")
    Check "publish 的条目：install --索引 后离线打包运行" (($r.code -eq 0) -and ($out -eq "1.2.0")) ($io + $r.out + "`n实际输出：" + $out)

    # 同版本重复发布：幂等（归档覆盖，索引仍 1 条）
    $o = (& $micc publish --目录 $p2lib --仓库 $p2repo 2>&1 | Out-String)
    $code = $LASTEXITCODE
    $idxText = [IO.File]::ReadAllText($p2idx, [Text.Encoding]::UTF8)
    $n = ($idxText -split "`n" | Where-Object { $_ -match '^\s*"' }).Count
    Check "publish：同版本重复发布幂等（索引仍 1 条）" (($code -eq 0) -and ($n -eq 1)) ("条目数 " + $n + "：`n" + $o)

    # 第二个版本：索引累计 2 条，检索按版本从高到低
    $p2lib2 = Join-Path $p2Root "源包14"
    $toml = @"
[包]
名字 = "工具"
版本 = "1.4.0"

[导出]
名字 = ["版本"]
"@
    New-Pkg $p2lib2 $toml @"
函数 版本()：
    返回 "1.4.0"
"@
    $o = (& $micc publish --目录 $p2lib2 --仓库 $p2repo 2>&1 | Out-String)
    $o2 = (& $micc search 工具 --索引 $p2idx 2>&1 | Out-String)
    $idxText = [IO.File]::ReadAllText($p2idx, [Text.Encoding]::UTF8)
    $n = ($idxText -split "`n" | Where-Object { $_ -match '^\s*"' }).Count
    $i14 = $o2.IndexOf("1.4.0")
    $i12 = $o2.IndexOf("1.2.0")
    Check "publish：第二版本累加，检索高版本在前" (($n -eq 2) -and $o2.Contains("命中 2 条") -and ($i14 -ge 0) -and ($i14 -lt $i12)) ("条目数 " + $n + "：`n" + $o + $o2)

    # 二进制包不能进索引（install 消费不了）
    $p2bin = Join-Path $p2Root "二进制包"
    $toml = @"
[包]
名字 = "二进制包"
版本 = "1.0.0"
产物 = "二进制包.jmb"
"@
    New-Pkg $p2bin $toml '打印("x")'
    $o = (& $micc publish --目录 $p2bin --仓库 $p2repo 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "publish：二进制包明确拒绝（E138）" @{ code = $code; out = $o } "E138" "二进制包"

    # HTTP 索引只读：发布必须拒绝
    $o = (& $micc publish --目录 $p2lib --索引 "https://example.com/li/砾索引.toml" 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check-Error "publish：HTTP 索引拒绝（E138）" @{ code = $code; out = $o } "E138" "HTTP"

    # 只给 --索引（不给 --仓库）：仓库取索引所在目录
    $p2repo2 = Join-Path $p2Root "仓库2"
    New-Item -ItemType Directory -Path $p2repo2 -Force | Out-Null
    $p2idx2 = Join-Path $p2repo2 "砾索引.toml"
    $o = (& $micc publish --目录 $p2lib --索引 $p2idx2 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Check "publish：只给 --索引 时仓库取索引所在目录" (($code -eq 0) -and (Test-Path (Join-Path $p2repo2 "工具\工具-1.2.0.zip"))) $o

    Write-Host ("包用例：共 " + $script:total + " 个，失败 " + $script:fail + " 个，跳过 " + $script:skip + " 个")
    if ($script:fail -gt 0) { exit 1 }
    exit 0
}
finally {
    # 用 .NET 直接删临时目录：临时夹具不值得进回收站，也更少受策略拦截。
    if ($sandbox -and (Test-Path $sandbox)) {
        try { [IO.Directory]::Delete($sandbox, $true) } catch { }
    }
    Pop-Location
}
