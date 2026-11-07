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
    Write-Host "  [PASS] 编译通过（导入 引脚）"
}

& $micc (Join-Path $root "examples\board\ADC采样.jsh") -o (Join-Path $root "build\native-adc-ok.jmb") | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Host "  [FAIL] ADC / 定时器示例被拒绝"
    $script:fail++
}
else {
    Write-Host "  [PASS] 编译通过（导入 ADC + 定时器）"
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

Expect-CompileError "tests\errors\原生名拼错.jsh" "E115" "引脚：名字拼错应报 E115"
Expect-CompileError "tests\errors\原生参数个数.jsh" "E115" "引脚：参数个数不符应报 E115"
Expect-CompileError "tests\errors\原生未导入.jsh" "E113" "引脚：未导入就调用应报 E113"

Expect-CompileError "tests\errors\ADC名拼错.jsh" "E115" "ADC：名字拼错应报 E115"
Expect-CompileError "tests\errors\ADC参数个数.jsh" "E115" "ADC：参数个数不符应报 E115"
Expect-CompileError "tests\errors\ADC未导入.jsh" "E113" "ADC：未导入就调用应报 E113"

Expect-CompileError "tests\errors\定时器名拼错.jsh" "E115" "定时器：名字拼错应报 E115"
Expect-CompileError "tests\errors\定时器参数个数.jsh" "E115" "定时器：参数个数不符应报 E115"
Expect-CompileError "tests\errors\定时器未导入.jsh" "E113" "定时器：未导入就调用应报 E113"

if ($script:fail -ne 0) {
    Write-Host ("原生导入回归失败：" + $script:fail + " 项")
    exit 1
}
Write-Host "原生导入回归：11/11 通过（引脚 / ADC / 定时器各 3 个错误用例 + 2 个正确示例）"
exit 0
