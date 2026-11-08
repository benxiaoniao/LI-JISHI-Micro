# Editor integration tests inside the VS Code extension host.
# Exits 2 when VS Code is not installed; otherwise returns the test host exit code.
$ErrorActionPreference = "Stop"
$ext = Split-Path -Parent $PSScriptRoot
$root = Split-Path -Parent (Split-Path -Parent $ext)
$suite = Join-Path $PSScriptRoot "suite\index.js"

# 集成夹具在仓库里提交（test\fixtures），每次运行同步到 build\editor-it；
# build 目录被 gitignore，不能指望它在新克隆里存在。
$fixtures = Join-Path $PSScriptRoot "fixtures"
$itDir = Join-Path $root "build\editor-it"
New-Item -ItemType Directory -Force -Path $itDir | Out-Null
Copy-Item -Recurse -Force (Join-Path $fixtures "*") $itDir

$code = $env:VSCODE_EXE
if (-not $code) {
    $candidates = @(
        "E:\IDE\Microsoft VS Code\Code.exe",
        "$env:LOCALAPPDATA\Programs\Microsoft VS Code\Code.exe",
        "$env:ProgramFiles\Microsoft VS Code\Code.exe"
    )
    $code = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $code) {
    Write-Host "SKIP: VS Code not found (set VSCODE_EXE to override)"
    exit 2
}

$tmp = Join-Path $env:TEMP ("li-vscode-it-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $args = @(
        "--extensionDevelopmentPath=$ext",
        "--extensionTestsPath=$suite",
        "--user-data-dir=$tmp\user",
        "--extensions-dir=$tmp\ext",
        "--disable-gpu",
        "--disable-telemetry",
        "--disable-workspace-trust",
        "--skip-welcome",
        "--new-window",
        $root
    )
    $proc = Start-Process -FilePath $code -ArgumentList $args -Wait -PassThru -NoNewWindow
    Write-Host ("VS Code integration exit code: " + $proc.ExitCode)
    exit $proc.ExitCode
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
