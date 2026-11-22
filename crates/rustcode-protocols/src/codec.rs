//! BaseProtocol stdio framed message codec shared by LSP, DAP, and Extension Host RPC.
//!
//! Handles encoding and decoding of messages framed with headers:
//! `Content-Length: <length>\r\n\r\n<body_bytes>`

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_HEADER_BYTES: usize = 64 * 1024;
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Header format error: {0}")]
    InvalidHeader(String),
    #[error("Header exceeds maximum size of {0} bytes")]
    HeaderTooLarge(usize),
    #[error("Missing Content-Length header")]
    MissingContentLength,
    #[error("Invalid Content-Length value: {0}")]
    InvalidContentLength(String),
    #[error("Frame size {size} exceeds maximum size of {max} bytes")]
    FrameTooLarge { size: usize, max: usize },
}

/// Find the first header terminator (`\r\n\r\n` or `\n\n`) in `buf`,
/// returning `(header_len, body_start)`.
pub fn find_header_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    let pos_crlf = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| (p, p + 4));
    let pos_lf = buf
        .windows(2)
        .position(|w| w == b"\n\n")
        .map(|p| (p, p + 2));
    match (pos_crlf, pos_lf) {
        (Some((h1, b1)), Some((h2, b2))) => {
            if h1 <= h2 {
                Some((h1, b1))
            } else {
                Some((h2, b2))
            }
        }
        (Some(pair), None) | (None, Some(pair)) => Some(pair),
        (None, None) => None,
    }
}

/// Trim any leading `\r` or `\n` bytes from `buf`.
pub fn trim_leading_line_breaks(buf: &mut Vec<u8>) {
    let trim_count = buf
        .iter()
        .take_while(|&&b| b == b'\r' || b == b'\n')
        .count();
    if trim_count > 0 {
        buf.drain(0..trim_count);
    }
}

/// Parse and validate `Content-Length` from raw header bytes, enforcing `MAX_FRAME_BYTES`.
pub fn parse_content_length_header(header_bytes: &[u8]) -> Result<usize, CodecError> {
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
                if content_length.is_some() {
                    return Err(CodecError::InvalidHeader(
                        "Duplicate Content-Length header".to_string(),
                    ));
                }
                let val = v.trim();
                if val.is_empty() || !val.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(CodecError::InvalidContentLength(val.to_string()));
                }
                let len: usize = val
                    .parse()
                    .map_err(|_| CodecError::InvalidContentLength(val.to_string()))?;
                content_length = Some(len);
            }
        }
    }

    let length = content_length.ok_or(CodecError::MissingContentLength)?;
    if length > MAX_FRAME_BYTES {
        return Err(CodecError::FrameTooLarge {
            size: length,
            max: MAX_FRAME_BYTES,
        });
    }
    Ok(length)
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
/// `Ok(None)` if more data is needed, or `Err(...)` if headers or limits are invalid.
pub fn decode_framed_message(buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>, CodecError> {
    // 0. Trim any leading newline or carriage-return bytes left by previous messages
    trim_leading_line_breaks(buffer);

    // 1. Locate delimiter between headers and body within MAX_HEADER_BYTES
    let search_end = buffer.len().min(MAX_HEADER_BYTES);
    let (header_len, body_start) = match find_header_terminator(&buffer[..search_end]) {
        Some(pair) => pair,
        None => {
            if buffer.len() >= MAX_HEADER_BYTES {
                return Err(CodecError::HeaderTooLarge(buffer.len()));
            }
            return Ok(None);
        }
    };

    // 2. Parse headers & enforce MAX_FRAME_BYTES before checking/allocating body
    let length = parse_content_length_header(&buffer[..header_len])?;

    // 3. Check if body is fully received
    if buffer.len().saturating_sub(body_start) < length {
        return Ok(None);
    }

    // 4. Extract message and drain buffer
    let message_bytes = buffer[body_start..body_start + length].to_vec();
    buffer.drain(0..body_start + length);
    Ok(Some(message_bytes))
}

/// Reads from stream until one full framed message is decoded using O(N) linear
/// header scanning, bounded `MAX_HEADER_BYTES` / `MAX_FRAME_BYTES` checks, and `read_exact` body reads.
pub async fn read_framed_message<R: AsyncRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>, CodecError> {
    trim_leading_line_breaks(buffer);

    let mut scan_from = 0usize;
    let (header_len, body_start) = loop {
        let search_end = buffer.len().min(MAX_HEADER_BYTES);
        if search_end > scan_from {
            if let Some((rel_h, rel_b)) = find_header_terminator(&buffer[scan_from..search_end]) {
                break (scan_from + rel_h, scan_from + rel_b);
            }
        }

        if buffer.len() >= MAX_HEADER_BYTES {
            return Err(CodecError::HeaderTooLarge(buffer.len()));
        }

        let prev_len = buffer.len();
        let mut temp = [0u8; 8192];
        let n = reader.read(&mut temp).await?;
        if n == 0 {
            if buffer.is_empty() {
                return Ok(None);
            }
            return Err(CodecError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Stream closed with incomplete framed message",
            )));
        }

        if prev_len == 0 {
            let lead = temp[..n]
                .iter()
                .take_while(|&&b| b == b'\r' || b == b'\n')
                .count();
            buffer.extend_from_slice(&temp[lead..n]);
            scan_from = 0;
        } else {
            buffer.extend_from_slice(&temp[..n]);
            scan_from = prev_len.saturating_sub(3);
        }
    };

    let length = parse_content_length_header(&buffer[..header_len])?;

    let available = buffer.len().saturating_sub(body_start);
    if available >= length {
        let body = buffer[body_start..body_start + length].to_vec();
        buffer.drain(0..body_start + length);
        return Ok(Some(body));
    }

    let mut body = vec![0u8; length];
    if available > 0 {
        body[..available].copy_from_slice(&buffer[body_start..]);
    }
    buffer.clear();
    reader.read_exact(&mut body[available..]).await?;
    Ok(Some(body))
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
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{BufReader, ReadBuf};

    struct ChunkedReader {
        data: Vec<u8>,
        pos: usize,
        chunk_size: usize,
    }

    impl AsyncRead for ChunkedReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.pos >= self.data.len() {
                return Poll::Ready(Ok(()));
            }
            let remaining = self.data.len() - self.pos;
            let take = remaining.min(self.chunk_size).min(buf.remaining());
            let end = self.pos + take;
            buf.put_slice(&self.data[self.pos..end]);
            self.pos = end;
            Poll::Ready(Ok(()))
        }
    }

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

        let mut buf_negative = b"Content-Length: -5\r\n\r\n{}".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_negative),
            Err(CodecError::InvalidContentLength(_))
        ));

        let mut buf_plus = b"Content-Length: +5\r\n\r\nhello".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_plus),
            Err(CodecError::InvalidContentLength(_))
        ));

        let mut buf_empty_val = b"Content-Length: \r\n\r\n".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_empty_val),
            Err(CodecError::InvalidContentLength(_))
        ));

        let mut buf_dup_first_small = b"Content-Length: 2\r\nContent-Length: 999\r\n\r\n{}".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_dup_first_small),
            Err(CodecError::InvalidHeader(_))
        ));

        let mut buf_dup_last_small = b"Content-Length: 999\r\nContent-Length: 2\r\n\r\n{}".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_dup_last_small),
            Err(CodecError::InvalidHeader(_))
        ));

        let mut buf_dup_same = b"Content-Length: 2\r\ncontent-length: 2\r\n\r\n{}".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_dup_same),
            Err(CodecError::InvalidHeader(_))
        ));

        let mut buf_non_utf8 = b"X-Bad: \xFF\xFE\r\nContent-Length: 2\r\n\r\n{}".to_vec();
        assert!(matches!(
            decode_framed_message(&mut buf_non_utf8),
            Err(CodecError::InvalidHeader(_))
        ));
    }

    #[test]
    fn test_extra_headers_and_case_insensitivity() {
        let raw = b"content-length: 15\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{\"ok\":12345678}";
        let mut buf = raw.to_vec();
        let decoded = decode_framed_message(&mut buf).unwrap().expect("message");
        assert_eq!(decoded, b"{\"ok\":12345678}");
        assert!(buf.is_empty());
    }

    #[test]
    fn test_decode_chunked_streaming_and_lf_only_boundary() {
        let raw = b"Content-Length: 5\n\nhello";
        let mut buf = Vec::new();

        // Feed up to the middle of "\n\n"
        buf.extend_from_slice(&raw[..18]); // "Content-Length: 5\n"
        assert!(decode_framed_message(&mut buf).unwrap().is_none());

        buf.extend_from_slice(&raw[18..]);
        let decoded = decode_framed_message(&mut buf).unwrap().expect("message");
        assert_eq!(decoded, b"hello");
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn test_roundtrip_empty_json_and_pipelined_frames_in_bufreader() {
        let mut wire = encode_framed_message(b"{}");
        wire.extend_from_slice(&encode_framed_message("{\"msg\":\"日本語🚀\"}".as_bytes()));
        wire.extend_from_slice(&encode_framed_message(b"{\"n\":3}"));

        let mut reader = BufReader::new(&wire[..]);
        let mut buf = Vec::new();

        let m1 = read_framed_message(&mut reader, &mut buf).await.unwrap().unwrap();
        assert_eq!(m1, b"{}");
        let m2 = read_framed_message(&mut reader, &mut buf).await.unwrap().unwrap();
        assert_eq!(m2, "{\"msg\":\"日本語🚀\"}".as_bytes());
        let m3 = read_framed_message(&mut reader, &mut buf).await.unwrap().unwrap();
        assert_eq!(m3, b"{\"n\":3}");
        assert!(read_framed_message(&mut reader, &mut buf).await.unwrap().is_none());
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn test_header_too_large_rejected_without_unbounded_allocation() {
        let oversized = vec![b'A'; MAX_HEADER_BYTES + 128];
        let mut reader = ChunkedReader {
            data: oversized.clone(),
            pos: 0,
            chunk_size: 4096,
        };
        let mut buf = Vec::new();
        let err = read_framed_message(&mut reader, &mut buf).await.unwrap_err();
        assert!(matches!(err, CodecError::HeaderTooLarge(_)));

        let mut sync_buf = oversized;
        let sync_err = decode_framed_message(&mut sync_buf).unwrap_err();
        assert!(matches!(sync_err, CodecError::HeaderTooLarge(_)));
    }

    #[tokio::test]
    async fn test_frame_too_large_rejected_before_body_allocation() {
        // 64 MiB + 1 must be rejected immediately even without body bytes present
        let raw_over = format!("Content-Length: {}\r\n\r\n", MAX_FRAME_BYTES + 1);
        let mut cursor = raw_over.as_bytes();
        let mut buf = Vec::new();
        let err = read_framed_message(&mut cursor, &mut buf).await.unwrap_err();
        assert!(matches!(err, CodecError::FrameTooLarge { .. }));

        let mut sync_buf = raw_over.into_bytes();
        let sync_err = decode_framed_message(&mut sync_buf).unwrap_err();
        assert!(matches!(sync_err, CodecError::FrameTooLarge { .. }));

        // Exact MAX_FRAME_BYTES (64 MiB) must be accepted by header validation
        let exact_header = format!("Content-Length: {}", MAX_FRAME_BYTES);
        assert_eq!(
            parse_content_length_header(exact_header.as_bytes()).unwrap(),
            MAX_FRAME_BYTES
        );
    }

    #[tokio::test]
    async fn test_incremental_header_scan_and_split_terminators() {
        // ~60 KiB header delivered 17 bytes per AsyncRead poll
        let payload = b"{\"ok\":true}";
        let pad_len = 60 * 1024;
        let mut wire = format!("X-Pad: {}\r\nContent-Length: {}\r\n\r\n", "x".repeat(pad_len), payload.len()).into_bytes();
        wire.extend_from_slice(payload);

        let mut reader = ChunkedReader {
            data: wire,
            pos: 0,
            chunk_size: 17,
        };
        let mut buf = Vec::new();
        let decoded = read_framed_message(&mut reader, &mut buf).await.unwrap().unwrap();
        assert_eq!(decoded, payload);

        // Split \r\n|\r\n across 2-byte reads
        let split_wire = b"\r\nContent-Length: 4\r\n\r\nping".to_vec();
        let mut split_reader = ChunkedReader {
            data: split_wire,
            pos: 0,
            chunk_size: 2,
        };
        let mut split_buf = Vec::new();
        let decoded2 = read_framed_message(&mut split_reader, &mut split_buf).await.unwrap().unwrap();
        assert_eq!(decoded2, b"ping");

        // Split \n|\n across 1-byte reads
        let lf_wire = b"Content-Length: 3\n\nbye".to_vec();
        let mut lf_reader = ChunkedReader {
            data: lf_wire,
            pos: 0,
            chunk_size: 1,
        };
        let mut lf_buf = Vec::new();
        let decoded3 = read_framed_message(&mut lf_reader, &mut lf_buf).await.unwrap().unwrap();
        assert_eq!(decoded3, b"bye");
    }

    #[tokio::test]
    async fn test_zero_content_length_and_mid_frame_eof() {
        let mut zero_wire = b"Content-Length: 0\r\n\r\n".as_slice();
        let mut buf = Vec::new();
        let empty = read_framed_message(&mut zero_wire, &mut buf).await.unwrap().unwrap();
        assert!(empty.is_empty());

        let mut truncated = b"Content-Length: 10\r\n\r\nhello".as_slice();
        let mut tbuf = Vec::new();
        let err = read_framed_message(&mut truncated, &mut tbuf).await.unwrap_err();
        assert!(matches!(
            err,
            CodecError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::UnexpectedEof
        ));
    }

    #[tokio::test]
    async fn test_async_rejects_duplicate_and_plus_content_length() {
        let mut dup_wire = b"Content-Length: 2\r\nContent-Length: 999\r\n\r\n{}".as_slice();
        let mut buf = Vec::new();
        let err = read_framed_message(&mut dup_wire, &mut buf).await.unwrap_err();
        assert!(matches!(err, CodecError::InvalidHeader(_)));

        let mut plus_wire = b"Content-Length: +5\r\n\r\nhello".as_slice();
        let mut pbuf = Vec::new();
        let err = read_framed_message(&mut plus_wire, &mut pbuf).await.unwrap_err();
        assert!(matches!(err, CodecError::InvalidContentLength(_)));
    }
}
