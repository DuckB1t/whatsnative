//! Native stdio MCP server, Phase 1.
//!
//! This process is a thin read-only front end onto `oxidezapd`. It owns no
//! session, no store and no account state beyond the account id it was told to
//! bind at startup. Phase 1 speaks only the typed wire plane and exposes only
//! `account_status`. The legacy plane is deliberately not implemented here, so
//! no legacy tool is advertised or callable.

use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use oxidezap_ipc::client::IpcClient;
use oxidezap_wire::dto::ConnectionStatusDto;
use oxidezap_wire::envelope::CURRENT_PROTOCOL_VERSION;
use oxidezap_wire::error::ApiError;
use oxidezap_wire::request::ClientRequest;
use oxidezap_wire::response::DaemonResponse;
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorCode, Implementation,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
    Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::oneshot;

const TOOL_NAME: &str = "account_status";
const CLIENT_NAME: &str = "oxidezap-mcp";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// A caller-visible tool failure, always a fixed sentence.
///
/// Daemon messages are never copied here. They may carry a JID, phone number,
/// token or local path, none of which belongs in an MCP error.
#[derive(Clone, Debug)]
struct Failure {
    code: &'static str,
    message: &'static str,
}

impl Failure {
    fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }

    fn envelope(&self) -> Value {
        json!({
            "ok": false,
            "error": {
                "code": self.code,
                "message": self.message,
            },
        })
    }

    fn result(&self) -> CallToolResult {
        CallToolResult::structured_error(self.envelope())
    }
}

fn failure(code: &'static str, message: &'static str) -> Failure {
    Failure::new(code, message)
}

fn invalid_account_id() -> Failure {
    failure(
        "invalid_account_id",
        "startup local account must be a positive integer",
    )
}

fn account_not_selected() -> Failure {
    failure(
        "account_not_selected",
        "startup local account was not selected",
    )
}

fn daemon_not_running() -> Failure {
    failure(
        "daemon_not_running",
        "no daemon is listening on the resolved socket",
    )
}

fn frame_plane_mismatch() -> Failure {
    failure(
        "frame_plane_mismatch",
        "daemon answered with the wrong frame shape",
    )
}

fn response_too_large() -> Failure {
    failure("response_too_large", "result would exceed one mebibyte")
}

fn internal_error() -> Failure {
    failure(
        "internal_error",
        "the daemon request could not be completed",
    )
}

/// Known daemon codes pass through, but the message never does.
fn failure_from_api(error: &ApiError) -> Failure {
    match error.code.as_str() {
        "invalid_account_id" => invalid_account_id(),
        "account_not_selected" => account_not_selected(),
        "account_unavailable" => failure(
            "account_unavailable",
            "the selected local account is unavailable",
        ),
        "daemon_not_running" => daemon_not_running(),
        "socket_connect_failed" => failure(
            "socket_connect_failed",
            "the daemon socket could not be connected",
        ),
        "protocol_mismatch" => failure(
            "protocol_mismatch",
            "daemon reported an unsupported protocol version",
        ),
        "frame_plane_mismatch" => frame_plane_mismatch(),
        "invalid_request" => failure(
            "invalid_request",
            "daemon frame was invalid for this connection",
        ),
        "invalid_argument" => failure("invalid_argument", "daemon rejected the request"),
        "not_connected" => failure("not_connected", "the account has no live session"),
        "not_found" => failure("not_found", "the named item was not found"),
        "permission_denied" => failure("permission_denied", "the daemon refused the request"),
        "too_busy" => failure("too_busy", "the daemon is too busy"),
        "timeout" => failure("timeout", "the daemon did not answer in time"),
        "unsupported" => failure("unsupported", "the daemon lacks that capability"),
        "response_too_large" => response_too_large(),
        "read_only_violation" => failure("read_only_violation", "the request would change state"),
        _ => internal_error(),
    }
}

struct Args {
    local_account: i32,
    socket: Option<PathBuf>,
}

impl Args {
    fn from_environment() -> Result<Self, Failure> {
        let mut socket = std::env::var_os("OXIDEZAP_SOCKET").map(PathBuf::from);
        let mut requested_account = None;
        let mut arguments = std::env::args_os().skip(1);

        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--local-account") => {
                    let raw = arguments.next().ok_or_else(invalid_account_id)?;
                    let raw = raw.to_str().ok_or_else(invalid_account_id)?;
                    requested_account = Some(parse_account(raw)?);
                }
                Some("--socket") => {
                    let raw = arguments.next().ok_or_else(invalid_request_arg)?;
                    socket = Some(PathBuf::from(raw));
                }
                _ => return Err(invalid_request_arg()),
            }
        }

        let local_account = match requested_account {
            Some(id) => id,
            None => match std::env::var("OXIDEZAP_LOCAL_ACCOUNT") {
                Ok(raw) => parse_account(&raw)?,
                Err(_) => return Err(account_not_selected()),
            },
        };

        Ok(Self {
            local_account,
            socket,
        })
    }
}

fn invalid_request_arg() -> Failure {
    failure("invalid_request", "invalid startup argument")
}

fn parse_account(raw: &str) -> Result<i32, Failure> {
    match raw.parse::<i32>() {
        Ok(id) if id > 0 => Ok(id),
        _ => Err(invalid_account_id()),
    }
}

type Reply = Result<Value, Failure>;

struct WorkerRequest {
    reply: oneshot::Sender<Reply>,
}

/// Shared daemon state. The blocking IPC client stays on one worker thread so
/// a `tools/call` never parks a tokio task in a socket read.
struct Backend {
    startup_error: Option<Failure>,
    request_tx: Mutex<Option<mpsc::Sender<WorkerRequest>>>,
}

impl Backend {
    fn start(args: Args) -> Result<Self, Failure> {
        let mut client = match args.socket {
            Some(path) => IpcClient::connect_at(&path).map_err(connect_failure)?,
            None => IpcClient::connect().map_err(connect_failure)?,
        };

        let hello = ClientRequest::Hello {
            protocol: CURRENT_PROTOCOL_VERSION,
            client_name: CLIENT_NAME.into(),
            local_account: Some(args.local_account),
            account_selection: true,
            read_only: true,
            session_events: false,
        };
        let selected = match client.request(hello) {
            Ok(DaemonResponse::AccountSelected { local_account }) => local_account,
            Ok(_) => return Err(frame_plane_mismatch()),
            Err(error) => return Err(failure_from_api(&error)),
        };
        if selected != args.local_account {
            return Err(frame_plane_mismatch());
        }

        let (request_tx, request_rx) = mpsc::channel::<WorkerRequest>();
        thread::Builder::new()
            .name("oxidezap-mcp-daemon".into())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let reply = match client.request(ClientRequest::GetStatus) {
                        Ok(DaemonResponse::Status(status)) => Ok(status_envelope(selected, status)),
                        Ok(_) => Err(frame_plane_mismatch()),
                        Err(error) => Err(failure_from_api(&error)),
                    };
                    let _ = request.reply.send(reply);
                }
            })
            .map_err(|_| internal_error())?;

        Ok(Self {
            startup_error: None,
            request_tx: Mutex::new(Some(request_tx)),
        })
    }

    fn failed(error: Failure) -> Self {
        Self {
            startup_error: Some(error),
            request_tx: Mutex::new(None),
        }
    }

    async fn status(&self) -> Reply {
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        {
            let guard = self.request_tx.lock().map_err(|_| internal_error())?;
            let sender = guard.as_ref().ok_or_else(internal_error)?;
            sender
                .send(WorkerRequest { reply: reply_tx })
                .map_err(|_| internal_error())?;
        }
        reply_rx.await.map_err(|_| internal_error())?
    }
}

fn connect_failure(error: io::Error) -> Failure {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => daemon_not_running(),
        _ => failure(
            "socket_connect_failed",
            "the daemon socket could not be connected",
        ),
    }
}

fn status_envelope(local_account: i32, status: ConnectionStatusDto) -> Value {
    json!({
        "ok": true,
        "data": {
            "local_account": local_account,
            "state": status.state,
            "phone": status.phone,
            "name": status.name,
            "jid": status.jid,
            "lid": status.lid,
            "qr_ascii": status.qr_ascii,
            "pair_code": status.pair_code,
            "pair_expires_at_ms": status.pair_expires_at_ms,
        },
    })
}

fn tool_result(reply: Reply) -> CallToolResponse {
    match reply {
        Ok(value) => {
            if too_large(&value) {
                response_too_large().result().into()
            } else {
                CallToolResult::structured(value).into()
            }
        }
        Err(error) => error.result().into(),
    }
}

fn too_large(value: &Value) -> bool {
    match serde_json::to_vec(value) {
        Ok(bytes) => bytes.len() > MAX_RESPONSE_BYTES,
        Err(_) => true,
    }
}

#[derive(Clone)]
struct McpServer {
    backend: Arc<Backend>,
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        let mut config = ServerConfig::new(capabilities)
            .with_server_info(Implementation::new(CLIENT_NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions("Phase 1 exposes only account_status and never writes state.");
        config.protocol_version = ProtocolVersion::V_2025_11_25;
        config
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::V_2025_11_25))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let input_schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {},
        });
        let output_schema = json!({
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
        });
        let mut tool = Tool::default();
        tool.name = TOOL_NAME.into();
        tool.description = Some("Read the selected local account connection status.".into());
        tool.input_schema = Arc::new(
            input_schema
                .as_object()
                .expect("the literal schema is an object")
                .clone(),
        );
        tool.output_schema = Some(Arc::new(
            output_schema
                .as_object()
                .expect("the literal schema is an object")
                .clone(),
        ));
        let mut annotations = ToolAnnotations::default();
        annotations.read_only_hint = Some(true);
        tool.annotations = Some(annotations);
        Ok(ListToolsResult::with_all_items(vec![tool]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if request.name != TOOL_NAME {
            return Err(McpError::new(
                ErrorCode::METHOD_NOT_FOUND,
                "unknown tool",
                None,
            ));
        }
        if request.input_responses.is_some()
            || request.request_state.is_some()
            || request
                .arguments
                .as_ref()
                .is_some_and(|arguments| !arguments.is_empty())
        {
            return Err(McpError::invalid_params(
                "account_status takes no arguments",
                None,
            ));
        }

        Ok(tool_result(self.backend.status().await))
    }
}

/// Feed stdin to the SDK one line at a time.
///
/// `rmcp` skips unparsable lines without answering them. The tool contract
/// requires `-32600` for malformed JSON-RPC, so an unparsable line is replaced
/// by a well-formed frame with no JSON-RPC shape. The SDK answers that with
/// `-32600` and keeps serving the next line.
fn sanitize_stdin(mut sink: tokio::io::DuplexStream) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(tokio::io::stdin());
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = match reader.read_until(b'\n', &mut line).await {
                Ok(read) => read,
                Err(_) => break,
            };
            if read == 0 {
                break;
            }
            if !line.ends_with(b"\n") {
                line.push(b'\n');
            }
            let body = line.strip_suffix(b"\n").unwrap_or(&line);
            let outgoing: &[u8] = if serde_json::from_slice::<Value>(body).is_ok() {
                &line
            } else {
                b"{\"foo\":\"bar\"}\n"
            };
            if sink.write_all(outgoing).await.is_err() || sink.flush().await.is_err() {
                break;
            }
        }
    });
}

async fn serve(backend: Arc<Backend>) -> Result<(), Box<dyn std::error::Error>> {
    let server = McpServer { backend };
    let (stdin_rx, stdin_tx) = tokio::io::duplex(64 * 1024);
    sanitize_stdin(stdin_tx);
    let running = server.serve((stdin_rx, tokio::io::stdout())).await?;
    running.waiting().await?;
    Ok(())
}

fn main() -> ExitCode {
    let backend = match Args::from_environment().and_then(Backend::start) {
        Ok(backend) => Arc::new(backend),
        Err(error) => Arc::new(Backend::failed(error)),
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return ExitCode::FAILURE,
    };

    match runtime.block_on(serve(backend)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
