//! `micc dap`：Debug Adapter Protocol over stdio。
//!
//! 只做协议翻译：前端是 VS Code 的调试 UI，后端是 `jmrun --dbg --事件流`。
//! 断点/条件由 `--break` 传入，暂停/结束读 NDJSON；`bt`/`v` 另读适配器约定的
//! `栈`/`变量` 事件。不引第三方 crate，传输复用 `lsp.rs` 的 Content-Length 读写。

use crate::json::{self, obj, s, Json};
use crate::lsp::{read_message, write_message};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const THREAD_ID: i64 = 1;
const MAX_BREAKS: usize = 8;
const REPLY_TIMEOUT_MS: u64 = 10000;

pub fn run() -> i32 {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut adapter = Adapter::new();
    loop {
        let Some(body) = read_message(&mut reader) else {
            adapter.shutdown();
            return 0;
        };
        let Ok(msg) = json::parse(&body) else {
            continue;
        };
        if msg.string("type").as_deref() != Some("request") {
            continue;
        }
        adapter.handle(&msg);
        if adapter.done {
            adapter.shutdown();
            return 0;
        }
    }
}

/* ---------------- 输出 ---------------- */

#[derive(Clone)]
struct Emitter {
    seq: Arc<AtomicI64>,
    out: Arc<Mutex<()>>,
}

impl Emitter {
    fn send(&self, fields: Vec<(&str, Json)>) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let mut all: Vec<(String, Json)> = Vec::with_capacity(fields.len() + 1);
        all.push(("seq".to_string(), json::n(seq as f64)));
        for (key, value) in fields {
            all.push((key.to_string(), value));
        }
        let _guard = self.out.lock().unwrap();
        write_message(&mut std::io::stdout().lock(), &Json::Obj(all));
    }

    fn respond(&self, request: &Json, body: Option<Json>) {
        let request_seq = request.get("seq").and_then(Json::as_i64).unwrap_or(0);
        let command = request.string("command").unwrap_or_default();
        let mut fields = vec![
            ("type", s("response")),
            ("request_seq", json::n(request_seq as f64)),
            ("success", Json::Bool(true)),
            ("command", s(command)),
        ];
        if let Some(body) = body {
            fields.push(("body", body));
        }
        self.send(fields);
    }

    fn fail(&self, request: &Json, message: &str) {
        let request_seq = request.get("seq").and_then(Json::as_i64).unwrap_or(0);
        let command = request.string("command").unwrap_or_default();
        self.send(vec![
            ("type", s("response")),
            ("request_seq", json::n(request_seq as f64)),
            ("success", Json::Bool(false)),
            ("command", s(command)),
            ("message", s(message)),
            (
                "body",
                obj(vec![(
                    "error",
                    obj(vec![("id", json::n(1.0)), ("format", s(message))]),
                )]),
            ),
        ]);
    }

    fn event(&self, name: &str, body: Json) {
        self.send(vec![
            ("type", s("event")),
            ("event", s(name)),
            ("body", body),
        ]);
    }

    fn output(&self, category: &str, text: &str) {
        self.event(
            "output",
            obj(vec![("category", s(category)), ("output", s(text))]),
        );
    }
}

/* ---------------- 会话状态 ---------------- */

struct State {
    paused: bool,
    ended: bool,
    task: i64,
}

#[derive(Clone)]
struct Break {
    file: String,
    line: i64,
    condition: String,
}

#[derive(Clone)]
struct Launch {
    program: PathBuf,
    cwd: PathBuf,
    jmrun: PathBuf,
    task: Option<i64>,
    cleanup: bool,
}

struct Adapter {
    emitter: Emitter,
    state: Arc<Mutex<State>>,
    replies: Arc<(Mutex<Vec<Json>>, Condvar)>,
    client_breaks: HashMap<String, Vec<Break>>,
    launch_breaks: Vec<Break>,
    launch: Option<Launch>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    done: bool,
}

impl Adapter {
    fn new() -> Self {
        Adapter {
            emitter: Emitter {
                seq: Arc::new(AtomicI64::new(1)),
                out: Arc::new(Mutex::new(())),
            },
            state: Arc::new(Mutex::new(State {
                paused: false,
                ended: false,
                task: 0,
            })),
            replies: Arc::new((Mutex::new(Vec::new()), Condvar::new())),
            client_breaks: HashMap::new(),
            launch_breaks: Vec::new(),
            launch: None,
            child: None,
            stdin: None,
            done: false,
        }
    }

    fn handle(&mut self, request: &Json) {
        let command = request.string("command").unwrap_or_default();
        let args = request.get("arguments").cloned().unwrap_or(Json::Null);
        match command.as_str() {
            "initialize" => {
                // DAP 的 initialize 响应 body 就是能力对象本身（不像 LSP 再包一层
                // capabilities）；VS Code 会直接把 body 合并进 capabilities。
                self.emitter.respond(
                    request,
                    Some(obj(vec![
                        ("supportsConfigurationDoneRequest", Json::Bool(true)),
                        ("supportsConditionalBreakpoints", Json::Bool(true)),
                        ("supportsHitConditionalBreakpoints", Json::Bool(true)),
                        ("supportsTerminateRequest", Json::Bool(true)),
                        ("supportsEvaluateForHovers", Json::Bool(false)),
                        ("supportsSetVariable", Json::Bool(false)),
                        ("supportsRestartRequest", Json::Bool(false)),
                        ("supportsStepBack", Json::Bool(false)),
                        ("exceptionBreakpointFilters", Json::Arr(vec![])),
                    ])),
                );
            }
            "setBreakpoints" => self.set_breakpoints(request, &args),
            "launch" => self.launch_request(request, &args),
            "configurationDone" => {
                self.emitter.respond(request, Some(obj(vec![])));
                self.start();
            }
            "threads" => {
                self.emitter.respond(
                    request,
                    Some(obj(vec![(
                        "threads",
                        Json::Arr(vec![obj(vec![
                            ("id", json::n(THREAD_ID as f64)),
                            ("name", s("砾调试")),
                        ])]),
                    )])),
                );
            }
            "stackTrace" => self.stack_trace(request),
            "scopes" => self.scopes(request),
            "variables" => self.variables(request, &args),
            "continue" => {
                self.command("c");
                self.emitter.respond(
                    request,
                    Some(obj(vec![("allThreadsContinued", Json::Bool(true))])),
                );
            }
            "next" => {
                self.command("n");
                self.emitter.respond(request, Some(obj(vec![])));
            }
            "stepIn" => {
                self.command("s");
                self.emitter.respond(request, Some(obj(vec![])));
            }
            "stepOut" => {
                self.command("o");
                self.emitter.respond(request, Some(obj(vec![])));
            }
            "pause" => {
                self.emitter.fail(request, "后端没有异步暂停入口");
            }
            "evaluate" => {
                self.emitter.fail(request, "暂不支持表达式求值");
            }
            "setExceptionBreakpoints" => {
                self.emitter
                    .respond(request, Some(obj(vec![("breakpoints", Json::Arr(vec![]))])));
            }
            "disconnect" | "terminate" => self.stop(request),
            other => self.emitter.fail(request, &format!("不支持：{}", other)),
        }
    }

    fn set_breakpoints(&mut self, request: &Json, args: &Json) {
        let path = args
            .get("source")
            .and_then(|source| source.string("path"))
            .unwrap_or_default();
        let mut list = Vec::new();
        let mut result = Vec::new();
        let items = args
            .get("breakpoints")
            .and_then(Json::as_arr)
            .unwrap_or(&[]);
        for (index, item) in items.iter().enumerate() {
            let line = item.get("line").and_then(Json::as_i64).unwrap_or(0);
            if line <= 0 {
                continue;
            }
            list.push(Break {
                file: path.clone(),
                line,
                condition: item
                    .string("condition")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            });
            let verified = index < MAX_BREAKS;
            let mut fields = vec![
                ("verified", Json::Bool(verified)),
                ("line", json::n(line as f64)),
                ("source", obj(vec![("path", s(path.clone()))])),
            ];
            if !verified {
                fields.push(("message", s("断点最多 8 个（jmrun 上限）")));
            }
            result.push(obj(fields));
        }
        if !path.is_empty() {
            self.client_breaks.insert(path, list);
        }
        let mut body = vec![("breakpoints", Json::Arr(result))];
        if self.child.is_some() {
            body.push(("message", s("运行中修改断点将在下次会话生效")));
        }
        self.emitter.respond(request, Some(obj(body)));
    }

    fn launch_request(&mut self, request: &Json, args: &Json) {
        let Some(program) = args.string("program").filter(|p| !p.is_empty()) else {
            self.emitter.fail(request, "launch 缺少 program");
            return;
        };
        let cwd = args
            .string("cwd")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let program_path = resolve(&cwd, &program);
        if !program_path.is_file() {
            self.emitter
                .fail(request, &format!("找不到程序：{}", program_path.display()));
            return;
        }
        let micc = args
            .string("miccPath")
            .map(|p| resolve(&cwd, &p))
            .or_else(|| find_tool(&cwd, MICC_CANDIDATES));
        let Some(jmrun) = args
            .string("jmrunPath")
            .map(|p| resolve(&cwd, &p))
            .or_else(|| find_tool(&cwd, JMRUN_CANDIDATES))
        else {
            self.emitter.fail(
                request,
                "找不到 jmrun（先运行 tools\\build.ps1 或用 jmrunPath 指定）",
            );
            return;
        };

        let mut final_program = program_path.clone();
        let mut cleanup = args
            .get("cleanup")
            .map(|value| matches!(value, Json::Bool(true)))
            .unwrap_or(false);
        if program_path.extension().and_then(|ext| ext.to_str()) == Some("jsh") {
            let Some(micc) = micc else {
                self.emitter.fail(request, "编译 .jsh 需要 micc");
                return;
            };
            let target = temp_program();
            let result = Command::new(&micc)
                .arg(&program_path)
                .arg("-o")
                .arg(&target)
                .current_dir(&cwd)
                .output();
            match result {
                Ok(output) if output.status.success() => {
                    final_program = target;
                    cleanup = true;
                }
                Ok(output) => {
                    let text = String::from_utf8_lossy(&output.stderr);
                    self.emitter.output("console", &text);
                    self.emitter.fail(request, "micc 编译失败");
                    return;
                }
                Err(error) => {
                    self.emitter
                        .fail(request, &format!("无法启动 micc：{}", error));
                    return;
                }
            }
        }

        let mut breaks = Vec::new();
        if let Some(items) = args.get("breaks").and_then(Json::as_arr) {
            for item in items {
                let parsed = match item {
                    Json::Str(text) => parse_spec(text),
                    _ => break_from_json(item),
                };
                if let Some(found) = parsed {
                    breaks.push(found);
                }
            }
        }
        self.launch_breaks = breaks;
        self.launch = Some(Launch {
            program: final_program,
            cwd,
            jmrun,
            task: args.get("task").and_then(Json::as_i64),
            cleanup,
        });
        self.emitter.respond(request, Some(obj(vec![])));
        // DAP 的顺序：launch 响应之后再发 initialized，客户端才会下发
        // setBreakpoints / configurationDone。
        self.emitter.event("initialized", obj(vec![]));
    }

    fn start(&mut self) {
        let Some(launch) = self.launch.clone() else {
            return;
        };
        let mut command = Command::new(&launch.jmrun);
        command.arg("--dbg").arg("--事件流");
        for item in self.merged_breaks() {
            command.arg("--break").arg(break_spec(&item));
        }
        if let Some(task) = launch.task {
            command.arg("--task").arg(task.to_string());
        }
        command
            .arg(&launch.program)
            .current_dir(&launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match command.spawn() {
            Ok(mut child) => {
                self.stdin = child.stdin.take();
                if let Some(stdout) = child.stdout.take() {
                    spawn_stdout(self.emitter.clone(), stdout);
                }
                if let Some(stderr) = child.stderr.take() {
                    spawn_stderr(
                        self.emitter.clone(),
                        self.state.clone(),
                        self.replies.clone(),
                        stderr,
                    );
                }
                self.child = Some(child);
            }
            Err(error) => {
                self.emitter
                    .output("console", &format!("无法启动 jmrun：{}\n", error));
                self.emitter.event("terminated", obj(vec![]));
            }
        }
    }

    fn merged_breaks(&self) -> Vec<Break> {
        let mut out: Vec<Break> = Vec::new();
        for item in self
            .launch_breaks
            .iter()
            .chain(self.client_breaks.values().flatten())
        {
            if out.len() >= MAX_BREAKS {
                break;
            }
            if out
                .iter()
                .any(|existing| existing.file == item.file && existing.line == item.line)
            {
                continue;
            }
            out.push(item.clone());
        }
        out
    }

    fn stack_trace(&mut self, request: &Json) {
        if !self.is_paused() {
            self.emitter.respond(
                request,
                Some(obj(vec![
                    ("stackFrames", Json::Arr(vec![])),
                    ("totalFrames", json::n(0.0)),
                ])),
            );
            return;
        }
        self.drain_replies("栈");
        self.inspect("bt");
        let reply = wait_reply(&self.replies, "栈", REPLY_TIMEOUT_MS);
        let mut frames = Vec::new();
        if let Some(items) = reply
            .as_ref()
            .and_then(|value| value.get("帧"))
            .and_then(Json::as_arr)
        {
            for (index, item) in items.iter().enumerate() {
                let file = item.string("文件").unwrap_or_default();
                let line = item.get("行").and_then(Json::as_i64).unwrap_or(0).max(1);
                frames.push(obj(vec![
                    ("id", json::n((index + 1) as f64)),
                    (
                        "name",
                        s(item.string("函数").unwrap_or_else(|| "?".to_string())),
                    ),
                    (
                        "source",
                        obj(vec![
                            ("name", s(file.clone())),
                            ("path", s(self.source_path(&file))),
                        ]),
                    ),
                    ("line", json::n(line as f64)),
                    ("column", json::n(1.0)),
                ]));
            }
        }
        let total = frames.len();
        self.emitter.respond(
            request,
            Some(obj(vec![
                ("stackFrames", Json::Arr(frames)),
                ("totalFrames", json::n(total as f64)),
            ])),
        );
    }

    fn scopes(&self, request: &Json) {
        let scopes = if self.is_paused() {
            vec![obj(vec![
                ("name", s("局部变量")),
                ("variablesReference", json::n(1.0)),
                ("expensive", Json::Bool(false)),
            ])]
        } else {
            vec![]
        };
        self.emitter
            .respond(request, Some(obj(vec![("scopes", Json::Arr(scopes))])));
    }

    fn variables(&mut self, request: &Json, args: &Json) {
        let reference = args
            .get("variablesReference")
            .and_then(Json::as_i64)
            .unwrap_or(0);
        if reference != 1 || !self.is_paused() {
            self.emitter
                .respond(request, Some(obj(vec![("variables", Json::Arr(vec![]))])));
            return;
        }
        self.drain_replies("变量");
        self.inspect("v");
        let reply = wait_reply(&self.replies, "变量", REPLY_TIMEOUT_MS);
        let mut variables = Vec::new();
        if let Some(items) = reply
            .as_ref()
            .and_then(|value| value.get("局部"))
            .and_then(Json::as_arr)
        {
            for item in items {
                variables.push(obj(vec![
                    ("name", s(item.string("名").unwrap_or_default())),
                    ("value", s(item.string("值").unwrap_or_default())),
                    ("variablesReference", json::n(0.0)),
                ]));
            }
        }
        self.emitter.respond(
            request,
            Some(obj(vec![("variables", Json::Arr(variables))])),
        );
    }

    fn stop(&mut self, request: &Json) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "q");
            let _ = stdin.flush();
        }
        self.kill_child();
        self.emitter.respond(request, Some(obj(vec![])));
        let first = {
            let mut state = self.state.lock().unwrap();
            let first = !state.ended;
            state.ended = true;
            state.paused = false;
            first
        };
        if first {
            self.emitter.event("terminated", obj(vec![]));
        }
        self.cleanup_program();
        self.done = true;
    }

    fn shutdown(&mut self) {
        self.kill_child();
        self.cleanup_program();
    }

    fn kill_child(&mut self) {
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn cleanup_program(&mut self) {
        if let Some(launch) = self.launch.as_ref() {
            if launch.cleanup {
                let _ = std::fs::remove_file(&launch.program);
            }
        }
    }

    fn is_paused(&self) -> bool {
        self.state.lock().map(|state| state.paused).unwrap_or(false)
    }

    fn command(&mut self, text: &str) {
        self.write_stdin(text);
        if let Ok(mut state) = self.state.lock() {
            state.paused = false;
        }
    }

    /// 只读命令（`bt` / `v`）：不改变暂停状态。
    fn inspect(&mut self, text: &str) {
        self.write_stdin(text);
    }

    /// 上一次请求超时留下的同类回复不能算到这一次头上。
    fn drain_replies(&self, kind: &str) {
        let (lock, _) = &*self.replies;
        if let Ok(mut queue) = lock.lock() {
            queue.retain(|item| item.string("事件").as_deref() != Some(kind));
        }
    }

    fn write_stdin(&mut self, text: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "{}", text);
            let _ = stdin.flush();
        }
    }

    fn source_path(&self, file: &str) -> String {
        if file.is_empty() {
            return String::new();
        }
        let path = Path::new(file);
        if path.is_absolute() {
            return file.to_string();
        }
        match self.launch.as_ref() {
            Some(launch) => launch.cwd.join(path).to_string_lossy().to_string(),
            None => file.to_string(),
        }
    }
}

/* ---------------- 子进程读取 ---------------- */

fn spawn_stdout(emitter: Emitter, stdout: impl Read + Send + 'static) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&buf).to_string();
            if !text.is_empty() {
                emitter.output("stdout", &text);
            }
        }
    });
}

fn spawn_stderr(
    emitter: Emitter,
    state: Arc<Mutex<State>>,
    replies: Arc<(Mutex<Vec<Json>>, Condvar)>,
    stderr: impl Read + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let raw = String::from_utf8_lossy(&buf);
            let line = raw.trim_end_matches(['\r', '\n']).to_string();
            if line.trim().is_empty() {
                continue;
            }
            if handle_stream_line(&emitter, &state, &replies, &line) {
                continue;
            }
            if line.contains("(砾调试)") {
                continue;
            }
            emitter.output("console", &format!("{}\n", line));
        }
        let first = {
            let mut state = state.lock().unwrap();
            let first = !state.ended;
            state.ended = true;
            state.paused = false;
            first
        };
        if first {
            emitter.event("terminated", obj(vec![]));
        }
    });
}

/// 返回 true = 已作为事件/响应消费，不再当普通文本。
fn handle_stream_line(
    emitter: &Emitter,
    state: &Arc<Mutex<State>>,
    replies: &Arc<(Mutex<Vec<Json>>, Condvar)>,
    line: &str,
) -> bool {
    let text = line.trim_start_matches('\u{feff}').trim();
    let Some(start) = text.find('{') else {
        return false;
    };
    let Ok(value) = json::parse(&text[start..]) else {
        return false;
    };
    let Some(kind) = value.string("事件") else {
        return false;
    };
    match kind.as_str() {
        "暂停" => {
            let task = value.get("任务").and_then(Json::as_i64).unwrap_or(0);
            let func = value.string("函数").unwrap_or_else(|| "?".to_string());
            let file = value.string("文件").unwrap_or_default();
            let line_no = value.get("行").and_then(Json::as_i64).unwrap_or(0);
            let reason_text = value.string("原因").unwrap_or_default();
            let reason = if reason_text == "断点" {
                "breakpoint"
            } else {
                "step"
            };
            {
                let mut state = state.lock().unwrap();
                state.paused = true;
                state.ended = false;
                state.task = task;
            }
            let where_text = if line_no > 0 {
                format!("{}:{}", file, line_no)
            } else {
                "（无行表）".to_string()
            };
            emitter.event(
                "stopped",
                obj(vec![
                    ("reason", s(reason)),
                    (
                        "description",
                        s(format!("任务 {}「{}」 {}", task, func, where_text)),
                    ),
                    ("threadId", json::n(THREAD_ID as f64)),
                    ("allThreadsStopped", Json::Bool(true)),
                ]),
            );
            true
        }
        "结束" => {
            let code = value.get("码").and_then(Json::as_i64).unwrap_or(0);
            let first = {
                let mut state = state.lock().unwrap();
                let first = !state.ended;
                state.ended = true;
                state.paused = false;
                first
            };
            if first {
                emitter.event("exited", obj(vec![("exitCode", json::n(code as f64))]));
                emitter.event("terminated", obj(vec![]));
            }
            true
        }
        "栈" | "变量" => {
            let (lock, cv) = &**replies;
            let mut queue = lock.lock().unwrap();
            queue.push(value);
            cv.notify_all();
            true
        }
        _ => false,
    }
}

fn wait_reply(
    replies: &Arc<(Mutex<Vec<Json>>, Condvar)>,
    kind: &str,
    timeout_ms: u64,
) -> Option<Json> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let (lock, cv) = &**replies;
    let mut queue = lock.lock().unwrap();
    loop {
        if let Some(index) = queue
            .iter()
            .position(|item| item.string("事件").as_deref() == Some(kind))
        {
            return Some(queue.remove(index));
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let (guard, timeout) = cv.wait_timeout(queue, deadline - now).ok()?;
        queue = guard;
        if timeout.timed_out() {
            return queue
                .iter()
                .position(|item| item.string("事件").as_deref() == Some(kind))
                .map(|index| queue.remove(index));
        }
    }
}

/* ---------------- 路径与断点 ---------------- */

const MICC_CANDIDATES: &[&str] = &[
    "compiler/target/release/micc.exe",
    "compiler/target/release/micc",
    "compiler/target/debug/micc.exe",
    "compiler/target/debug/micc",
];
const JMRUN_CANDIDATES: &[&str] = &["build/jmrun.exe", "build/jmrun"];

fn find_tool(cwd: &Path, candidates: &[&str]) -> Option<PathBuf> {
    for candidate in candidates {
        let path = cwd.join(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn resolve(base: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn temp_program() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("li-dap-{}-{}.jmb", std::process::id(), nanos))
}

fn break_from_json(value: &Json) -> Option<Break> {
    let file = value.string("file")?;
    let line = value.get("line").and_then(Json::as_i64)?;
    if file.is_empty() || line <= 0 {
        return None;
    }
    Some(Break {
        file,
        line,
        condition: value
            .string("condition")
            .unwrap_or_default()
            .trim()
            .to_string(),
    })
}

fn parse_spec(text: &str) -> Option<Break> {
    let text = text.trim();
    let (base, condition) = match text.split_once(" if ") {
        Some((head, tail)) => (head.trim(), tail.trim().to_string()),
        None => (text, String::new()),
    };
    let colon = base.rfind(':')?;
    let file = base[..colon].trim().to_string();
    let line = base[colon + 1..].trim().parse::<i64>().ok()?;
    if file.is_empty() || line <= 0 {
        return None;
    }
    Some(Break {
        file,
        line,
        condition,
    })
}

fn break_spec(item: &Break) -> String {
    let file = item.file.replace('\\', "/");
    if item.condition.is_empty() {
        format!("{}:{}", file, item.line)
    } else {
        format!("{}:{} if {}", file, item.line, item.condition)
    }
}
