//! LSP stdio framed message codec.
//!
//! Handles encoding and decoding of LSP JSON-RPC messages framed with headers:
//! `Content-Length: <length>\r\n\r\n<body_json>`

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Error)]
pub enum LspCodecError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Header format error: {0}")]
    InvalidHeader(String),
    #[error("Missing Content-Length header")]
    MissingContentLength,
    #[error("Invalid Content-Length value: {0}")]
    InvalidContentLength(String),
}

/// Encodes an LSP payload with framed `Content-Length` header.
pub fn encode_framed_message(payload: &[u8]) -> Vec<u8> {
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    let mut out = Vec::with_capacity(header.len() + payload.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(payload);
    out
}

/// Attempts to decode one framed message from the buffer.
/// Returns `Ok(Some(payload))` if a complete message is ready,
/// `Ok(None)` if more data is needed, or `Err(...)` if headers are invalid.
pub fn decode_framed_message(buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>, LspCodecError> {
    // 1. Locate delimiter between headers and body: "\r\n\r\n" or "\n\n"
    let (header_len, body_start) = if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
        (pos, pos + 4)
    } else if let Some(pos) = buffer.windows(2).position(|w| w == b"\n\n") {
        (pos, pos + 2)
    } else {
        return Ok(None);
    };

    // 2. Parse headers
    let header_bytes = &buffer[..header_len];
    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|e| LspCodecError::InvalidHeader(format!("Header is not valid UTF-8: {}", e)))?;

    let mut content_length = None;
    for line in header_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((name, val)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                let parsed: usize = val
                    .trim()
                    .parse()
                    .map_err(|e| LspCodecError::InvalidContentLength(format!("{}: {}", val, e)))?;
                content_length = Some(parsed);
            }
        }
    }

    let len = content_length.ok_or(LspCodecError::MissingContentLength)?;

    // 3. Check if full body has arrived
    if buffer.len() < body_start + len {
        return Ok(None);
    }

    // 4. Extract body and drain buffer
    let body = buffer[body_start..body_start + len].to_vec();
    buffer.drain(..body_start + len);
    Ok(Some(body))
}

/// Asynchronously write a framed message to a stream.
pub async fn write_framed_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<(), std::io::Error> {
    let framed = encode_framed_message(payload);
    writer.write_all(&framed).await?;
    writer.flush().await?;
    Ok(())
}

/// Asynchronously read the next framed message from a stream.
/// Returns `Ok(Some(body))` on success, `Ok(None)` if stream hit EOF,
/// or `Err` if parsing failed.
pub async fn read_framed_message<R: AsyncRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>, LspCodecError> {
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(msg) = decode_framed_message(buffer)? {
            return Ok(Some(msg));
        }

        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            if buffer.is_empty() {
                return Ok(None);
            } else {
                return Err(LspCodecError::InvalidHeader(
                    "Unexpected EOF with partial message in buffer".to_string(),
                ));
            }
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_and_decode_single_frame() {
        let payload = br#"{"jsonrpc":"2.0","method":"test","params":{}}"#;
        let framed = encode_framed_message(payload);
        let expected_header = format!("Content-Length: {}\r\n\r\n", payload.len());
        assert!(framed.starts_with(expected_header.as_bytes()));

        let mut buf = framed;
        let decoded = decode_framed_message(&mut buf).unwrap().expect("message");
        assert_eq!(decoded, payload);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_decode_chunked_streaming() {
        let payload = br#"{"id":1,"result":"ok"}"#;
        let framed = encode_framed_message(payload);

        let mut buf = Vec::new();
        // Feed byte by byte
        for &byte in &framed[..15] {
            buf.push(byte);
            assert!(decode_framed_message(&mut buf).unwrap().is_none());
        }

        buf.extend_from_slice(&framed[15..]);
        let decoded = decode_framed_message(&mut buf).unwrap().expect("message");
        assert_eq!(decoded, payload);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_multiple_messages_in_buffer() {
        let p1 = br#"{"msg":1}"#;
        let p2 = br#"{"msg":2}"#;
        let mut buf = encode_framed_message(p1);
        buf.extend_from_slice(&encode_framed_message(p2));

        let m1 = decode_framed_message(&mut buf).unwrap().expect("msg 1");
        assert_eq!(m1, p1);

        let m2 = decode_framed_message(&mut buf).unwrap().expect("msg 2");
        assert_eq!(m2, p2);

        assert!(decode_framed_message(&mut buf).unwrap().is_none());
    }

    #[test]
    fn test_extra_headers_and_case_insensitivity() {
        let raw = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: 12\r\n\r\nHello, World!";
        let mut buf = raw.to_vec();
        let decoded = decode_framed_message(&mut buf).unwrap().expect("message");
        assert_eq!(decoded, b"Hello, World");
        assert_eq!(buf, b"!");
    }
}
