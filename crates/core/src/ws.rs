//! WebSocket client with an event log (polled by the CLI and the GUI).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WsEvent {
    pub time: DateTime<Utc>,
    /// `open`, `sent`, `message`, `binary`, `ping`, `pong`, `close`, `error`
    pub kind: String,
    pub data: String,
}

enum Cmd {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Close,
}

pub struct WsClient {
    tx: mpsc::Sender<Cmd>,
    events: Arc<Mutex<Vec<WsEvent>>>,
    open: Arc<AtomicBool>,
    pub url: String,
}

fn push(events: &Arc<Mutex<Vec<WsEvent>>>, kind: &str, data: impl Into<String>) {
    if let Ok(mut e) = events.lock() {
        e.push(WsEvent { time: Utc::now(), kind: kind.into(), data: data.into() });
    }
}

impl WsClient {
    /// Connect to a `ws://` or `wss://` URL with optional extra handshake headers.
    pub fn connect(url: &str, headers: &[(String, String)]) -> Result<WsClient> {
        let mut req = url.into_client_request().map_err(|e| anyhow!("invalid WebSocket URL `{url}`: {e}"))?;
        for (k, v) in headers {
            let name = tungstenite::http::HeaderName::from_bytes(k.as_bytes()).map_err(|_| anyhow!("invalid header name `{k}`"))?;
            let value = tungstenite::http::HeaderValue::from_str(v).map_err(|_| anyhow!("invalid value for header `{k}`"))?;
            req.headers_mut().insert(name, value);
        }
        let (mut socket, response) = tungstenite::connect(req).map_err(|e| anyhow!("WebSocket connection failed: {e}"))?;
        let timeout = Some(Duration::from_millis(50));
        match socket.get_mut() {
            MaybeTlsStream::Plain(s) => s.set_read_timeout(timeout)?,
            MaybeTlsStream::Rustls(s) => s.get_mut().set_read_timeout(timeout)?,
            _ => {}
        }
        let events = Arc::new(Mutex::new(vec![]));
        let open = Arc::new(AtomicBool::new(true));
        push(&events, "open", format!("connected to {url} (HTTP {})", response.status().as_u16()));
        let (tx, rx) = mpsc::channel::<Cmd>();
        let (ev, op) = (events.clone(), open.clone());
        std::thread::spawn(move || {
            let mut closing = false;
            loop {
                loop {
                    match rx.try_recv() {
                        Ok(Cmd::Text(t)) => match socket.send(Message::Text(t.clone())) {
                            Ok(_) => push(&ev, "sent", t),
                            Err(e) => push(&ev, "error", format!("send failed: {e}")),
                        },
                        Ok(Cmd::Binary(b)) => {
                            let n = b.len();
                            match socket.send(Message::Binary(b)) {
                                Ok(_) => push(&ev, "sent", format!("<binary, {n} bytes>")),
                                Err(e) => push(&ev, "error", format!("send failed: {e}")),
                            }
                        }
                        Ok(Cmd::Ping) => {
                            let _ = socket.send(Message::Ping(vec![]));
                            push(&ev, "ping", "");
                        }
                        Ok(Cmd::Close) => {
                            let _ = socket.close(None);
                            closing = true;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            let _ = socket.close(None);
                            closing = true;
                            break;
                        }
                    }
                }
                match socket.read() {
                    Ok(Message::Text(t)) => push(&ev, "message", t),
                    Ok(Message::Binary(b)) => push(&ev, "binary", format!("<binary, {} bytes> {}", b.len(), hex::encode(&b[..b.len().min(64)]))),
                    Ok(Message::Ping(_)) => push(&ev, "ping", "received ping"),
                    Ok(Message::Pong(_)) => push(&ev, "pong", "received pong"),
                    Ok(Message::Close(frame)) => {
                        push(&ev, "close", frame.map(|f| format!("{} {}", u16::from(f.code), f.reason)).unwrap_or_else(|| "connection closed".into()));
                        break;
                    }
                    Ok(Message::Frame(_)) => {}
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                        if closing {
                            // give the peer a moment to answer the close frame, then stop
                            std::thread::sleep(Duration::from_millis(100));
                            if !matches!(socket.read(), Ok(Message::Close(_))) {
                                push(&ev, "close", "connection closed");
                            } else {
                                push(&ev, "close", "connection closed by client");
                            }
                            break;
                        }
                    }
                    Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                        push(&ev, "close", "connection closed");
                        break;
                    }
                    Err(_) if closing => {
                        // the peer dropped the connection after our close frame
                        push(&ev, "close", "connection closed");
                        break;
                    }
                    Err(e) => {
                        push(&ev, "error", e.to_string());
                        break;
                    }
                }
            }
            op.store(false, Ordering::SeqCst);
        });
        Ok(WsClient { tx, events, open, url: url.to_string() })
    }

    pub fn send(&self, text: &str) -> Result<()> {
        self.tx.send(Cmd::Text(text.to_string())).map_err(|_| anyhow!("WebSocket is closed"))
    }
    pub fn send_binary(&self, data: Vec<u8>) -> Result<()> {
        self.tx.send(Cmd::Binary(data)).map_err(|_| anyhow!("WebSocket is closed"))
    }
    pub fn ping(&self) -> Result<()> {
        self.tx.send(Cmd::Ping).map_err(|_| anyhow!("WebSocket is closed"))
    }
    pub fn close(&self) {
        let _ = self.tx.send(Cmd::Close);
    }
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
    /// Events starting at index `from`.
    pub fn events_since(&self, from: usize) -> Vec<WsEvent> {
        self.events.lock().map(|e| e.iter().skip(from).cloned().collect()).unwrap_or_default()
    }
    /// Block until at least `count` events exist or the timeout elapses.
    pub fn wait_for(&self, count: usize, timeout: Duration) -> Vec<WsEvent> {
        let end = std::time::Instant::now() + timeout;
        loop {
            let ev = self.events_since(0);
            if ev.len() >= count || std::time::Instant::now() >= end {
                return ev;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for WsClient {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_roundtrip() {
        let srv = crate::echo::start("127.0.0.1", 0).unwrap();
        let url = format!("ws://127.0.0.1:{}/ws", srv.port);
        let ws = WsClient::connect(&url, &[("X-Test".into(), "1".into())]).unwrap();
        ws.send("hello").unwrap();
        ws.send_binary(vec![1, 2, 3]).unwrap();
        let ev = ws.wait_for(6, Duration::from_secs(5));
        let kinds: Vec<&str> = ev.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds[0], "open");
        assert!(ev.iter().any(|e| e.kind == "message" && e.data.contains("connected")), "{ev:?}");
        assert!(ev.iter().any(|e| e.kind == "message" && e.data == "hello"), "{ev:?}");
        assert!(ev.iter().any(|e| e.kind == "binary" && e.data.contains("010203")), "{ev:?}");
        ws.close();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while ws.is_open() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!ws.is_open());
        assert!(ws.send("late").is_err() || ws.events_since(0).iter().any(|e| e.kind == "close"));
        assert!(WsClient::connect("ws://127.0.0.1:1/ws", &[]).is_err());
        assert!(WsClient::connect("not a url", &[]).is_err());
    }
}
