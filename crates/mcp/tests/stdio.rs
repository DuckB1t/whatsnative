//! End to end checks for the native stdio MCP server.
//!
//! Every daemon here is a fake on a temporary Unix socket. No test starts the
//! installed daemon, opens a real account, or uses real PII.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
static NEXT_CASE: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "oxidezap-mcp-{tag}-{}-{}",
            std::process::id(),
            NEXT_CASE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    fn socket(&self) -> PathBuf {
        self.0.join("daemon.sock")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

enum StatusReply {
    Status(Value),
    Error { code: &'static str, message: String },
    WrongPlane,
}

fn read_frame(reader: &mut impl BufRead) -> Result<Value, String> {
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|error| format!("fake daemon read failed: {error}"))?;
    if read == 0 {
        return Err("fake daemon saw EOF".into());
    }
    serde_json::from_str(line.trim()).map_err(|error| format!("bad frame: {error}"))
}

fn write_frame(writer: &mut impl Write, frame: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(frame).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .map_err(|error| format!("fake daemon write failed: {error}"))
}

fn serve(
    listener: UnixListener,
    local_account: i32,
    status: Option<StatusReply>,
    result: mpsc::Sender<Result<(), String>>,
) {
    let outcome = (|| -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);

        let hello = read_frame(&mut reader)?;
        let expected = json!({
            "protocol": oxidezap_wire::envelope::CURRENT_PROTOCOL_VERSION,
            "client_name": "oxidezap-mcp",
            "local_account": local_account,
            "account_selection": true,
            "read_only": true,
            "session_events": false,
        });
        if hello["action"] != json!("hello") || hello.get("params") != Some(&expected) {
            return Err(format!("unexpected hello: {hello}"));
        }
        let hello_id = hello.get("id").cloned().unwrap_or(Value::Null);
        let wrong_hello = matches!(status, Some(StatusReply::WrongPlane));
        let hello_response = if wrong_hello {
            json!({
                "id": hello_id,
                "status": "ok",
                "type": "ack",
                "data": null,
            })
        } else {
            json!({
                "id": hello_id,
                "status": "ok",
                "type": "account_selected",
                "data": { "local_account": local_account },
            })
        };
        write_frame(&mut stream, &hello_response)?;
        if wrong_hello {
            return Ok(());
        }

        let Some(reply) = status else {
            return Ok(());
        };
        let request = read_frame(&mut reader)?;
        if request["action"] != json!("get_status") {
            return Err(format!("expected get_status, got {request}"));
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let response = match reply {
            StatusReply::Status(status) => json!({
                "id": id,
                "status": "ok",
                "type": "status",
                "data": status,
            }),
            StatusReply::Error { code, message } => json!({
                "id": id,
                "status": "error",
                "code": code,
                "message": message,
            }),
            StatusReply::WrongPlane => json!({
                "id": id,
                "status": "ok",
                "type": "ack",
                "data": null,
            }),
        };
        write_frame(&mut stream, &response)
    })();
    let _ = result.send(outcome);
}

struct FakeDaemon {
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeDaemon {
    fn start(
        socket: &Path,
        local_account: i32,
        status: Option<StatusReply>,
    ) -> (Self, mpsc::Receiver<Result<(), String>>) {
        let listener = UnixListener::bind(socket).expect("fake daemon bind");
        let (result_tx, result_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("oxidezap-mcp-fake-daemon".into())
            .spawn(move || serve(listener, local_account, status, result_tx))
            .expect("fake daemon thread");
        (
            Self {
                thread: Some(thread),
            },
            result_rx,
        )
    }

    fn assert_healthy(receiver: &mpsc::Receiver<Result<(), String>>) {
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("fake daemon finished")
            .expect("fake daemon saw the expected frames");
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Launch<'a> {
    socket: Option<&'a Path>,
    account_arg: Option<&'a str>,
    account_env: Option<&'a str>,
}

impl Launch<'_> {
    fn spawn(self) -> McpProcess {
        let mut command = Command::new(env!("CARGO_BIN_EXE_oxidezap-mcp"));
        command
            .env_remove("OXIDEZAP_SOCKET")
            .env_remove("OXIDEZAP_LOCAL_ACCOUNT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(socket) = self.socket {
            command.arg("--socket").arg(socket);
        }
        if let Some(account) = self.account_arg {
            command.arg("--local-account").arg(account);
        }
        if let Some(account) = self.account_env {
            command.env("OXIDEZAP_LOCAL_ACCOUNT", account);
        }
        let mut child = command.spawn().expect("spawn oxidezap-mcp");
        McpProcess {
            next_id: 1,
            stdin: child.stdin.take().expect("stdin pipe"),
            stdout: BufReader::new(child.stdout.take().expect("stdout pipe")),
            child,
        }
    }
}

struct McpProcess {
    next_id: u64,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    child: Child,
}

impl McpProcess {
    fn write_line(&mut self, line: &str) {
        self.stdin
            .write_all(line.as_bytes())
            .expect("write request to server");
        let _ = self.stdin.write_all(b"\n");
        self.stdin.flush().expect("flush request to server");
    }

    fn read_response(&mut self) -> Value {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).expect("read server line");
        assert_ne!(read, 0, "server closed stdout before answering");
        let value: Value = serde_json::from_str(line.trim()).expect("server JSON line");
        assert_eq!(
            value["jsonrpc"],
            json!("2.0"),
            "unexpected response: {value}"
        );
        value
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.write_line(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        );
        let response = self.read_response();
        assert_eq!(response["id"], json!(id), "response id: {response}");
        response
    }

    fn request_raw(&mut self, line: &str) -> Value {
        self.write_line(line);
        self.read_response()
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.write_line(&json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string());
    }

    fn initialize(&mut self) -> Value {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "mcp-tests", "version": "1" },
            }),
        );
        assert!(
            response.get("result").is_some(),
            "initialize failed: {response}"
        );
        self.notify("notifications/initialized", json!({}));
        response
    }

    fn list_tools(&mut self) -> Value {
        self.request("tools/list", json!({}))
    }

    fn call_tool(&mut self, name: &str, arguments: Option<Value>) -> Value {
        let mut params = json!({ "name": name });
        if let Some(arguments) = arguments {
            params["arguments"] = arguments;
        }
        self.request("tools/call", params)
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {},
    })
}

fn output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "local_account", "state", "phone", "name", "jid", "lid", "qr_ascii",
            "pair_code", "pair_expires_at_ms"
        ],
        "properties": {
            "local_account": { "type": "integer", "minimum": 1 },
            "state": { "type": "string" },
            "phone": { "type": ["string", "null"] },
            "name": { "type": ["string", "null"] },
            "jid": { "type": ["string", "null"] },
            "lid": { "type": ["string", "null"] },
            "qr_ascii": { "type": ["string", "null"] },
            "pair_code": { "type": ["string", "null"] },
            "pair_expires_at_ms": { "type": ["integer", "null"] },
        },
    })
}

fn error_envelope(response: &Value) -> Value {
    let result = &response["result"];
    assert_eq!(
        result["isError"],
        json!(true),
        "expected an error: {response}"
    );
    assert_eq!(result["content"].as_array().map(Vec::len), Some(1));
    let text = result["content"][0]["text"]
        .as_str()
        .expect("text content item");
    let from_text: Value = serde_json::from_str(text).expect("text content JSON");
    assert_eq!(&from_text, &result["structuredContent"]);
    from_text
}

fn error_code(response: &Value) -> Value {
    error_envelope(response)["error"]["code"].clone()
}

#[test]
fn initialize_and_tools_list_advertise_only_account_status() {
    let scratch = Scratch::new("list");
    let (daemon, hello) = FakeDaemon::start(&scratch.socket(), 1, None);
    let mut client = Launch {
        socket: Some(&scratch.socket()),
        account_arg: Some("1"),
        account_env: None,
    }
    .spawn();

    let initialize = client.initialize();
    assert_eq!(
        initialize["result"]["serverInfo"]["name"],
        json!("oxidezap-mcp")
    );
    let tools = client.list_tools();
    let listed = tools["result"]["tools"].as_array().expect("tools array");
    assert_eq!(
        listed.len(),
        1,
        "only account_status may be listed: {tools}"
    );
    assert_eq!(listed[0]["name"], json!("account_status"));
    assert_eq!(listed[0]["inputSchema"], input_schema());
    assert_eq!(listed[0]["outputSchema"], output_schema());
    assert_eq!(listed[0]["annotations"]["readOnlyHint"], json!(true));

    FakeDaemon::assert_healthy(&hello);
    drop(daemon);
}

#[test]
fn account_status_success_matches_the_frozen_envelope() {
    let scratch = Scratch::new("success");
    let status = json!({
        "state": "connected",
        "phone": "phone.invalid",
        "name": "Test Name",
        "jid": "alice@jid.invalid",
        "lid": "lid.invalid",
        "qr_ascii": "QR",
        "pair_code": "PAIR",
        "pair_expires_at_ms": 1234,
    });
    let expected_data = json!({
        "local_account": 7,
        "state": "connected",
        "phone": "phone.invalid",
        "name": "Test Name",
        "jid": "alice@jid.invalid",
        "lid": "lid.invalid",
        "qr_ascii": "QR",
        "pair_code": "PAIR",
        "pair_expires_at_ms": 1234,
    });
    let (daemon, frames) =
        FakeDaemon::start(&scratch.socket(), 7, Some(StatusReply::Status(status)));
    let mut client = Launch {
        socket: Some(&scratch.socket()),
        account_arg: Some("7"),
        account_env: None,
    }
    .spawn();

    client.initialize();
    let response = client.call_tool("account_status", Some(json!({})));
    let result = &response["result"];
    assert_ne!(
        result["isError"],
        json!(true),
        "unexpected error: {response}"
    );
    assert_eq!(
        result["structuredContent"],
        json!({"ok": true, "data": expected_data})
    );
    assert_eq!(result["content"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["content"][0]["type"], json!("text"));
    let text: Value = serde_json::from_str(
        result["content"][0]["text"]
            .as_str()
            .expect("text content item"),
    )
    .expect("text content JSON");
    assert_eq!(text, result["structuredContent"]);

    FakeDaemon::assert_healthy(&frames);
    drop(daemon);
}

#[test]
fn malformed_and_unknown_requests_use_json_rpc_codes() {
    let scratch = Scratch::new("rpc-errors");
    let (daemon, hello) = FakeDaemon::start(&scratch.socket(), 1, None);
    let mut client = Launch {
        socket: Some(&scratch.socket()),
        account_arg: Some("1"),
        account_env: None,
    }
    .spawn();
    client.initialize();

    let malformed = client.request_raw("{\"jsonrpc\":\"2.0\",\"method\":");
    assert_eq!(malformed["error"]["code"], json!(-32600), "{malformed}");

    let unknown_method = client.request("does/not/exist", json!({}));
    assert_eq!(
        unknown_method["error"]["code"],
        json!(-32601),
        "{unknown_method}"
    );

    let unknown_tool = client.call_tool("list_chats", None);
    assert_eq!(
        unknown_tool["error"]["code"],
        json!(-32601),
        "{unknown_tool}"
    );

    let bad_params = client.call_tool("account_status", Some(json!({"limit": 1})));
    assert_eq!(bad_params["error"]["code"], json!(-32602), "{bad_params}");

    FakeDaemon::assert_healthy(&hello);
    drop(daemon);
}

#[test]
fn startup_without_an_account_reports_account_not_selected() {
    let mut client = Launch {
        socket: None,
        account_arg: None,
        account_env: None,
    }
    .spawn();
    client.initialize();

    let tools = client.list_tools();
    assert_eq!(tools["result"]["tools"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        error_code(&client.call_tool("account_status", None)),
        json!("account_not_selected")
    );
}

#[test]
fn startup_with_an_invalid_account_reports_invalid_account_id() {
    for invalid in ["0", "abc"] {
        let mut client = Launch {
            socket: None,
            account_arg: Some(invalid),
            account_env: None,
        }
        .spawn();
        client.initialize();
        assert_eq!(
            error_code(&client.call_tool("account_status", None)),
            json!("invalid_account_id"),
            "invalid value {invalid}"
        );
    }

    let mut from_env = Launch {
        socket: None,
        account_arg: None,
        account_env: Some("-1"),
    }
    .spawn();
    from_env.initialize();
    assert_eq!(
        error_code(&from_env.call_tool("account_status", None)),
        json!("invalid_account_id")
    );
}

#[test]
fn a_valid_environment_account_is_used_for_the_hello() {
    let scratch = Scratch::new("env-account");
    let (daemon, hello) = FakeDaemon::start(&scratch.socket(), 9, None);
    let mut client = Launch {
        socket: Some(&scratch.socket()),
        account_arg: None,
        account_env: Some("9"),
    }
    .spawn();
    client.initialize();
    let tools = client.list_tools();
    assert_eq!(tools["result"]["tools"].as_array().map(Vec::len), Some(1));
    FakeDaemon::assert_healthy(&hello);
    drop(daemon);
}

#[test]
fn missing_daemon_reports_daemon_not_running() {
    let scratch = Scratch::new("no-daemon");
    let mut client = Launch {
        socket: Some(&scratch.socket()),
        account_arg: Some("1"),
        account_env: None,
    }
    .spawn();
    client.initialize();
    assert_eq!(
        error_code(&client.call_tool("account_status", None)),
        json!("daemon_not_running")
    );
}

#[test]
fn wrong_frame_answer_reports_frame_plane_mismatch() {
    let scratch = Scratch::new("wrong-plane");
    let (daemon, frames) = FakeDaemon::start(&scratch.socket(), 1, Some(StatusReply::WrongPlane));
    let mut client = Launch {
        socket: Some(&scratch.socket()),
        account_arg: Some("1"),
        account_env: None,
    }
    .spawn();
    client.initialize();
    assert_eq!(
        error_code(&client.call_tool("account_status", None)),
        json!("frame_plane_mismatch")
    );
    FakeDaemon::assert_healthy(&frames);
    drop(daemon);
}

#[test]
fn daemon_error_messages_do_not_leak_account_details() {
    let scratch = Scratch::new("redaction");
    let secret = "token=abc 5551234567 jid@jid.invalid /tmp/secret";
    let (daemon, frames) = FakeDaemon::start(
        &scratch.socket(),
        1,
        Some(StatusReply::Error {
            code: "not_connected",
            message: secret.into(),
        }),
    );
    let mut client = Launch {
        socket: Some(&scratch.socket()),
        account_arg: Some("1"),
        account_env: None,
    }
    .spawn();
    client.initialize();
    let response = client.call_tool("account_status", None);
    let envelope = error_envelope(&response);
    assert_eq!(envelope["error"]["code"], json!("not_connected"));
    let rendered = response.to_string();
    for leak in ["token=abc", "5551234567", "jid@jid.invalid", "/tmp/secret"] {
        assert!(!rendered.contains(leak), "leaked {leak}: {rendered}");
    }
    FakeDaemon::assert_healthy(&frames);
    drop(daemon);
}

#[test]
fn oversized_status_reports_response_too_large() {
    let scratch = Scratch::new("too-large");
    let mut status = json!({
        "state": "connected",
        "phone": null,
        "name": null,
        "jid": null,
        "lid": null,
        "qr_ascii": null,
        "pair_code": null,
        "pair_expires_at_ms": null,
    });
    status["qr_ascii"] = json!("x".repeat(MAX_RESPONSE_BYTES + 1));
    let (daemon, frames) =
        FakeDaemon::start(&scratch.socket(), 1, Some(StatusReply::Status(status)));
    let mut client = Launch {
        socket: Some(&scratch.socket()),
        account_arg: Some("1"),
        account_env: None,
    }
    .spawn();
    client.initialize();
    assert_eq!(
        error_code(&client.call_tool("account_status", None)),
        json!("response_too_large")
    );
    FakeDaemon::assert_healthy(&frames);
    drop(daemon);
}
