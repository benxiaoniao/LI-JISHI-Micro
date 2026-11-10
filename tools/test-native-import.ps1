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

& $micc (Join-Path $root "examples\任务.jsh") -o (Join-Path $root "build\native-task-ok.jmb") | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Host "  [FAIL] 任务示例被拒绝"
    $script:fail++
}
else {
    Write-Host "  [PASS] 编译通过（导入 任务）"
}

& $micc (Join-Path $root "examples\队列.jsh") -o (Join-Path $root "build\native-ipc-ok.jmb") | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Host "  [FAIL] 队列 / 信号量示例被拒绝"
    $script:fail++
}
else {
    Write-Host "  [PASS] 编译通过（导入 队列 + 信号量）"
}

& $micc (Join-Path $root "examples\外部中断.jsh") -o (Join-Path $root "build\native-irq-ok.jmb") | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Host "  [FAIL] 外部中断示例被拒绝"
    $script:fail++
}
else {
    Write-Host "  [PASS] 编译通过（导入 外部中断）"
}

& $micc (Join-Path $root "examples\board\PWM呼吸灯.jsh") -o (Join-Path $root "build\native-pwm-ok.jmb") | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Host "  [FAIL] PWM 呼吸灯示例被拒绝"
    $script:fail++
}
else {
    Write-Host "  [PASS] 编译通过（导入 定时器 + PWM）"
}

& $micc (Join-Path $root "examples\board\OLED文字.jsh") -o (Join-Path $root "build\native-i2c-ok.jmb") | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Host "  [FAIL] OLED 文字示例被拒绝"
    $script:fail++
}
else {
    Write-Host "  [PASS] 编译通过（导入 总线 + I2C）"
}

& $micc (Join-Path $root "examples\SPI回环.jsh") -o (Join-Path $root "build\native-spi-ok.jmb") | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Host "  [FAIL] SPI 回环示例被拒绝"
    $script:fail++
}
else {
    Write-Host "  [PASS] 编译通过（导入 总线 + SPI）"
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

Expect-CompileError "tests\errors\PWM名拼错.jsh" "E115" "PWM：名字拼错应报 E115"
Expect-CompileError "tests\errors\PWM参数个数.jsh" "E115" "PWM：参数个数不符应报 E115"
Expect-CompileError "tests\errors\PWM未导入.jsh" "E113" "PWM：未导入就调用应报 E113"

Expect-CompileError "tests\errors\I2C名拼错.jsh" "E115" "总线：名字拼错应报 E115"
Expect-CompileError "tests\errors\I2C参数个数.jsh" "E115" "总线：参数个数不符应报 E115"
Expect-CompileError "tests\errors\I2C未导入.jsh" "E113" "总线：未导入就调用应报 E113"

Expect-CompileError "tests\errors\SPI名拼错.jsh" "E115" "SPI：名字拼错应报 E115"
Expect-CompileError "tests\errors\SPI参数个数.jsh" "E115" "SPI：参数个数不符应报 E115"
Expect-CompileError "tests\errors\SPI未导入.jsh" "E113" "SPI：未导入就调用应报 E113"

Expect-CompileError "tests\errors\任务参数个数.jsh" "E115" "任务：参数个数不符应报 E115"
Expect-CompileError "tests\errors\任务未导入.jsh" "E113" "任务：未导入就调用应报 E113"

Expect-CompileError "tests\errors\队列名拼错.jsh" "E115" "队列：名字拼错应报 E115"
Expect-CompileError "tests\errors\队列参数个数.jsh" "E115" "队列：参数个数不符应报 E115"
Expect-CompileError "tests\errors\队列未导入.jsh" "E113" "队列：未导入就调用应报 E113"

Expect-CompileError "tests\errors\信号量名拼错.jsh" "E115" "信号量：名字拼错应报 E115"
Expect-CompileError "tests\errors\信号量参数个数.jsh" "E115" "信号量：参数个数不符应报 E115"
Expect-CompileError "tests\errors\信号量未导入.jsh" "E113" "信号量：未导入就调用应报 E113"

Expect-CompileError "tests\errors\中断名拼错.jsh" "E115" "外部中断：名字拼错应报 E115"
Expect-CompileError "tests\errors\中断参数个数.jsh" "E115" "外部中断：参数个数不符应报 E115"
Expect-CompileError "tests\errors\中断未导入.jsh" "E113" "外部中断：未导入就调用应报 E113"

if ($script:fail -ne 0) {
    Write-Host ("原生导入回归失败：" + $script:fail + " 项")
    exit 1
}
Write-Host "原生导入回归：37/37 通过（引脚 / ADC / 定时器 / 队列 / 信号量 / 外部中断 / PWM / 总线 / SPI 各 3 个错误用例，任务 2 个错误用例；8 个正确示例）"
exit 0
