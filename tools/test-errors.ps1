$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "toolchains.ps1")

Push-Location $root
try {
    $micc = Join-Path $root ("compiler/target/release/micc" + $ExeSuffix)
    $jmrun = Join-Path $root ("build/jmrun" + $ExeSuffix)
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
        @{ file = "I2C探测未初始化.jsh"; code = "E033"; stage = "运行" },
        @{ file = "枚举局部重名.jsh"; code = "E139"; stage = "编译" },
        @{ file = "枚举参数重名.jsh"; code = "E139"; stage = "编译" },
        @{ file = "枚举内建重名.jsh"; code = "E139"; stage = "编译" },
        @{ file = "枚举原生重名.jsh"; code = "E139"; stage = "编译" },
        @{ file = "集合元素非法.jsh"; code = "E019"; stage = "运行" },
        @{ file = "集合参数个数.jsh"; code = "E012"; stage = "运行" },
        @{ file = "字节列表越界.jsh"; code = "E010"; stage = "运行" },
        @{ file = "字节下标越界.jsh"; code = "E014"; stage = "运行" },
        @{ file = "十六进制位数.jsh"; code = "E010"; stage = "运行" },
        @{ file = "字节参数类型.jsh"; code = "E010"; stage = "运行" },
        @{ file = "位运算小数.jsh"; code = "E010"; stage = "运行" },
        @{ file = "移位次数.jsh"; code = "E010"; stage = "运行" },
        @{ file = "左移溢出.jsh"; code = "E009"; stage = "运行" },
        @{ file = "断言缺条件.jsh"; code = "E109"; stage = "编译" },
        @{ file = "匿名函数捕获赋值.jsh"; code = "E117"; stage = "编译" },
        @{ file = "匿名函数参数枚举.jsh"; code = "E139"; stage = "编译" },
        @{ file = "匿名函数实参个数.jsh"; code = "E012"; stage = "运行" },
        @{ file = "匹配区间重叠.jsh"; code = "E139"; stage = "编译" },
        @{ file = "匹配表达式缺兜底.jsh"; code = "E139"; stage = "编译" },
        @{ file = "匹配表达式类型不一致.jsh"; code = "E139"; stage = "编译" },
        @{ file = "枚举取值重复.jsh"; code = "E139"; stage = "编译" },
        @{ file = "枚举自动取值重复.jsh"; code = "E139"; stage = "编译" },
        @{ file = "枚举取值越界.jsh"; code = "E110"; stage = "编译" },
        @{ file = "文本插值缺右括号.jsh"; code = "E139"; stage = "编译" },
        @{ file = "文本插值空.jsh"; code = "E139"; stage = "编译" },
        @{ file = "文本插值表达式错.jsh"; code = "E109"; stage = "编译" },
        @{ file = "标准库未导入.jsh"; code = "E112"; stage = "编译" },
        @{ file = "文本库参数类型.jsh"; code = "E010"; stage = "运行" },
        @{ file = "文本库下标类型.jsh"; code = "E013"; stage = "运行" },
        @{ file = "连接元素类型.jsh"; code = "E010"; stage = "运行" },
        @{ file = "追加参数类型.jsh"; code = "E010"; stage = "运行" },
        @{ file = "移除下标越界.jsh"; code = "E014"; stage = "运行" },
        @{ file = "切片下标类型.jsh"; code = "E013"; stage = "运行" },
        @{ file = "排序混类型.jsh"; code = "E010"; stage = "运行" },
        @{ file = "键列表参数类型.jsh"; code = "E010"; stage = "运行" },
        @{ file = "合并参数类型.jsh"; code = "E010"; stage = "运行" },
        @{ file = "默认值参数个数.jsh"; code = "E012"; stage = "运行" },
        @{ file = "继承父未定义.jsh"; code = "E140"; stage = "编译" },
        @{ file = "继承自己.jsh"; code = "E140"; stage = "编译" },
        @{ file = "继承成环.jsh"; code = "E140"; stage = "编译" },
        @{ file = "继承重定义字段.jsh"; code = "E140"; stage = "编译" },
        @{ file = "继承父构造带参.jsh"; code = "E140"; stage = "编译" },
        @{ file = "超在模块.jsh"; code = "E141"; stage = "编译" },
        @{ file = "超无父类.jsh"; code = "E141"; stage = "编译" },
        @{ file = "超未知方法.jsh"; code = "E141"; stage = "编译" },
        @{ file = "超初始化.jsh"; code = "E141"; stage = "编译" },
        @{ file = "超当值.jsh"; code = "E141"; stage = "编译" },
        @{ file = "超漏括号.jsh"; code = "E141"; stage = "编译" },
        @{ file = "最终类被继承.jsh"; code = "E142"; stage = "编译" },
        @{ file = "最终方法被覆写.jsh"; code = "E142"; stage = "编译" },
        @{ file = "最终初始化.jsh"; code = "E142"; stage = "编译" },
        @{ file = "最终顶层函数.jsh"; code = "E142"; stage = "编译" },
        @{ file = "最终修饰字段.jsh"; code = "E142"; stage = "编译" },
        @{ file = "最终当值.jsh"; code = "E142"; stage = "编译" },
        @{ file = "私有位置.jsh"; code = "E144"; stage = "编译" },
        @{ file = "私有重复.jsh"; code = "E144"; stage = "编译" },
        @{ file = "私有当值.jsh"; code = "E144"; stage = "编译" },
        @{ file = "用.jsh"; code = "E103"; stage = "编译" }
    )
    $fail = 0
    foreach ($c in $cases) {
        $src = "tests/errors/" + $c.file
        $jmb = "build/err_" + [IO.Path]::GetFileNameWithoutExtension($c.file) + ".jmb"
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
