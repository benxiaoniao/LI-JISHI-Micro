//! 最小 ZIP 读写（只支持 stored，不压缩、不加密、无数据描述符）。
//!
//! 只服务 H4b 的包归档：`micc pack --归档` 写，`micc verify` 读。
//! 不引入第三方依赖；条目名按 UTF-8 写（通用标志位 bit11）。
//! 不支持的能力（压缩、ZIP64、多卷、注释）一律明确报错，不静默降级。

pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

const SIG_LOCAL: u32 = 0x0403_4b50;
const SIG_CENTRAL: u32 = 0x0201_4b50;
const SIG_EOCD: u32 = 0x0605_4b50;
/// 通用标志位 bit11：条目名是 UTF-8。
const FLAG_UTF8: u16 = 0x0800;

const fn make_crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const CRC_TABLE: [u32; 256] = make_crc_table();

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn u16_at(b: &[u8], p: usize) -> Option<u16> {
    if p + 2 > b.len() {
        return None;
    }
    Some(u16::from_le_bytes([b[p], b[p + 1]]))
}

fn u32_at(b: &[u8], p: usize) -> Option<u32> {
    if p + 4 > b.len() {
        return None;
    }
    Some(u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]))
}

/// 打包成 ZIP 字节流（stored）。条目顺序即写入顺序。
pub fn write(entries: &[Entry]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();

    for e in entries {
        let name = e.name.as_bytes();
        let crc = crc32(&e.data);
        let size = e.data.len() as u32;
        let off = out.len() as u32;

        out.extend_from_slice(&SIG_LOCAL.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // 解压所需版本
        out.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // 方式 0 = stored
        out.extend_from_slice(&0u16.to_le_bytes()); // 时间
        out.extend_from_slice(&0u16.to_le_bytes()); // 日期
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // 扩展字段
        out.extend_from_slice(name);
        out.extend_from_slice(&e.data);

        central.extend_from_slice(&SIG_CENTRAL.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // 制作者版本
        central.extend_from_slice(&20u16.to_le_bytes()); // 解压所需版本
        central.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // 扩展字段
        central.extend_from_slice(&0u16.to_le_bytes()); // 注释
        central.extend_from_slice(&0u16.to_le_bytes()); // 起始盘
        central.extend_from_slice(&0u16.to_le_bytes()); // 内部属性
        central.extend_from_slice(&0u32.to_le_bytes()); // 外部属性
        central.extend_from_slice(&off.to_le_bytes());
        central.extend_from_slice(name);
    }

    let cd_off = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);

    out.extend_from_slice(&SIG_EOCD.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // 本盘号
    out.extend_from_slice(&0u16.to_le_bytes()); // 中央目录起始盘
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // 注释长度
    out
}

/// 读取 ZIP（只认 stored）。任何不支持的结构都返回可读的错误说明。
pub fn read(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    if bytes.len() < 22 {
        return Err("不是 ZIP 归档（文件太短）".to_string());
    }
    // 从尾部往前找 EOCD（注释长度必须为 0，否则视为不支持）。
    let mut eocd = None;
    let lo = bytes.len().saturating_sub(22 + 65535);
    let mut p = bytes.len() - 22;
    loop {
        if u32_at(bytes, p) == Some(SIG_EOCD) && u16_at(bytes, p + 20) == Some(0) {
            eocd = Some(p);
            break;
        }
        if p == lo {
            break;
        }
        p -= 1;
    }
    let Some(eocd) = eocd else {
        return Err("不是 ZIP 归档（找不到中央目录结尾）".to_string());
    };

    let count = u16_at(bytes, eocd + 10).ok_or("ZIP 中央目录记录数缺失")? as usize;
    let cd_off = u32_at(bytes, eocd + 16).ok_or("ZIP 中央目录偏移缺失")? as usize;

    let mut entries = Vec::new();
    let mut p = cd_off;
    for _ in 0..count {
        if u32_at(bytes, p) != Some(SIG_CENTRAL) {
            return Err("ZIP 中央目录损坏".to_string());
        }
        let method = u16_at(bytes, p + 10).ok_or("ZIP 条目缺压缩方式")?;
        let csize = u32_at(bytes, p + 20).ok_or("ZIP 条目缺压缩后大小")? as usize;
        let nlen = u16_at(bytes, p + 28).ok_or("ZIP 条目缺名字长度")? as usize;
        let elen = u16_at(bytes, p + 30).ok_or("ZIP 条目缺扩展字段长度")? as usize;
        let clen = u16_at(bytes, p + 32).ok_or("ZIP 条目缺注释长度")? as usize;
        let lho = u32_at(bytes, p + 42).ok_or("ZIP 条目缺本地头偏移")? as usize;
        if p + 46 + nlen > bytes.len() {
            return Err("ZIP 中央目录越界".to_string());
        }
        let name = String::from_utf8(bytes[p + 46..p + 46 + nlen].to_vec())
            .map_err(|_| "ZIP 条目名不是 UTF-8".to_string())?;
        if method != 0 {
            return Err(format!(
                "条目「{}」使用了压缩（方式 {}），本工具只支持 stored 归档；请用 `micc pack --归档` 生成",
                name, method
            ));
        }
        // 本地头：名字长度与扩展字段长度可能与中央目录不同，单独读。
        if u32_at(bytes, lho) != Some(SIG_LOCAL) {
            return Err(format!("条目「{}」的本地头损坏", name));
        }
        let lnlen = u16_at(bytes, lho + 26).ok_or("ZIP 本地头缺名字长度")? as usize;
        let lelen = u16_at(bytes, lho + 28).ok_or("ZIP 本地头缺扩展字段长度")? as usize;
        let start = lho + 30 + lnlen + lelen;
        if start + csize > bytes.len() {
            return Err(format!("条目「{}」的数据越界", name));
        }
        entries.push(Entry {
            name,
            data: bytes[start..start + csize].to_vec(),
        });
        p += 46 + nlen + elen + clen;
    }
    Ok(entries)
}
