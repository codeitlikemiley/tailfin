//! WebSocket passthrough for Codex `/v1/responses`.
//!
//! The relay copies bytes; it does not re-frame. Server-to-client text frames
//! are scanned for Responses usage so the ledger still meters when Codex stays
//! on the WebSocket instead of falling back to HTTPS SSE.

use hyper::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::upgrade::Upgraded;
use hyper::StatusCode;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Encode an unmasked text frame (RFC 6455).
#[cfg(test)]
pub fn encode_text_frame(payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(14 + payload.len());
    f.push(0x81);
    let n = payload.len();
    if n < 126 {
        f.push(n as u8);
    } else if n <= u16::MAX as usize {
        f.push(126);
        f.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        f.push(127);
        f.extend_from_slice(&(n as u64).to_be_bytes());
    }
    f.extend_from_slice(payload);
    f
}

#[derive(Default)]
pub struct WsTextDecoder {
    buf: Vec<u8>,
}

impl WsTextDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 2 {
                break;
            }
            let b0 = self.buf[0];
            let b1 = self.buf[1];
            let opcode = b0 & 0x0f;
            let masked = b1 & 0x80 != 0;
            let mut len = (b1 & 0x7f) as usize;
            let mut hdr = 2usize;
            if len == 126 {
                if self.buf.len() < 4 {
                    break;
                }
                len = u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize;
                hdr = 4;
            } else if len == 127 {
                if self.buf.len() < 10 {
                    break;
                }
                let mut n = [0u8; 8];
                n.copy_from_slice(&self.buf[2..10]);
                len = u64::from_be_bytes(n) as usize;
                hdr = 10;
            }
            let mask_len = if masked { 4 } else { 0 };
            if self.buf.len() < hdr + mask_len + len {
                break;
            }
            let mut payload = self.buf[hdr + mask_len..hdr + mask_len + len].to_vec();
            if masked {
                let mask = [
                    self.buf[hdr],
                    self.buf[hdr + 1],
                    self.buf[hdr + 2],
                    self.buf[hdr + 3],
                ];
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }
            self.buf.drain(..hdr + mask_len + len);
            match opcode {
                0x1 | 0x2 => out.push(payload),
                0x8 => return out,
                _ => {}
            }
        }
        out
    }
}

/// HTTP (not TLS) WebSocket handshake. The hyper-util Client does not perform
/// upgrades, which is why Codex saw `426 Upgrade Required`.
pub async fn http_handshake(
    host: &str,
    port: u16,
    path: &str,
    headers: &HeaderMap,
) -> Result<(TcpStream, StatusCode, HeaderMap), Box<dyn std::error::Error + Send + Sync>> {
    let mut tcp = TcpStream::connect((host, port)).await?;
    let host_hdr = if port == 80 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: {host_hdr}\r\n");
    for (name, value) in headers.iter() {
        let n = name.as_str();
        if n.eq_ignore_ascii_case("host") || n.eq_ignore_ascii_case("content-length") {
            continue;
        }
        if let Ok(v) = value.to_str() {
            req.push_str(n);
            req.push_str(": ");
            req.push_str(v);
            req.push_str("\r\n");
        }
    }
    if headers.get("connection").is_none() {
        req.push_str("Connection: Upgrade\r\n");
    }
    if headers.get("upgrade").is_none() {
        req.push_str("Upgrade: websocket\r\n");
    }
    req.push_str("\r\n");
    tcp.write_all(req.as_bytes()).await?;

    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        tcp.read_exact(&mut byte).await?;
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        if head.len() > 64 * 1024 {
            return Err("websocket handshake header too large".into());
        }
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(500);
    let status = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp_headers = HeaderMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(k.trim().as_bytes()),
                HeaderValue::from_str(v.trim()),
            ) {
                resp_headers.append(name, val);
            }
        }
    }
    Ok((tcp, status, resp_headers))
}

pub async fn splice_tcp(
    client: Upgraded,
    mut upstream: TcpStream,
    mut on_upstream: impl FnMut(&[u8]),
) {
    let mut client = TokioIo::new(client);
    let mut cbuf = [0u8; 16 * 1024];
    let mut ubuf = [0u8; 16 * 1024];
    loop {
        tokio::select! {
            n = client.read(&mut cbuf) => {
                let n = match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if upstream.write_all(&cbuf[..n]).await.is_err() {
                    break;
                }
            }
            n = upstream.read(&mut ubuf) => {
                let n = match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                on_upstream(&ubuf[..n]);
                if client.write_all(&ubuf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
}

pub async fn splice(client: Upgraded, upstream: Upgraded, mut on_upstream: impl FnMut(&[u8])) {
    let mut client = TokioIo::new(client);
    let mut upstream = TokioIo::new(upstream);
    let mut cbuf = [0u8; 16 * 1024];
    let mut ubuf = [0u8; 16 * 1024];
    loop {
        tokio::select! {
            n = client.read(&mut cbuf) => {
                let n = match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if upstream.write_all(&cbuf[..n]).await.is_err() {
                    break;
                }
            }
            n = upstream.read(&mut ubuf) => {
                let n = match n {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                on_upstream(&ubuf[..n]);
                if client.write_all(&ubuf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_frame_round_trips() {
        let mut d = WsTextDecoder::default();
        let payload = br#"{"type":"response.completed"}"#;
        let frames = d.push(&encode_text_frame(payload));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], payload);
    }

    #[test]
    fn split_header_still_decodes() {
        let mut d = WsTextDecoder::default();
        let f = encode_text_frame(b"hi");
        assert!(d.push(&f[..1]).is_empty());
        let got = d.push(&f[1..]);
        assert_eq!(got, vec![b"hi".to_vec()]);
    }
}
