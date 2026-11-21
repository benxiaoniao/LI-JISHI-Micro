# `micc test` 自身回归：正/负/期望输出/编译错误/超时/过滤/列出。
# 夹具全部建在临时目录，不碰仓库里 tests/cases 之外的产物。
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")
$micc = Join-Path $root ("compiler/target/release/micc" + $ExeSuffix)
$jmrun = Join-Path $root ("build/jmrun" + $ExeSuffix)

Push-Location $root
try {
    if (-not (Test-Path $micc)) { throw "先运行 tools/build.ps1" }
    if (-not (Test-Path $jmrun)) { throw "先运行 tools/build.ps1（缺 build/jmrun）" }

    $sandbox = Join-Path ([IO.Path]::GetTempPath()) ("li-micctest-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Path $sandbox | Out-Null
    $script:fail = 0
    $script:total = 0

    function Write-Utf8([string]$path, [string]$text) {
        [IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
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
    function Run-Test([string[]]$argv) {
        $out = (& $micc test @argv 2>&1 | Out-String)
        return @{ code = $LASTEXITCODE; out = $out }
    }

    # 仓库自带用例目录
    $r = Run-Test @("--jmrun", $jmrun)
    Check "tests/cases 默认目录全部通过" `
        (($r.code -eq 0) -and $r.out.Contains("通过 3") -and $r.out.Contains("[通过]")) $r.out

    # 临时正/负用例
    $okDir = Join-Path $sandbox "正常"
    New-Item -ItemType Directory -Path $okDir -Force | Out-Null
    Write-Utf8 (Join-Path $okDir "通过.jsh") "# @期望输出: 你好`n打印(`"你好`")`n"
    Write-Utf8 (Join-Path $okDir "断言.jsh") "# @期望错误: E027`n断言 假, `"坏了`"`n"
    Write-Utf8 (Join-Path $okDir "编译错.jsh") "# @期望错误: E109`n断言`n"
    Write-Utf8 (Join-Path $okDir "多行.jsh") "# @期望输出: 甲`n# @期望输出: 乙`n打印(`"甲`")`n打印(`"乙`")`n"
    Write-Utf8 (Join-Path $okDir "命名.jsh") "# @名称: 别名`n打印(`"x`")`n"

    $r = Run-Test @("--目录", $okDir, "--jmrun", $jmrun, "--超时", "3000")
    Check "正/负/输出/编译错误四类都通过" `
        (($r.code -eq 0) -and $r.out.Contains("通过 5") -and (-not $r.out.Contains("[不通过]")) -and
         $r.out.Contains("[通过] 别名")) $r.out

    $r = Run-Test @("通过", "--目录", $okDir, "--jmrun", $jmrun)
    Check "名称过滤：只跑匹配的 1 个" `
        (($r.code -eq 0) -and $r.out.Contains("1 个用例，通过 1") -and (-not $r.out.Contains("断言"))) $r.out

    $r = Run-Test @("--目录", $okDir, "--列出")
    Check "列出模式：输出名称与文件且不执行" `
        (($r.code -eq 0) -and $r.out.Contains("别名") -and (-not $r.out.Contains("[通过]"))) $r.out

    # 超时
    $timeoutDir = Join-Path $sandbox "超时"
    New-Item -ItemType Directory -Path $timeoutDir -Force | Out-Null
    Write-Utf8 (Join-Path $timeoutDir "死循环.jsh") "# @超时: 300`n令 i = 0`n当 真:`n    i += 1`n"
    $r = Run-Test @("--目录", $timeoutDir, "--jmrun", $jmrun)
    Check "用例内 @超时：进程被终止并计为超时" `
        (($r.code -eq 1) -and $r.out.Contains("[超时]") -and $r.out.Contains("超时 1")) $r.out

    $globalTimeoutDir = Join-Path $sandbox "全局超时"
    New-Item -ItemType Directory -Path $globalTimeoutDir -Force | Out-Null
    Write-Utf8 (Join-Path $globalTimeoutDir "死循环.jsh") "令 i = 0`n当 真:`n    i += 1`n"
    $r = Run-Test @("--目录", $globalTimeoutDir, "--jmrun", $jmrun, "--超时", "200")
    Check "全局 --超时 覆盖默认值" `
        (($r.code -eq 1) -and $r.out.Contains("超过 200 ms")) $r.out

    # 失败用例
    $badDir = Join-Path $sandbox "失败"
    New-Item -ItemType Directory -Path $badDir -Force | Out-Null
    Write-Utf8 (Join-Path $badDir "失败.jsh") "断言 假, `"没通过`"`n"
    $r = Run-Test @("--目录", $badDir, "--jmrun", $jmrun)
    Check "失败用例：退出码 1 并打印 E027" `
        (($r.code -eq 1) -and $r.out.Contains("[不通过]") -and $r.out.Contains("E027")) $r.out

    $r = Run-Test @("--目录", (Join-Path $sandbox "不存在"), "--jmrun", $jmrun)
    Check "缺用例目录：E125" `
        (($r.code -eq 1) -and $r.out.Contains("E125")) $r.out

    Write-Host ("micc test 用例：共 " + $script:total + " 个，失败 " + $script:fail + " 个")
    if ($script:fail -gt 0) { exit 1 }
    exit 0
}
finally {
    if ($sandbox -and (Test-Path $sandbox)) {
        try { [IO.Directory]::Delete($sandbox, $true) } catch { }
    }
    Pop-Location
}
