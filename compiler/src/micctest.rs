//! `micc test`：把 `*.jsh` 当用例跑，零第三方依赖。
//!
//! 用例就是普通砾程序，用 `断言` 表达检查；文件顶部的注释声明期望：
//! `# @期望输出: 一行`（可重复）、`# @期望错误: E027`、`# @超时: 2000`、`# @名称: 别名`。
//! 编译失败时也接受 `@期望错误`（例如 E109），这样正/负用例可以放在同一套里。

use crate::error::{CompileError, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Opts {
    dir: PathBuf,
    timeout_ms: u64,
    jmrun: PathBuf,
    list: bool,
    filters: Vec<String>,
}

struct Case {
    path: PathBuf,
    name: String,
    expected_output: Option<String>,
    expected_error: Option<String>,
    timeout_ms: Option<u64>,
}

enum Outcome {
    Pass,
    Fail(String),
    Timeout,
}

pub fn cmd_test(args: &[String]) -> i32 {
    match run(args) {
        Ok(0) => 0,
        Ok(_) => 1,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

fn e125(msg: impl Into<String>) -> CompileError {
    CompileError::new("E125", 1, 1, msg)
}

fn parse_opts(args: &[String]) -> Result<Opts> {
    let usage =
        "用法：micc test [过滤...] [--目录 <用例目录>] [--超时 <毫秒>] [--jmrun <路径>] [--列出]";
    let mut dir = PathBuf::from("tests/cases");
    let mut timeout_ms = 5000u64;
    let mut jmrun = default_jmrun();
    let mut list = false;
    let mut filters = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--目录" | "--dir" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--目录 后面要跟用例目录"));
                }
                dir = PathBuf::from(&args[i]);
            }
            "--超时" | "--timeout" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--超时 后面要跟毫秒数"));
                }
                timeout_ms = args[i]
                    .parse()
                    .map_err(|_| e125(format!("--超时 不是正整数：{}", args[i])))?;
                if timeout_ms == 0 {
                    return Err(e125("--超时 必须大于 0"));
                }
            }
            "--jmrun" => {
                i += 1;
                if i >= args.len() {
                    return Err(e125("--jmrun 后面要跟可执行文件"));
                }
                jmrun = PathBuf::from(&args[i]);
            }
            "--列出" | "--list" => list = true,
            "-h" | "--help" => return Err(e125(usage)),
            other if other.starts_with('-') => {
                return Err(e125(format!("不认识的参数：{}；{}", other, usage)));
            }
            other => filters.push(other.to_string()),
        }
        i += 1;
    }
    if !dir.is_dir() {
        return Err(e125(format!(
            "用例目录不存在：{}（用 --目录 指定，默认 tests/cases）",
            dir.display()
        ))
        .with_hint("每个 .jsh 就是一个用例；顶部用 # @期望输出: ... / # @期望错误: E027 / # @超时: 3000 声明期望"));
    }
    if !jmrun.is_file() {
        return Err(e125(format!(
            "找不到 jmrun：{}（先运行 tools/build.ps1，或用 --jmrun 指定）",
            jmrun.display()
        )));
    }
    Ok(Opts {
        dir,
        timeout_ms,
        jmrun,
        list,
        filters,
    })
}

fn default_jmrun() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("build/jmrun.exe")
    } else {
        PathBuf::from("build/jmrun")
    }
}

fn run(args: &[String]) -> Result<usize> {
    let opts = parse_opts(args)?;
    let mut files = Vec::new();
    collect_cases(&opts.dir, &mut files)?;
    files.sort();

    let micc = std::env::current_exe().map_err(|e| e125(format!("取当前 micc 路径失败：{}", e)))?;
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut timedout = 0usize;

    for path in files {
        let case = match Case::load(&path) {
            Ok(case) => case,
            Err(e) => {
                println!("[不通过] {}（用例格式错误）", display(&opts.dir, &path));
                println!("{}", e);
                failed += 1;
                total += 1;
                continue;
            }
        };
        if !opts.filters.is_empty() && !matches(&case, &opts.dir, &opts.filters) {
            continue;
        }
        total += 1;
        if opts.list {
            println!("{}\t{}", case.name, display(&opts.dir, &case.path));
            continue;
        }
        match run_case(&micc, &opts.jmrun, &case, opts.timeout_ms) {
            Outcome::Pass => {
                println!("[通过] {}", case.name);
                passed += 1;
            }
            Outcome::Fail(detail) => {
                println!("[不通过] {}", case.name);
                if !detail.is_empty() {
                    println!("{}", detail);
                }
                failed += 1;
            }
            Outcome::Timeout => {
                println!(
                    "[超时] {}（超过 {} ms）",
                    case.name,
                    case.timeout_ms.unwrap_or(opts.timeout_ms)
                );
                timedout += 1;
            }
        }
    }

    if opts.list {
        return Ok(0);
    }
    if total == 0 {
        println!("（没有匹配的用例）");
        return Ok(1);
    }
    println!(
        "micc test：{} 个用例，通过 {}，失败 {}，超时 {}",
        total, passed, failed, timedout
    );
    Ok(failed + timedout)
}

impl Case {
    fn load(path: &Path) -> Result<Case> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| e125(format!("读不了 {}：{}", path.display(), e)))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let mut name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "未命名".to_string());
        let mut expected_lines: Vec<String> = Vec::new();
        let mut has_output = false;
        let mut expected_error: Option<String> = None;
        let mut timeout_ms: Option<u64> = None;

        for (idx, raw) in text.lines().enumerate() {
            let line = raw.trim();
            let Some(rest) = line.strip_prefix('#') else {
                continue;
            };
            let directive = rest.trim();
            if let Some(value) = directive.strip_prefix("@期望输出:") {
                has_output = true;
                expected_lines.push(value.strip_prefix(' ').unwrap_or(value).to_string());
            } else if let Some(value) = directive.strip_prefix("@期望错误:") {
                expected_error = Some(normalize_code(value.trim()).ok_or_else(|| {
                    e125(format!(
                        "{} 第 {} 行的 @期望错误 要写成 E027 这样的错误码",
                        path.display(),
                        idx + 1
                    ))
                })?);
            } else if let Some(value) = directive.strip_prefix("@超时:") {
                let ms: u64 = value.trim().parse().map_err(|_| {
                    e125(format!(
                        "{} 第 {} 行的 @超时 不是正整数：{}",
                        path.display(),
                        idx + 1,
                        value.trim()
                    ))
                })?;
                if ms == 0 {
                    return Err(e125(format!(
                        "{} 第 {} 行的 @超时 必须大于 0",
                        path.display(),
                        idx + 1
                    )));
                }
                timeout_ms = Some(ms);
            } else if let Some(value) = directive.strip_prefix("@名称:") {
                let value = value.trim();
                if value.is_empty() {
                    return Err(e125(format!(
                        "{} 第 {} 行的 @名称 不能为空",
                        path.display(),
                        idx + 1
                    )));
                }
                name = value.to_string();
            }
        }

        Ok(Case {
            path: path.to_path_buf(),
            name,
            expected_output: if has_output {
                Some(expected_lines.join("\n"))
            } else {
                None
            },
            expected_error,
            timeout_ms,
        })
    }
}

fn normalize_code(text: &str) -> Option<String> {
    let text = text
        .strip_prefix("错误[")
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(text)
        .trim();
    let digits = text.strip_prefix('E')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("E{}", digits))
}

fn collect_cases(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e125(format!("读不了用例目录 {}：{}", dir.display(), e)))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_cases(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsh") {
            out.push(path);
        }
    }
    Ok(())
}

fn display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn matches(case: &Case, root: &Path, filters: &[String]) -> bool {
    let name = case.name.to_lowercase();
    let rel = display(root, &case.path).to_lowercase();
    filters.iter().any(|filter| {
        let f = filter.to_lowercase();
        name.contains(&f) || rel.contains(&f)
    })
}

fn run_case(micc: &Path, jmrun: &Path, case: &Case, default_timeout: u64) -> Outcome {
    let jmb = temp_jmb();
    let compiled = Command::new(micc)
        .arg(&case.path)
        .arg("-o")
        .arg(&jmb)
        .output();
    let compiled = match compiled {
        Ok(output) => output,
        Err(e) => {
            let _ = std::fs::remove_file(&jmb);
            return Outcome::Fail(format!("无法启动 micc：{}", e));
        }
    };
    let compile_err = norm(&compiled.stderr);
    if !compiled.status.success() {
        let _ = std::fs::remove_file(&jmb);
        if let Some(code) = &case.expected_error {
            if compile_err.contains(&format!("错误[{}]", code)) {
                return Outcome::Pass;
            }
            return Outcome::Fail(format!(
                "期望编译错误 {}，实际不是：\n{}",
                code, compile_err
            ));
        }
        return Outcome::Fail(format!("编译失败：\n{}", compile_err));
    }

    let timeout = case.timeout_ms.unwrap_or(default_timeout);
    let outcome = run_jmb(jmrun, &jmb, timeout);
    let _ = std::fs::remove_file(&jmb);
    let (status_code, stdout, stderr) = match outcome {
        Exec::Timeout => return Outcome::Timeout,
        Exec::Error(message) => return Outcome::Fail(message),
        Exec::Done {
            code,
            stdout,
            stderr,
        } => (code, stdout, stderr),
    };
    let stdout = norm(&stdout);
    let stderr = norm(&stderr);

    if let Some(code) = &case.expected_error {
        if status_code == Some(0) {
            return Outcome::Fail(format!("期望错误 {}，但程序正常结束", code));
        }
        if !stderr.contains(&format!("错误[{}]", code)) {
            return Outcome::Fail(format!("期望错误 {}，实际输出：\n{}", code, stderr));
        }
    } else if status_code != Some(0) {
        return Outcome::Fail(format!("退出码 {:?}（非 0）：\n{}", status_code, stderr));
    }

    if let Some(expected) = &case.expected_output {
        let expected = expected.replace("\r\n", "\n");
        let expected = expected.trim_end_matches('\n');
        if stdout.trim_end_matches('\n') != expected {
            return Outcome::Fail(format!(
                "输出不一致\n--- 期望 ---\n{}\n--- 实际 ---\n{}",
                expected, stdout
            ));
        }
    }
    Outcome::Pass
}

/// 子进程执行结果；超时与启动失败要分开报。
enum Exec {
    Done {
        code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    Timeout,
    Error(String),
}

fn run_jmb(jmrun: &Path, jmb: &Path, timeout_ms: u64) -> Exec {
    let mut child = match Command::new(jmrun)
        .arg(jmb)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return Exec::Error(format!("无法启动 jmrun：{}", e)),
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Exec::Timeout;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Exec::Error(format!("等待 jmrun 失败：{}", e));
            }
        }
    }
    match child.wait_with_output() {
        Ok(output) => Exec::Done {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        },
        Err(e) => Exec::Error(format!("读取 jmrun 输出失败：{}", e)),
    }
}

fn temp_jmb() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("li-micctest-{}-{}.jmb", std::process::id(), nanos))
}

fn norm(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string()
}
