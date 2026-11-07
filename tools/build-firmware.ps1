# 构建 STM32F103C8T6 固件骨架（砾 / JMB v1）
# 产物：build\stm32\jishim.elf、jishim.bin、jishim.hex、jishim.map
# 用法：.\tools\build-firmware.ps1 [-Example 你好]

param(
    [string]$Example = "你好"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
. "$PSScriptRoot\toolchains.ps1"

if (-not $ArmGcc) { throw "找不到 arm-none-eabi-gcc（设置 JM_ARM_GCC）" }

# specs 要用绝对路径：本机工具链按相对名查找会失败
$tcRoot = Split-Path (Split-Path $ArmGcc)
$NanoSpecs = Join-Path $tcRoot "arm-none-eabi\lib\nano.specs"
$NosysSpecs = Join-Path $tcRoot "arm-none-eabi\lib\nosys.specs"
foreach ($p in @($NanoSpecs, $NosysSpecs)) {
    if (-not (Test-Path $p)) { throw ("找不到 specs 文件：" + $p) }
}

Push-Location $root
try {
    $out = "build\stm32"
    New-Item -ItemType Directory -Force -Path $out | Out-Null

    Write-Host "== 1/5 构建 micc"
    & cargo build --release --manifest-path "compiler\Cargo.toml" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "micc 构建失败" }
    $micc = "compiler\target\release\micc.exe"

    Write-Host "== 2/5 编译示例为 .jmb"
    & $micc ("examples\" + $Example + ".jsh") -o "$out\app.jmb" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw ("编译失败：" + $Example + ".jsh") }

    Write-Host "== 3/5 生成内嵌字节数组"
    $bytes = [System.IO.File]::ReadAllBytes((Join-Path $root "$out\app.jmb"))
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append("#include <stdint.h>`n")
    [void]$sb.Append("const unsigned char jm_app_jmb[] = {`n")
    for ($i = 0; $i -lt $bytes.Length; $i++) {
        if ($i % 12 -eq 0) { [void]$sb.Append("    ") }
        [void]$sb.Append(("0x{0:X2}" -f $bytes[$i]))
        if ($i -lt $bytes.Length - 1) { [void]$sb.Append(",") }
        if ($i % 12 -eq 11) { [void]$sb.Append("`n") }
    }
    [void]$sb.Append("`n};`n")
    [void]$sb.Append("const uint32_t jm_app_jmb_len = $($bytes.Length)u;`n")
    [System.IO.File]::WriteAllText(
        (Join-Path $root "$out\jm_blob.c"),
        $sb.ToString(),
        (New-Object System.Text.UTF8Encoding($false)))

    Write-Host "== 4/5 交叉编译并链接"
    $src = @(
        "runtime\src\jm_heap.c", "runtime\src\jm_vm.c", "runtime\src\jm_builtin.c",
        "runtime\src\jm_loader.c", "runtime\src\jm_error.c",
        "runtime\src\jm_native.c",
        "firmware\stm32f103\stm32f103_port.c",
        "firmware\stm32f103\f103_native.c",
        "firmware\stm32f103\jmk.c",
        "firmware\stm32f103\main.c",
        "$out\jm_blob.c"
    )
    & $ArmGcc `
        -mcpu=cortex-m3 -mthumb -std=c99 -Os -g `
        -Wall -Wextra -ffreestanding -fno-common `
        -fdata-sections -ffunction-sections `
        -DJM_HEAP_SIZE=6144 `
        -I "runtime\include" -I "runtime\src" -I "firmware\stm32f103" `
        -nostartfiles `
        -T "firmware\stm32f103\stm32f103c8.ld" `
        "-Wl,--gc-sections" "-Wl,--print-memory-usage" `
        "-Wl,-Map=$out\jishim.map" `
        "-specs=$NanoSpecs" "-specs=$NosysSpecs" `
        "firmware\stm32f103\startup_stm32f103c8.s" `
        "firmware\stm32f103\jmk_port_cm3.s" `
        $src `
        -o "$out\jishim.elf"
    if ($LASTEXITCODE -ne 0) { throw "链接失败" }

    Write-Host "== 5/5 生成 bin / hex 并统计体积"
    $objcopy = Join-Path (Split-Path $ArmGcc) "arm-none-eabi-objcopy.exe"
    & $objcopy -O binary "$out\jishim.elf" "$out\jishim.bin"
    & $objcopy -O ihex "$out\jishim.elf" "$out\jishim.hex"
    if ($ArmSize) {
        & $ArmSize "$out\jishim.elf"
    }
    Write-Host ("固件产物：" + (Join-Path $root "$out") + "\jishim.{elf,bin,hex}")
}
finally {
    Pop-Location
}
