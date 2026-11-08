$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot

Push-Location $root
try {
    $micc = "compiler\target\release\micc.exe"
    $jmrun = "build\jmrun.exe"
    $cases = @(
        @{ file = "除零.jsh";       code = "E007"; stage = "运行" },
        @{ file = "小数除零.jsh";   code = "E007"; stage = "运行" },
        @{ file = "真除法.jsh";     code = "E023"; stage = "运行" },
        @{ file = "未定义名字.jsh"; code = "E004"; stage = "运行" },
        @{ file = "小数溢出.jsh";   code = "E009"; stage = "运行" },
        @{ file = "小数下标.jsh";   code = "E013"; stage = "运行" },
        @{ file = "闭包赋值.jsh";   code = "E117"; stage = "编译" },
        @{ file = "闭包自捕获.jsh"; code = "E117"; stage = "编译" },
        @{ file = "指数.jsh";       code = "E102"; stage = "编译" },
        @{ file = "类嵌套.jsh";     code = "E118"; stage = "编译" },
        @{ file = "类初始化返回.jsh"; code = "E120"; stage = "编译" },
        @{ file = "类默认值本.jsh"; code = "E120"; stage = "编译" },
        @{ file = "类未定义.jsh";   code = "E121"; stage = "编译" },
        @{ file = "类字段拼写.jsh"; code = "E122"; stage = "编译" },
        @{ file = "类未声明字段.jsh"; code = "E122"; stage = "编译" },
        @{ file = "类方法当值.jsh"; code = "E123"; stage = "编译" },
        @{ file = "类运行期字段.jsh"; code = "E035"; stage = "运行" },
        @{ file = "类运行期非实例.jsh"; code = "E036"; stage = "运行" },
        @{ file = "类运行期方法.jsh"; code = "E037"; stage = "运行" },
        @{ file = "未捕获异常.jsh"; code = "E027"; stage = "运行" },
        @{ file = "运行故障不可捕获.jsh"; code = "E007"; stage = "运行" },
        @{ file = "尝试嵌套过深.jsh"; code = "E028"; stage = "运行" },
        @{ file = "任务超限.jsh";   code = "E038"; stage = "运行" },
        @{ file = "任务当值.jsh";   code = "E115"; stage = "编译" },
        @{ file = "PWM引脚.jsh";     code = "E030"; stage = "运行" },
        @{ file = "PWM频率.jsh";     code = "E030"; stage = "运行" },
        @{ file = "PWM占空比越界.jsh"; code = "E030"; stage = "运行" },
        @{ file = "PWM类型.jsh";     code = "E031"; stage = "运行" },
        @{ file = "PWM未启动.jsh";   code = "E033"; stage = "运行" },
        @{ file = "PWM停止未启动.jsh"; code = "E033"; stage = "运行" },
        @{ file = "I2C未初始化.jsh"; code = "E033"; stage = "运行" },
        @{ file = "I2C地址越界.jsh"; code = "E030"; stage = "运行" },
        @{ file = "I2C引脚越界.jsh"; code = "E030"; stage = "运行" },
        @{ file = "I2C速率越界.jsh"; code = "E030"; stage = "运行" },
        @{ file = "I2C数据越界.jsh"; code = "E030"; stage = "运行" },
        @{ file = "I2C类型.jsh";     code = "E031"; stage = "运行" },
        @{ file = "I2C设备未应答.jsh"; code = "E032"; stage = "运行" },
        @{ file = "I2C探测未初始化.jsh"; code = "E033"; stage = "运行" }
    )
    $fail = 0
    foreach ($c in $cases) {
        $src = "tests\errors\" + $c.file
        $jmb = "build\err_" + [IO.Path]::GetFileNameWithoutExtension($c.file) + ".jmb"
        $out = (& $micc $src -o $jmb 2>&1 | Out-String)
        if ($LASTEXITCODE -eq 0) {
            $out += (& $jmrun $jmb 2>&1 | Out-String)
        }
        if ($out -match $c.code) {
            Write-Host ("[通过] " + $c.file + " -> " + $c.code + "（" + $c.stage + "）")
        } else {
            Write-Host ("[不通过] " + $c.file + " —— 没看到 " + $c.code)
            Write-Host $out
            $fail++
        }
    }
    Write-Host ("错误用例：失败 " + $fail + " 个")
    if ($fail -gt 0) { exit 1 }
    exit 0
}
finally {
    Pop-Location
}
