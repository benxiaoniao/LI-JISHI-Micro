//! 标准库模块的唯一事实来源（LI018）。
//!
//! `导入 文本/列表/字典` 才让对应函数名进入作用域；未导入时报 E112，
//! 与原生模块同一套规则。函数本体仍在运行期内建表里，按需裁剪只影响
//! 编译期可见性与名字解析。

pub struct StdModule {
    pub name: &'static str,
    pub fns: &'static [&'static str],
}

pub const MODULES: &[StdModule] = &[
    StdModule {
        name: "文本",
        fns: &[
            "子串", "查找", "替换", "分割", "连接", "去空白", "转大写", "转小写",
        ],
    },
    StdModule {
        name: "列表",
        fns: &["追加", "插入", "移除", "切片", "反转", "排序", "查找"],
    },
    StdModule {
        name: "字典",
        fns: &["键列表", "包含", "合并", "移除键", "默认值"],
    },
];

pub fn module(name: &str) -> Option<&'static StdModule> {
    MODULES.iter().find(|m| m.name == name)
}

/// 这个名字属于哪些标准库模块（`查找` 同时属于文本与列表）。
pub fn modules_for(name: &str) -> Vec<&'static str> {
    MODULES
        .iter()
        .filter(|m| m.fns.contains(&name))
        .map(|m| m.name)
        .collect()
}

pub fn module_names() -> String {
    MODULES
        .iter()
        .map(|m| m.name)
        .collect::<Vec<_>>()
        .join("、")
}
