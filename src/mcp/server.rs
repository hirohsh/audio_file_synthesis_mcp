use std::io::{self, BufRead, BufReader, BufWriter, Write};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::AppError;

use super::tools::{McpServer, SynthesizeMonoAudioRequest, TOOL_NAME};

const JSON_RPC_VERSION: &str = "2.0";
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MAX_CONTENT_LENGTH: usize = 200 * 1024 * 1024; // 200 MB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageFraming {
    ContentLength,
    LineDelimited,
}

#[derive(Debug)]
struct InboundMessage {
    payload: String,
    framing: MessageFraming,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default = "default_jsonrpc_version")]
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

fn default_jsonrpc_version() -> String {
    JSON_RPC_VERSION.to_string()
}

pub fn run_stdio_server() -> Result<(), AppError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    let server = McpServer;

    loop {
        let Some(message) = read_message(&mut reader)? else {
            break;
        };

        let response = match serde_json::from_str::<JsonRpcRequest>(&message.payload) {
            Ok(request) => handle_request(&server, request),
            Err(error) => Some(error_response(
                &Value::Null,
                -32700,
                format!("parse error: {error}"),
            )),
        };

        if let Some(response) = response {
            write_message(&mut writer, &response, message.framing)?;
        }
    }

    writer.flush()?;
    Ok(())
}

fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<InboundMessage>, AppError> {
    let mut first_line = String::new();
    loop {
        first_line.clear();
        let bytes_read = reader.read_line(&mut first_line)?;
        if bytes_read == 0 {
            return Ok(None);
        }
        if !first_line.trim().is_empty() {
            break;
        }
    }

    let trimmed_first = first_line.trim_end_matches(['\r', '\n']);
    let first_non_ws = trimmed_first.trim_start();
    if first_non_ws.starts_with('{') || first_non_ws.starts_with('[') {
        return Ok(Some(InboundMessage {
            payload: trimmed_first.to_string(),
            framing: MessageFraming::LineDelimited,
        }));
    }

    let mut content_length = parse_content_length_header(trimmed_first)?;
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            return Err(AppError::Decode(
                "unexpected EOF while reading MCP headers".to_string(),
            ));
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if let Some(parsed) = parse_content_length_header(trimmed)? {
            content_length = Some(parsed);
        }
    }

    let content_length = content_length
        .ok_or_else(|| AppError::Decode("missing Content-Length header".to_string()))?;

    if content_length > MAX_CONTENT_LENGTH {
        return Err(AppError::Decode(format!(
            "Content-Length {} exceeds maximum allowed {}",
            content_length, MAX_CONTENT_LENGTH
        )));
    }

    let mut payload = vec![0_u8; content_length];
    reader.read_exact(&mut payload)?;

    let payload =
        String::from_utf8(payload).map_err(|_| AppError::Decode("non-utf8 payload".to_string()))?;
    Ok(Some(InboundMessage {
        payload,
        framing: MessageFraming::ContentLength,
    }))
}

fn parse_content_length_header(line: &str) -> Result<Option<usize>, AppError> {
    let Some((name, value)) = line.split_once(':') else {
        return Ok(None);
    };
    if !name.eq_ignore_ascii_case("Content-Length") {
        return Ok(None);
    }
    let parsed = value
        .trim()
        .parse::<usize>()
        .map_err(|_| AppError::Decode("invalid Content-Length header".to_string()))?;
    Ok(Some(parsed))
}

fn write_message<W: Write>(
    writer: &mut W,
    message: &Value,
    framing: MessageFraming,
) -> Result<(), AppError> {
    let payload = serde_json::to_vec(message)
        .map_err(|error| AppError::Format(format!("failed to serialize response: {error}")))?;
    match framing {
        MessageFraming::ContentLength => {
            write!(writer, "Content-Length: {}\r\n\r\n", payload.len())?;
            writer.write_all(&payload)?;
        }
        MessageFraming::LineDelimited => {
            writer.write_all(&payload)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn handle_request(server: &McpServer, request: JsonRpcRequest) -> Option<Value> {
    let is_notification = request.method.starts_with("notifications/");
    if request.jsonrpc != JSON_RPC_VERSION {
        if is_notification {
            return None;
        }
        return Some(error_response(
            &request.id,
            -32600,
            "invalid request: jsonrpc must be 2.0",
        ));
    }

    let response = match request.method.as_str() {
        "initialize" => success_response(
            &request.id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": env!("CARGO_PKG_NAME"),
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "notifications/initialized" => return None,
        "tools/list" => success_response(
            &request.id,
            json!({
                "tools": [
                    {
                        "name": TOOL_NAME,
                        "description": "Synthesize multiple speaker audio files into one mono WAV file.",
                        "inputSchema": tool_input_schema()
                    }
                ]
            }),
        ),
        "tools/call" => handle_tool_call(server, &request.id, request.params),
        "ping" => success_response(&request.id, json!({})),
        _ => error_response(
            &request.id,
            -32601,
            format!("method not found: {}", request.method),
        ),
    };

    if is_notification {
        None
    } else {
        Some(response)
    }
}

fn handle_tool_call(server: &McpServer, id: &Value, params: Value) -> Value {
    let parsed: ToolCallParams = match serde_json::from_value(params) {
        Ok(value) => value,
        Err(error) => {
            return error_response(id, -32602, format!("invalid tools/call params: {error}"));
        }
    };

    if parsed.name != TOOL_NAME {
        return error_response(id, -32602, format!("unknown tool: {}", parsed.name));
    }

    let request: SynthesizeMonoAudioRequest = match serde_json::from_value(parsed.arguments) {
        Ok(value) => value,
        Err(error) => {
            return error_response(id, -32602, format!("invalid tool arguments: {error}"));
        }
    };

    match server.call_synthesize_mono_audio(request) {
        Ok(result) => {
            let structured = match serde_json::to_value(&result) {
                Ok(value) => value,
                Err(error) => {
                    return error_response(
                        id,
                        -32603,
                        format!("failed to serialize tool response: {error}"),
                    );
                }
            };
            success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": "synthesize_mono_audio completed"
                        }
                    ],
                    "structuredContent": structured,
                    "isError": false
                }),
            )
        }
        Err(error) => success_response(
            id,
            json!({
                "content": [
                    {
                        "type": "text",
                        "text": error.to_string()
                    }
                ],
                "isError": true
            }),
        ),
    }
}

fn tool_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["inputs", "output_path"],
        "properties": {
            "inputs": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "required": ["speaker_id", "path"],
                    "properties": {
                        "speaker_id": { "type": "string" },
                        "path": { "type": "string" },
                        "gain_db": { "type": "number", "default": 0.0 },
                        "start_ms": { "type": "integer", "minimum": 0, "default": 0 }
                    }
                }
            },
            "output_path": { "type": "string" },
            "target_sample_rate": { "type": "integer", "minimum": 1, "default": 48000 },
            "normalization": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean", "default": true },
                    "peak_dbfs": { "type": "number", "default": -1.0 }
                }
            }
        }
    })
}

fn success_response(id: &Value, result: Value) -> Value {
    json!({
        "jsonrpc": JSON_RPC_VERSION,
        "id": id,
        "result": result
    })
}

fn error_response(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": JSON_RPC_VERSION,
        "id": id,
        "error": {
            "code": code,
            "message": message.into()
        }
    })
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use serde_json::json;

    use super::{MessageFraming, read_message, write_message};

    #[test]
    fn read_message_supports_line_delimited_json() {
        let input = br#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}
"#;
        let mut reader = BufReader::new(Cursor::new(input));

        let message = read_message(&mut reader)
            .expect("line-delimited message should parse")
            .expect("message should exist");

        assert_eq!(message.framing, MessageFraming::LineDelimited);
        assert_eq!(
            message.payload,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#
        );
    }

    #[test]
    fn read_message_supports_content_length_framing() {
        let payload = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
        let input = format!("Content-Length: {}\r\n\r\n{payload}", payload.len());
        let mut reader = BufReader::new(Cursor::new(input.as_bytes()));

        let message = read_message(&mut reader)
            .expect("content-length message should parse")
            .expect("message should exist");

        assert_eq!(message.framing, MessageFraming::ContentLength);
        assert_eq!(message.payload, payload);
    }

    #[test]
    fn write_message_uses_line_delimited_framing() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {}
        });
        let mut output = Vec::new();

        write_message(&mut output, &response, MessageFraming::LineDelimited)
            .expect("write should succeed");

        let as_text = String::from_utf8(output).expect("output should be utf8");
        assert!(!as_text.starts_with("Content-Length:"));
        assert!(as_text.ends_with('\n'));
        let parsed: serde_json::Value =
            serde_json::from_str(as_text.trim_end()).expect("payload should be valid json");
        assert_eq!(parsed, response);
    }
}
