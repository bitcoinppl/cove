//! The minimal HTTP/1.1 the fake Esplora server and the control API speak

use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::TcpStream,
};

pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    pub(crate) fn json(value: &Value) -> Self {
        Self::json_with_status(200, value)
    }

    pub(crate) fn json_with_status(status: u16, value: &Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(value).expect("json serializes"),
        }
    }

    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self { status: 200, content_type: "text/plain", body: text.into().into_bytes() }
    }

    pub(crate) fn bytes(body: Vec<u8>) -> Self {
        Self { status: 200, content_type: "application/octet-stream", body }
    }

    pub(crate) fn error(status: u16, message: &str) -> Self {
        Self { status, content_type: "text/plain", body: message.as_bytes().to_vec() }
    }

    pub(crate) fn not_found(message: &str) -> Self {
        Self::error(404, message)
    }
}

/// Reads one request, `None` once the client closes the connection or sends garbage
pub(crate) async fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.ok()? == 0 {
        return None;
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await.ok()?;

        let header = header.trim();
        if header.is_empty() {
            break;
        }

        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().ok()?;
        }
    }

    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).await.ok()?;

    Some(Request { method, path, body })
}

pub(crate) async fn write_response(
    stream: &mut TcpStream,
    response: &Response,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\n\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len(),
    );

    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.flush().await
}

const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    }
}
