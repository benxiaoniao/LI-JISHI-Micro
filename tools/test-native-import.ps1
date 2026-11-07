# 原生模块导入的编译期回归
# 覆盖：正确导入通过、名字拼错报错、参数个数不符报错、未导入就调用报错

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$micc = Join-Path $root "compiler\target\release\micc.exe"

if (-not (Test-Path $micc)) { throw "先构建 micc：" + $micc }

$script:fail = 0
New-Item -ItemType Directory -Force -Path (Join-Path $root "build") | Out-Null

Write-Host "== 原生导入：正确导入应通过"
& $micc (Join-Path $root "examples\board\点灯.jsh") -o (Join-Path $root "build\native-import-ok.jmb") | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Host "  [FAIL] 正确导入被拒绝"
    $script:fail++
}
else {
    Write-Host "  [PASS] 编译通过"
}

function Expect-CompileError {
    param([string]$File, [string]$Code, [string]$Title)
    Write-Host ("== " + $Title)
    # 编译失败会把中文错误写到 stderr；临时放宽 Stop，避免被当成脚本异常。
    $saved = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $out = & $micc (Join-Path $root $File) -o (Join-Path $root "build\native-import-bad.jmb") 2>&1 | Out-String
    $exit = $LASTEXITCODE
    $ErrorActionPreference = $saved
    $ok = ($exit -ne 0) -and ($out -match $Code)
    if ($ok) {
        Write-Host ("  [PASS] " + $Code)
    }
    else {
        Write-Host ("  [FAIL] 期望 " + $Code + "，实际输出：" + $out.Trim())
        $script:fail++
    }
}

Expect-CompileError "tests\errors\原生名拼错.jsh" "E115" "名字拼错应报 E115"
Expect-CompileError "tests\errors\原生参数个数.jsh" "E115" "参数个数不符应报 E115"
Expect-CompileError "tests\errors\原生未导入.jsh" "E113" "未导入就调用应报 E113"

if ($script:fail -ne 0) {
    Write-Host ("原生导入回归失败：" + $script:fail + " 项")
    exit 1
}
Write-Host "原生导入回归：4/4 通过"
exit 0
