//! JMB 尾部可选调试段的解析与查询（调试段格式自 v4 起不变）。
//!
//! 解析一律防御式：旧文件没有调试段、`--strip-debug` 剥掉了、或段损坏，
//! 都返回「没有可用调试信息」，绝不阻止字节码运行（运行侧根本不读这段）。

/// 代码表里的一项（调试段之外的基本信息）。
pub struct CodeEntry {
    pub len: u16,
    pub name: String,
    /// 参数个数（代码表 p+5），接口清单校验用。
    pub nparams: u8,
    name_idx: usize,
}

pub struct DebugEntry {
    pub off: u16,
    pub line: u16,
    /// 该条记录所属源文件；多包合并时同一条指令流里可以有多个文件。
    pub file_id: u8,
}

pub struct CodeDebug {
    pub file_id: u8,
    pub entries: Vec<DebugEntry>,
    /// 局部槽符号：`(槽号, 名字)`，按槽号递增；没有符号区时为空。
    pub locals: Vec<(u8, String)>,
}

pub struct DebugInfo {
    pub files: Vec<String>,
    pub codes: Vec<CodeDebug>,
}

/// 类表的一项（字段已按继承扁平化）。
pub struct ClassEntry {
    pub name: String,
    pub fields: Vec<String>,
    /// `(方法名, 代码对象下标)`；不含构造方法。
    pub methods: Vec<(String, u8)>,
    /// 构造方法代码下标。
    pub init_code: u8,
}

pub struct Image {
    pub codes: Vec<CodeEntry>,
    pub classes: Vec<ClassEntry>,
    /// `None` 表示没有调试段、被剥离或已损坏。
    pub debug: Option<DebugInfo>,
}

fn u16_at(b: &[u8], p: usize) -> Option<u16> {
    if p + 2 > b.len() {
        return None;
    }
    Some(u16::from_le_bytes([b[p], b[p + 1]]))
}

fn read_cstr(b: &[u8], start: usize, end: usize) -> Option<String> {
    if start >= end || end > b.len() {
        return None;
    }
    let mut e = start;
    while e < end && b[e] != 0 {
        e += 1;
    }
    if e >= end {
        return None;
    }
    String::from_utf8(b[start..e].to_vec()).ok()
}

/// 解析整份 `.jmb`：先读基本表区，再看尾部调试段。
/// 基本表区不合法返回 `Err(说明)`；调试段不合法只让 `debug` 为 `None`。
pub fn parse(bytes: &[u8]) -> Result<Image, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"JMB1" {
        return Err("不是 JMB 文件（魔数或长度不对）".into());
    }
    let flags = bytes[5];
    let nconsts = bytes[6] as usize;
    let nnames = bytes[7] as usize;
    let ncodes = bytes[8] as usize;

    let const_start = 12usize;
    let name_start = const_start + 8 * nconsts;
    let code_start = name_start + 2 * nnames;
    let cap_start = code_start + 10 * ncodes;
    if cap_start > bytes.len() {
        return Err("JMB 表区越界".into());
    }

    let mut codes = Vec::new();
    let mut max_code_end = 0usize;
    for i in 0..ncodes {
        let p = code_start + 10 * i;
        let off = u16_at(bytes, p).unwrap_or(0);
        let len = u16_at(bytes, p + 2).unwrap_or(0);
        let name_idx = bytes.get(p + 4).copied().unwrap_or(0) as usize;
        let nparams = bytes.get(p + 5).copied().unwrap_or(0);
        max_code_end = max_code_end.max(off as usize + len as usize);
        codes.push(CodeEntry {
            len,
            name: String::new(),
            nparams,
            name_idx,
        });
    }

    // 名字表里存的是**绝对**文件偏移（与加载器一致），直接按偏移取字符串。
    let name_at = |idx: usize| -> Option<String> {
        if idx >= nnames {
            return None;
        }
        let off = u16_at(bytes, name_start + 2 * idx)?;
        read_cstr(bytes, off as usize, bytes.len())
    };
    for c in codes.iter_mut() {
        if let Some(n) = name_at(c.name_idx) {
            c.name = n;
        }
    }

    // 类表：定长 10 B 记录，随后是字段名数组与方法(name,code)数组。
    // 起点要先跳过捕获表（每项 2 B，数量在代码记录 p+8）。
    // v5 起才有 10 B 类记录（带父类下标）；更老的产物不解析类表。
    let nclasses = if bytes.get(4).copied().unwrap_or(0) >= 5 {
        bytes.get(10).copied().unwrap_or(0) as usize
    } else {
        0
    };
    let cap_start = code_start + 10 * ncodes;
    let mut ncap_total = 0usize;
    for i in 0..ncodes {
        ncap_total += bytes.get(code_start + 10 * i + 8).copied().unwrap_or(0) as usize;
    }
    let class_start = cap_start + 2 * ncap_total;
    let mut classes = Vec::new();
    if class_start + 10 * nclasses > bytes.len() {
        return Err("JMB 类表越界".into());
    }
    for i in 0..nclasses {
        let p = class_start + 10 * i;
        let name_idx = bytes[p] as usize;
        let nfields = bytes[p + 1] as usize;
        let nmethods = bytes[p + 2] as usize;
        let init_code = bytes[p + 3];
        let foff = class_start + u16_at(bytes, p + 4).unwrap_or(0) as usize;
        let moff = class_start + u16_at(bytes, p + 6).unwrap_or(0) as usize;
        if foff + nfields > bytes.len() || moff + 2 * nmethods > bytes.len() {
            return Err("JMB 类成员表越界".into());
        }
        let name = name_at(name_idx).ok_or_else(|| "JMB 类名下标越界".to_string())?;
        let mut fields = Vec::new();
        for k in 0..nfields {
            let fi = bytes[foff + k] as usize;
            fields.push(name_at(fi).ok_or_else(|| "JMB 字段名下标越界".to_string())?);
        }
        let mut methods = Vec::new();
        for k in 0..nmethods {
            let ni = bytes[moff + 2 * k] as usize;
            let ci = bytes[moff + 2 * k + 1];
            methods.push((
                name_at(ni).ok_or_else(|| "JMB 方法名下标越界".to_string())?,
                ci,
            ));
        }
        classes.push(ClassEntry {
            name,
            fields,
            methods,
            init_code,
        });
    }

    let debug = if flags & 1 == 0 {
        None
    } else {
        parse_debug(bytes, ncodes, max_code_end)
    };

    Ok(Image {
        codes,
        classes,
        debug,
    })
}

/// 读 varint：<255 单字节，255 后跟 u16。
fn read_var(b: &[u8], mut p: usize, end: usize) -> Option<(u16, usize)> {
    if p >= end {
        return None;
    }
    let v = b[p];
    p += 1;
    if v < 255 {
        return Some((v as u16, p));
    }
    let w = u16_at(b, p)?;
    p += 2;
    if p > end {
        return None;
    }
    Some((w, p))
}

fn zigzag_decode(z: u16) -> i32 {
    ((z >> 1) as i32) ^ -((z & 1) as i32)
}

fn parse_debug(bytes: &[u8], ncodes: usize, max_code_end: usize) -> Option<DebugInfo> {
    if bytes.len() < 6 + 16 {
        return None;
    }
    let tail = bytes.len() - 6;
    if &bytes[tail + 2..tail + 6] != b"DBG1" {
        return None;
    }
    let total_len = u16_at(bytes, tail)? as usize;
    if total_len < 16 || total_len > tail {
        return None;
    }
    let start = tail - total_len;
    if start < max_code_end {
        return None; // 调试段与指令流重叠，按损坏处理
    }
    let section = &bytes[start..tail];
    if &section[0..4] != b"DBG1" || section[4] != 1 {
        return None;
    }
    let nfiles = section[6] as usize;
    let ncode_dbg = section[7] as usize;
    if ncode_dbg != ncodes {
        return None;
    }
    let files_off = u16_at(section, 8)? as usize;
    let codes_off = u16_at(section, 10)? as usize;
    let str_off = u16_at(section, 12)? as usize;
    let per_record_file = section[5] & 2 != 0;
    if files_off + 2 * nfiles > section.len()
        || codes_off + 6 * ncode_dbg > section.len()
        || str_off > section.len()
    {
        return None;
    }

    let mut files = Vec::new();
    for i in 0..nfiles {
        let off = u16_at(section, files_off + 2 * i)? as usize;
        files.push(read_cstr(section, str_off + off, section.len())?);
    }

    let mut codes = Vec::new();
    for i in 0..ncode_dbg {
        let p = codes_off + 6 * i;
        let entries_off = u16_at(section, p)? as usize;
        let nentries = u16_at(section, p + 2)? as usize;
        let file_id = section[p + 4];
        if file_id as usize >= nfiles {
            return None;
        }
        let mut entries = Vec::new();
        let mut pos = start + entries_off;
        let end = start + section.len();
        let mut prev_off = 0u32;
        let mut prev_line = 0u32;
        let mut cur_file = file_id;
        for j in 0..nentries {
            if per_record_file {
                if pos >= end {
                    return None;
                }
                let tag = bytes[pos];
                pos += 1;
                if tag != 0 {
                    cur_file = tag - 1;
                    if cur_file as usize >= nfiles {
                        return None;
                    }
                }
            }
            let (a, np) = read_var(bytes, pos, end)?;
            let (b, np2) = read_var(bytes, np, end)?;
            pos = np2;
            if j == 0 {
                prev_off = a as u32;
                prev_line = b as u32;
            } else {
                prev_off += a as u32;
                let d = zigzag_decode(b);
                prev_line = (prev_line as i32 + d).max(0) as u32;
            }
            if prev_off > 65535 || prev_line > 65535 {
                return None;
            }
            entries.push(DebugEntry {
                off: prev_off as u16,
                line: prev_line as u16,
                file_id: cur_file,
            });
        }
        codes.push(CodeDebug {
            file_id,
            entries,
            locals: Vec::new(),
        });
    }

    // 符号区（flags.bit2）：目录 → 各代码块 → 名字池，偏移都相对 sym_off。
    if section[5] & 4 != 0 {
        let sym_off = u16_at(section, 14)? as usize;
        if sym_off == 0 || sym_off + 2 * ncode_dbg > section.len() {
            return None;
        }
        for (i, c) in codes.iter_mut().enumerate() {
            let entry_off = u16_at(section, sym_off + 2 * i)? as usize;
            if entry_off == 0 {
                continue;
            }
            let mut pos = sym_off + entry_off;
            if pos >= section.len() {
                return None;
            }
            let n = section[pos] as usize;
            pos += 1;
            for _ in 0..n {
                if pos + 3 > section.len() {
                    return None;
                }
                let slot = section[pos];
                let name_off = u16_at(section, pos + 1)? as usize;
                pos += 3;
                let name = read_cstr(section, sym_off + name_off, section.len())?;
                c.locals.push((slot, name));
            }
        }
    }

    Some(DebugInfo { files, codes })
}

impl Image {
    /// 正向查询：代码对象 + 指令偏移 → 文件名与行号。
    pub fn line_at(&self, code: usize, off: u16) -> Option<(&str, u16)> {
        let d = self.debug.as_ref()?;
        let c = d.codes.get(code)?;
        let idx = c.entries.partition_point(|e| e.off <= off);
        if idx == 0 {
            return None;
        }
        let e = &c.entries[idx - 1];
        Some((d.files.get(e.file_id as usize)?.as_str(), e.line))
    }

    /// 代码对象的局部槽符号；没有符号区时返回 `None`。
    pub fn locals_at(&self, code: usize) -> Option<&[(u8, String)]> {
        let d = self.debug.as_ref()?;
        let c = d.codes.get(code)?;
        if c.locals.is_empty() {
            return None;
        }
        Some(&c.locals)
    }

    pub fn file_id(&self, name: &str) -> Option<u8> {
        let d = self.debug.as_ref()?;
        let norm = norm_debug_path(name);
        // 调试表里存的是编译期 `canonicalize` 后的路径（可能相对 cwd）；macOS 的
        // /var 是指向 /private/var 的符号链接，Windows 也可能走 junction，
        // 所以两边都规范化成 canonical 再比，找不到文件时退回字符串比较。
        let canon = canonical_key(name);
        // 绝对路径的尾部匹配要去掉开头 `/`，否则 `/private/var/x` 对 `/var/x`
        // 会拼成 `//var/x` 而匹配不上（macOS 的 /var 正好是软链）。
        let tail = norm.trim_start_matches('/');
        d.files
            .iter()
            .position(|f| {
                let nf = norm_debug_path(f);
                nf == norm
                    || (canon.is_some() && canon == canonical_key(f))
                    || nf.ends_with(&format!("/{}", tail))
                    || norm.ends_with(&format!("/{}", nf))
            })
            .map(|i| i as u8)
    }
}

/// 调试表路径比较用：统一用 `/`，去掉 Windows 的 `//?/` 前缀。
fn norm_debug_path(s: &str) -> String {
    let s = s.replace('\\', "/");
    s.strip_prefix("//?/").unwrap_or(&s).to_string()
}

/// 路径的 canonical 形式（解析符号链接 / junction）；文件不存在时返回 `None`。
fn canonical_key(p: &str) -> Option<String> {
    std::fs::canonicalize(p)
        .ok()
        .map(|q| norm_debug_path(&q.to_string_lossy()))
}

/// 某一行的指令范围（用于反查）。
pub struct LineSpan {
    pub code: usize,
    pub start: u16,
    pub end: u16,
}

/// 反查：文件（`None` = 所有文件）+ 行号 → 各代码对象里的指令范围。
pub fn spans_of_line(image: &Image, file_id: Option<u8>, line: u16) -> Vec<LineSpan> {
    let mut out = Vec::new();
    let Some(d) = image.debug.as_ref() else {
        return out;
    };
    for (ci, c) in d.codes.iter().enumerate() {
        let Some(ce) = image.codes.get(ci) else {
            continue;
        };
        for (i, e) in c.entries.iter().enumerate() {
            if e.line != line {
                continue;
            }
            if let Some(f) = file_id {
                if e.file_id != f {
                    continue;
                }
            }
            let end = c
                .entries
                .get(i + 1)
                .map(|n| n.off)
                .unwrap_or(ce.len)
                .max(e.off);
            out.push(LineSpan {
                code: ci,
                start: e.off,
                end,
            });
        }
    }
    out
}

/// 生成给 H3/调试器用的文本报告。
pub fn dump(image: &Image) -> String {
    let mut s = String::new();
    match &image.debug {
        None => s.push_str("没有可用调试信息（旧文件、已剥离或调试段损坏）\n"),
        Some(d) => {
            s.push_str("源文件：\n");
            for (i, f) in d.files.iter().enumerate() {
                s.push_str(&format!("  [{}] {}\n", i, f));
            }
            for (ci, c) in d.codes.iter().enumerate() {
                let name = image
                    .codes
                    .get(ci)
                    .map(|x| x.name.as_str())
                    .unwrap_or("?");
                s.push_str(&format!(
                    "代码 {} 「{}」 文件 {} 行记录 {}\n",
                    ci,
                    name,
                    c.file_id,
                    c.entries.len()
                ));
                if !c.locals.is_empty() {
                    let list = c
                        .locals
                        .iter()
                        .map(|(slot, name)| format!("{}={}", slot, name))
                        .collect::<Vec<_>>()
                        .join(" ");
                    s.push_str(&format!("  局部: {}\n", list));
                }
                for e in &c.entries {
                    if d.files.len() > 1 {
                        s.push_str(&format!(
                            "  {:04}  文件 {} 行 {}\n",
                            e.off, e.file_id, e.line
                        ));
                    } else {
                        s.push_str(&format!("  {:04}  行 {}\n", e.off, e.line));
                    }
                }
            }
        }
    }
    s
}
