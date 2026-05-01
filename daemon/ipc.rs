// IPC framing + message types.
//
// Wire format (per docs/DAEMON.md "IPC wire format"):
//   [4 bytes: u32 BE length] [length bytes: UTF-8 JSON]
//
// Envelopes:
//   client → daemon (handshake) : { "hello":   { ... } }
//   daemon → client (handshake) : { "welcome": { ... } | "err": { ... } }
//   client → daemon (request)   : { "id": u64, "op": str, "args": {...} }
//   daemon → client (response)  : { "id": u64, "ok": bool, [payload...] | "err": {...} }
//   daemon → client (async evt) : { "event": str, [payload...] }
//
// JSON-tagged via serde's untagged-enum dispatch (kind detected by which top-level key
// exists). This keeps every message a flat object on the wire, debuggable with `socat`.

use std::io;

use byteorder::{BigEndian, ByteOrder};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{DaemonError, Result};

/// Daemon's protocol version — bumped on incompatible wire changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest single frame the daemon will accept. Guards against runaway client.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion(pub u32);

/// First message client sends after connect.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub version: u32,
    pub client_pid: i32,
    pub tty: Option<String>,
    pub cwd: Option<String>,
    pub argv0: Option<String>,
}

/// Daemon's response to a successful Hello.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Welcome {
    pub version: u32,
    pub client_id: u64,
    pub session_id: String,
    pub daemon_pid: i32,
    pub daemon_uptime_ms: u64,
}

/// Error payload — paired with `ok: false` on response, or `err: {...}` on welcome failure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrPayload {
    pub code: String,
    pub msg: String,
}

impl ErrPayload {
    pub fn new<C: Into<String>, M: Into<String>>(code: C, msg: M) -> Self {
        Self {
            code: code.into(),
            msg: msg.into(),
        }
    }
}

impl From<rusqlite::Error> for ErrPayload {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("sqlite", e.to_string())
    }
}

impl From<std::io::Error> for ErrPayload {
    fn from(e: std::io::Error) -> Self {
        Self::new("io", e.to_string())
    }
}

impl From<super::DaemonError> for ErrPayload {
    fn from(e: super::DaemonError) -> Self {
        Self::new("daemon", e.to_string())
    }
}

/// Top-level frame envelope. One of these JSON objects per length-prefixed frame.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Frame {
    Hello {
        hello: Hello,
    },
    Welcome {
        welcome: Welcome,
    },
    WelcomeErr {
        welcome: serde_json::Value,
        err: ErrPayload,
    },
    Request {
        id: u64,
        op: String,
        #[serde(default)]
        args: serde_json::Value,
    },
    Response {
        id: u64,
        ok: bool,
        #[serde(flatten)]
        payload: serde_json::Value,
    },
    Event {
        event: String,
        #[serde(flatten)]
        payload: serde_json::Value,
    },
}

impl Frame {
    pub fn hello(h: Hello) -> Self {
        Frame::Hello { hello: h }
    }
    pub fn welcome(w: Welcome) -> Self {
        Frame::Welcome { welcome: w }
    }
    pub fn request(id: u64, op: impl Into<String>, args: serde_json::Value) -> Self {
        Frame::Request {
            id,
            op: op.into(),
            args,
        }
    }
    pub fn ok_response(id: u64, payload: serde_json::Value) -> Self {
        Frame::Response {
            id,
            ok: true,
            payload,
        }
    }
    pub fn err_response(id: u64, err: ErrPayload) -> Self {
        let payload = serde_json::json!({ "err": err });
        Frame::Response {
            id,
            ok: false,
            payload,
        }
    }
    pub fn event(name: impl Into<String>, payload: serde_json::Value) -> Self {
        Frame::Event {
            event: name.into(),
            payload,
        }
    }
}

/// Async event types pushed daemon → client. Names match the doc's event table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    ShardUpdated,
    RebuildComplete,
    CanonicalChanged,
    Match,
    CmdExecute,
    Notify,
    DaemonShutdown,
    AskPending,
    AskDismissed,
    AskProgress,
    LongCmdComplete,
    LongCmdStarted,
    LongCmdFailed,
    LongCmdSignaled,
}

// -------- Wire framing helpers --------

/// Read one length-prefixed frame from the socket.
pub async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Frame> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = BigEndian::read_u32(&len_buf) as usize;
    if len == 0 {
        return Err(DaemonError::other("zero-length frame"));
    }
    if len > MAX_FRAME_BYTES {
        return Err(DaemonError::FrameTooLarge {
            size: len,
            max: MAX_FRAME_BYTES,
        });
    }

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;

    let frame: Frame = serde_json::from_slice(&buf)?;
    Ok(frame)
}

/// Write one length-prefixed frame to the socket.
pub async fn write_frame<W: AsyncWriteExt + Unpin>(writer: &mut W, frame: &Frame) -> Result<()> {
    let body = serde_json::to_vec(frame)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(DaemonError::FrameTooLarge {
            size: body.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    let mut header = [0u8; 4];
    BigEndian::write_u32(&mut header, body.len() as u32);
    writer.write_all(&header).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

/// Synchronous variant for client-side blocking IPC (used from non-async builtins).
pub fn read_frame_sync<R: io::Read>(reader: &mut R) -> Result<Frame> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = BigEndian::read_u32(&len_buf) as usize;
    if len == 0 {
        return Err(DaemonError::other("zero-length frame"));
    }
    if len > MAX_FRAME_BYTES {
        return Err(DaemonError::FrameTooLarge {
            size: len,
            max: MAX_FRAME_BYTES,
        });
    }

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;

    let frame: Frame = serde_json::from_slice(&buf)?;
    Ok(frame)
}

/// Synchronous frame write (client side).
pub fn write_frame_sync<W: io::Write>(writer: &mut W, frame: &Frame) -> Result<()> {
    let body = serde_json::to_vec(frame)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(DaemonError::FrameTooLarge {
            size: body.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    let mut header = [0u8; 4];
    BigEndian::write_u32(&mut header, body.len() as u32);
    writer.write_all(&header)?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip_hello_sync() {
        let h = Hello {
            version: PROTOCOL_VERSION,
            client_pid: 12345,
            tty: Some("/dev/ttys003".into()),
            cwd: Some("/home/wizard".into()),
            argv0: Some("zshrs".into()),
        };
        let frame = Frame::hello(h);

        let mut buf = Vec::new();
        write_frame_sync(&mut buf, &frame).unwrap();

        let mut cur = Cursor::new(buf);
        let read = read_frame_sync(&mut cur).unwrap();

        match read {
            Frame::Hello { hello } => {
                assert_eq!(hello.version, PROTOCOL_VERSION);
                assert_eq!(hello.client_pid, 12345);
                assert_eq!(hello.tty.as_deref(), Some("/dev/ttys003"));
            }
            _ => panic!("expected Hello, got {:?}", read),
        }
    }

    #[test]
    fn roundtrip_request_sync() {
        let frame = Frame::request(42, "ping", serde_json::json!({}));
        let mut buf = Vec::new();
        write_frame_sync(&mut buf, &frame).unwrap();

        let mut cur = Cursor::new(buf);
        let read = read_frame_sync(&mut cur).unwrap();

        match read {
            Frame::Request { id, op, args } => {
                assert_eq!(id, 42);
                assert_eq!(op, "ping");
                assert!(args.is_object());
            }
            _ => panic!("expected Request, got {:?}", read),
        }
    }

    #[test]
    fn roundtrip_event_sync() {
        let frame = Frame::event(
            "shard_updated",
            serde_json::json!({"shard":"foo","generation":3}),
        );
        let mut buf = Vec::new();
        write_frame_sync(&mut buf, &frame).unwrap();

        let mut cur = Cursor::new(buf);
        let read = read_frame_sync(&mut cur).unwrap();

        match read {
            Frame::Event { event, payload } => {
                assert_eq!(event, "shard_updated");
                assert_eq!(payload["shard"], "foo");
                assert_eq!(payload["generation"], 3);
            }
            _ => panic!("expected Event, got {:?}", read),
        }
    }

    #[test]
    fn frame_too_large_rejected_on_write() {
        let big = "x".repeat(MAX_FRAME_BYTES + 1);
        let frame = Frame::request(1, "ping", serde_json::json!({"big": big}));
        let mut buf = Vec::new();
        let err = write_frame_sync(&mut buf, &frame).unwrap_err();
        matches!(err, DaemonError::FrameTooLarge { .. });
    }

    #[test]
    fn frame_too_large_rejected_on_read() {
        let mut buf = Vec::new();
        let bogus_len = (MAX_FRAME_BYTES + 1) as u32;
        let mut hdr = [0u8; 4];
        BigEndian::write_u32(&mut hdr, bogus_len);
        buf.extend_from_slice(&hdr);
        let mut cur = Cursor::new(buf);
        let err = read_frame_sync(&mut cur).unwrap_err();
        matches!(err, DaemonError::FrameTooLarge { .. });
    }

    #[tokio::test]
    async fn roundtrip_async() {
        let frame = Frame::request(7, "info", serde_json::json!({}));
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let writer_frame = frame.clone();
        tokio::spawn(async move {
            write_frame(&mut a, &writer_frame).await.unwrap();
        });
        let read = read_frame(&mut b).await.unwrap();
        match read {
            Frame::Request { id, op, .. } => {
                assert_eq!(id, 7);
                assert_eq!(op, "info");
            }
            _ => panic!("expected Request"),
        }
    }
}
