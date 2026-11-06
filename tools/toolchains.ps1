# 工具链定位（本机默认值 + 环境变量覆盖）
# 用法：在其它脚本里 . "$PSScriptRoot\toolchains.ps1"

function Find-Tool($envName, $exeName, $fallback) {
    if ((Get-Item "env:$envName" -ErrorAction SilentlyContinue).Value) {
        return (Get-Item "env:$envName").Value
    }
    $cmd = Get-Command $exeName -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    if ($fallback -and (Test-Path $fallback)) { return $fallback }
    return $null
}

$HostGcc = Find-Tool "JM_HOST_GCC" "gcc" "C:\Users\benxi\.local\share\mingw64-16.1\mingw64\bin\gcc.exe"
$ArmGcc  = Find-Tool "JM_ARM_GCC"  "arm-none-eabi-gcc" "C:\Users\benxi\.local\share\arm-gnu-toolchain-13.3.1\xpack-arm-none-eabi-gcc-13.3.1-1.1\bin\arm-none-eabi-gcc.exe"
$ArmSize = Find-Tool "JM_ARM_SIZE" "arm-none-eabi-size" "C:\Users\benxi\.local\share\arm-gnu-toolchain-13.3.1\xpack-arm-none-eabi-gcc-13.3.1-1.1\bin\arm-none-eabi-size.exe"

if (-not $HostGcc) {
    Write-Warning "找不到主机 C 编译器（gcc）。设置 JM_HOST_GCC 指向 gcc.exe。"
}
if (-not $ArmGcc) {
    Write-Warning "找不到 arm-none-eabi-gcc。设置 JM_ARM_GCC；没有它只是不能交叉编译，不影响主机验证。"
}
