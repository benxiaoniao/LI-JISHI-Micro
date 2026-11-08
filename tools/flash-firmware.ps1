# 烧录 STM32F103C8T6（砾 固件骨架）
# 默认只打印命令，不执行。需要真正烧录时加 -Apply。
# 用法：.\tools\flash-firmware.ps1            # 预览命令
#       .\tools\flash-firmware.ps1 -Apply     # 实际烧录（需要用户明确授权）

param(
    [switch]$Apply
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

$cli = if ($env:STM32_PROGRAMMER_CLI) {
    $env:STM32_PROGRAMMER_CLI
} else {
    "F:\STM32CubeIDE_2.2.0\STM32CubeIDE\plugins\com.st.stm32cube.ide.mcu.externaltools.cubeprogrammer.win32_2.2.500.202603051304\tools\bin\STM32_Programmer_CLI.exe"
}
$bin = Join-Path $root "build\stm32\jishim.bin"
$addr = "0x08000000"

if (-not (Test-Path $cli)) {
    throw ("找不到 STM32_Programmer_CLI.exe：" + $cli + "`n装 STM32CubeProgrammer 后设环境变量 STM32_PROGRAMMER_CLI，或改本脚本顶部 `$cli（见 docs\工具链.md 第 5 步）")
}
if (-not (Test-Path $bin)) { throw "找不到固件：$bin（先运行 .\tools\build-firmware.ps1）" }

# 只读：列出探针并连接；写入：-d 下载 + 校验 + 复位
$cmd = @($cli, "-c", "port=SWD", "mode=UR", "-d", $bin, $addr, "-v", "-rst")

Write-Host "烧录命令："
Write-Host ("  " + ($cmd -join " "))
Write-Host ""
Write-Host "只读检查（不写入）可用："
Write-Host ("  " + $cli + " -c port=SWD mode=UR -q")

if (-not $Apply) {
    Write-Host ""
    Write-Host "仅预览，未执行。确认要烧录后加 -Apply。"
    exit 0
}

Write-Host "== 开始烧录"
& $cmd[0] $cmd[1..($cmd.Length - 1)]
exit $LASTEXITCODE
