//! BaseProtocol stdio framed message codec shared by LSP and DAP.
//!
//! Handles encoding and decoding of JSON messages framed with headers:
//! `Content-Length: <length>\r\n\r\n<body_json>`

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Header format error: {0}")]
    InvalidHeader(String),
    #[error("Missing Content-Length header")]
    MissingContentLength,
    #[error("Invalid Content-Length value: {0}")]
    InvalidContentLength(String),
}

/// Encodes a payload with framed `Content-Length` header.
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
pub fn decode_framed_message(buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>, CodecError> {
    // 0. Trim any leading newline or carriage-return bytes left by previous messages
    let trim_count = buffer
        .iter()
        .take_while(|&&b| b == b'\r' || b == b'\n')
        .count();
    if trim_count > 0 {
        buffer.drain(0..trim_count);
    }

    // 1. Locate delimiter between headers and body: whichever occurs first between "\r\n\r\n" and "\n\n"
    let pos_crlf = buffer.windows(4).position(|w| w == b"\r\n\r\n").map(|p| (p, p + 4));
    let pos_lf = buffer.windows(2).position(|w| w == b"\n\n").map(|p| (p, p + 2));

    let (header_len, body_start) = match (pos_crlf, pos_lf) {
        (Some((h1, b1)), Some((h2, b2))) => {
            if h1 <= h2 {
                (h1, b1)
            } else {
                (h2, b2)
            }
        }
        (Some((h, b)), None) => (h, b),
        (None, Some((h, b))) => (h, b),
        (None, None) => return Ok(None),
    };

    // 2. Parse headers
    let header_bytes = &buffer[..header_len];
    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|e| CodecError::InvalidHeader(format!("Header is not valid UTF-8: {}", e)))?;

    let mut content_length = None;
    for line in header_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim().eq_ignore_ascii_case("Content-Length") {
                let len: usize = v.trim().parse().map_err(|_| {
                    CodecError::InvalidContentLength(v.trim().to_string())
                })?;
                content_length = Some(len);
            }
        }
    }

    let length = content_length.ok_or(CodecError::MissingContentLength)?;

    // 3. Check if body is fully received
    if buffer.len() < body_start + length {
        return Ok(None);
    }

    // 4. Extract message and drain buffer
    let message_bytes = buffer[body_start..body_start + length].to_vec();
    buffer.drain(0..body_start + length);
    Ok(Some(message_bytes))
}

/// Reads from stream until one full framed message is decoded.
pub async fn read_framed_message<R: AsyncRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>, CodecError> {
    let mut temp = [0u8; 4096];
    loop {
        if let Some(msg) = decode_framed_message(buffer)? {
            return Ok(Some(msg));
        }

        let n = reader.read(&mut temp).await?;
        if n == 0 {
            if buffer.is_empty() {
                return Ok(None);
            } else {
                return Err(CodecError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "Stream closed with incomplete framed message",
                )));
            }
        }
        buffer.extend_from_slice(&temp[..n]);
    }
}

/// Writes a framed message payload to the output stream.
pub async fn write_framed_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<(), CodecError> {
    let framed = encode_framed_message(payload);
    writer.write_all(&framed).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trailing_newlines_between_messages() {
        // Message 1 followed by trailing CRLF before Message 2
        let mut buffer = Vec::new();
        let payload1 = b"{\"msg\":1}";
        let payload2 = b"{\"msg\":2}";

        buffer.extend_from_slice(&encode_framed_message(payload1));
        buffer.extend_from_slice(b"\r\n\r\n"); // trailing blank lines
        buffer.extend_from_slice(&encode_framed_message(payload2));

        let res1 = decode_framed_message(&mut buffer).unwrap();
        assert_eq!(res1, Some(payload1.to_vec()));

        let res2 = decode_framed_message(&mut buffer).unwrap();
        assert_eq!(res2, Some(payload2.to_vec()));

        assert!(buffer.is_empty());
    }

    #[test]
    fn test_unicode_multibyte_payload() {
        let mut buffer = Vec::new();
        let payload = "{\"text\": \"こんにちは 🌍 (RustCode DAP)\"}".as_bytes();

        let framed = encode_framed_message(payload);
        buffer.extend_from_slice(&framed);

        let decoded = decode_framed_message(&mut buffer).unwrap();
        assert_eq!(decoded, Some(payload.to_vec()));
    }

    #[test]
    fn test_malformed_and_missing_content_length() {
        let mut buf_bad_val = b"Content-Length: not_a_number\r\n\r\n{}".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_bad_val),
            Err(CodecError::InvalidContentLength(_))
        ));

        let mut buf_missing = b"Content-Type: application/json\r\n\r\n{}".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_missing),
            Err(CodecError::MissingContentLength)
        ));
    }
}
