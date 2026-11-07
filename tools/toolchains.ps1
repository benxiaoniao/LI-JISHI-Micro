# 工具链定位与版本校验（可复现）
#
# 解析顺序：包内 tools/toolchains/<名字>/bin → 外部缓存 → 环境变量 → PATH
#   1) 包内：仓库路径是纯 ASCII 时可以直接放包内，整台机器不依赖 PATH
#   2) 外部缓存：%LOCALAPPDATA%\jishi\toolchains\<名字>（可用 JM_TOOLCHAIN_ROOT 改）
#      仓库路径含中文/非 ASCII 时必须走这里：MinGW 版 gcc/ld 用 ANSI 文件 API，
#      中文目录下会报 "cannot find crti.o / -lgcc"
#   3) 环境变量：JM_HOST_GCC / JM_ARM_GCC / JM_ARM_SIZE（显式指定）
#   4) PATH：最后兜底
#
# 用法 A（被其它脚本引用）：
#   . "$PSScriptRoot\toolchains.ps1"     # 得到 $HostGcc / $ArmGcc / $ArmSize
# 用法 B（自检）：
#   powershell -ExecutionPolicy Bypass -File tools\toolchains.ps1
#
# 版本与哈希清单：tools/toolchains/toolchain.lock.json

$ToolchainDir  = Join-Path $PSScriptRoot "toolchains"
$ToolchainLock = Join-Path $ToolchainDir "toolchain.lock.json"
# 外部缓存目录：必须放在纯 ASCII 路径下（MinGW 版 gcc/ld 用 ANSI 文件 API，中文目录会找不到 crt/lib）
$ToolchainCache = if ($env:JM_TOOLCHAIN_ROOT) { $env:JM_TOOLCHAIN_ROOT } else { Join-Path $env:LOCALAPPDATA "jishi\toolchains" }
$script:ToolchainIssues = @()
$script:ToolchainInfo   = @()

function Read-ToolchainLock {
    if (-not (Test-Path $ToolchainLock)) { return $null }
    return (Get-Content -Path $ToolchainLock -Raw -Encoding UTF8 | ConvertFrom-Json)
}

function Find-Tool {
    param(
        [string]$EnvName,
        [string]$ExeName,
        [string]$BundleName
    )

    if ($BundleName) {
        $candidate = Join-Path $ToolchainDir (Join-Path $BundleName ("bin\" + $ExeName))
        if (Test-Path $candidate) {
            return @{ Path = $candidate; Origin = ("包内 tools/toolchains/" + $BundleName) }
        }
        $cached = Join-Path $ToolchainCache (Join-Path $BundleName ("bin\" + $ExeName))
        if (Test-Path $cached) {
            return @{ Path = $cached; Origin = ("缓存 " + $ToolchainCache + "\" + $BundleName) }
        }
    }

    $envValue = (Get-Item ("env:" + $EnvName) -ErrorAction SilentlyContinue).Value
    if ($envValue) {
        return @{ Path = $envValue; Origin = ("环境变量 " + $EnvName) }
    }

    $cmd = Get-Command $ExeName -ErrorAction SilentlyContinue
    if ($cmd) {
        return @{ Path = $cmd.Source; Origin = "PATH" }
    }
    return $null
}

function Test-ToolHash {
    param($Tool, [string]$Want, [string]$Label)

    if (-not $Tool) { return }
    if (-not $Want) { return }
    try {
        $actual = (Get-FileHash -Path $Tool.Path -Algorithm SHA256).Hash
    } catch {
        $script:ToolchainIssues += ("{0} 哈希读取失败：{1}" -f $Label, $_)
        return
    }
    if ($actual -eq $Want) {
        $script:ToolchainInfo += ("{0}: sha256 与锁文件一致（{1}）" -f $Label, $actual.Substring(0, 16))
    } else {
        $script:ToolchainIssues += ("{0} 哈希不符：期望 {1}，实测 {2}" -f $Label, $Want, $actual)
    }
}

function Test-ToolVersion {
    param($Tool, [string]$Want, [string]$Label)

    if (-not $Tool) { return }
    try {
        $line = (& $Tool.Path --version 2>$null | Select-Object -First 1)
    } catch {
        $line = ""
    }
    if ($line -and $Want -and ($line -match [regex]::Escape($Want))) {
        $script:ToolchainInfo += ("{0}: {1}（{2}）" -f $Label, $line, $Tool.Origin)
        return
    }
    $script:ToolchainIssues += ("{0} 版本不符：期望 {1}，实测 {2}（{3}）" -f $Label, $Want, $line, $Tool.Origin)
}

$lock = Read-ToolchainLock
$armName = "arm-none-eabi"
$hostName = "host-gcc"
$armLock  = $null
$hostLock = $null
if ($lock) {
    $armLock  = $lock.toolchains.$armName
    $hostLock = $lock.toolchains.$hostName
}

$hostTool     = Find-Tool "JM_HOST_GCC" "gcc.exe" $hostName
$armTool      = Find-Tool "JM_ARM_GCC"  "arm-none-eabi-gcc.exe" $armName
$armSizeTool  = Find-Tool "JM_ARM_SIZE" "arm-none-eabi-size.exe" $armName

$HostGcc = if ($hostTool) { $hostTool.Path } else { $null }
$ArmGcc  = if ($armTool)  { $armTool.Path }  else { $null }
$ArmSize = if ($armSizeTool) { $armSizeTool.Path } else { $null }

if (-not $env:JM_SKIP_TOOLCHAIN_CHECK) {
    if ($armLock)  { Test-ToolVersion $armTool     $armLock.version  "arm-none-eabi-gcc" }
    if ($hostLock) { Test-ToolVersion $hostTool    $hostLock.version "gcc" }
    if ($armLock -and $armSizeTool) {
        $sizeVersion = if ($armLock.binutils_version) { $armLock.binutils_version } else { $armLock.version }
        Test-ToolVersion $armSizeTool $sizeVersion "arm-none-eabi-size"
    }
    if ($lock) {
        foreach ($entry in $lock.toolchains.PSObject.Properties) {
            foreach ($file in $entry.Value.files) {
                $abs = Join-Path $ToolchainDir (Join-Path $entry.Name $file.path)
                if (-not (Test-Path $abs)) { $abs = Join-Path $ToolchainCache (Join-Path $entry.Name $file.path) }
                if (Test-Path $abs) {
                    Test-ToolHash @{ Path = $abs; Origin = "本地" } $file.sha256 ($entry.Name + "/" + $file.path)
                }
            }
        }
    }
}

if (-not $HostGcc) {
    $script:ToolchainIssues += ("找不到主机 C 编译器 gcc：放到 " + $ToolchainCache + "\host-gcc\bin，或设置 JM_HOST_GCC")
}
if (-not $ArmGcc) {
    $script:ToolchainIssues += ("找不到 arm-none-eabi-gcc：放到 " + $ToolchainCache + "\arm-none-eabi\bin，或设置 JM_ARM_GCC")
}

# 被其它脚本引用时只给变量，不输出；直接运行时打印自检报告
if ($MyInvocation.InvocationName -ne '.') {
    if ($lock) { "工具链锁定：tools/toolchains/toolchain.lock.json（" + $lock.updated + "）" } else { "缺少锁定文件 tools/toolchains/toolchain.lock.json" }
    "包内目录：" + $ToolchainDir
    ""
    foreach ($line in $script:ToolchainInfo) { "  [OK] " + $line }
    foreach ($issue in $script:ToolchainIssues) { "  [问题] " + $issue }
    ""
    if ($script:ToolchainIssues.Count -gt 0) { exit 1 } else { "全部符合锁定版本。"; exit 0 }
}
