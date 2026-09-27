//! The network itself, with a thread per request and per socket reporting through a channel.

use std::collections::HashMap;
use std::io::Read;
use std::net::TcpStream;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::Duration;

use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use super::{closed, message, response, send_failure, send_success, status_response, Backend};
use crate::events::Event;

/// `requestBuilder.timeout(Duration.ofSeconds(3))`.
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a socket thread waits for a frame before looking at its command queue.
const POLL: Duration = Duration::from_millis(50);
/// A ceiling on a response body.
const MAX_BODY: u64 = 8 * 1024 * 1024;

enum Command {
    Send(Vec<u8>, bool),
    Close,
}

pub struct Live {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    sockets: HashMap<i32, Sender<Command>>,
}

impl Default for Live {
    fn default() -> Live {
        Live::new()
    }
}

impl Live {
    pub fn new() -> Live {
        let (tx, rx) = channel();
        Live {
            tx,
            rx,
            sockets: HashMap::new(),
        }
    }
}

impl Backend for Live {
    fn get(&mut self, id: i32, url: &str, headers: Vec<(String, String)>) {
        spawn_request(self.tx.clone(), id, "GET", url.to_string(), headers, None);
    }

    fn post(&mut self, id: i32, url: &str, headers: Vec<(String, String)>, body: Vec<u8>) {
        spawn_request(
            self.tx.clone(),
            id,
            "POST",
            url.to_string(),
            headers,
            Some(body),
        );
    }

    fn open_socket(
        &mut self,
        id: i32,
        url: &str,
        headers: Vec<(String, String)>,
    ) -> Result<(), String> {
        let (commands, inbox) = channel();
        let tx = self.tx.clone();
        let url = url.to_string();
        std::thread::spawn(move || socket_thread(tx, id, url, headers, inbox));
        self.sockets.insert(id, commands);
        Ok(())
    }

    fn send(&mut self, id: i32, data: Vec<u8>, binary: bool) {
        match self.sockets.get(&id) {
            Some(commands) => {
                if commands.send(Command::Send(data, binary)).is_err() {
                    let _ = self.tx.send(send_failure(id, "Line closed"));
                }
            }
            None => {
                let _ = self.tx.send(send_failure(id, "Line closed"));
            }
        }
    }

    fn close(&mut self, id: i32) -> Result<(), String> {
        match self.sockets.get(&id) {
            Some(commands) => {
                let _ = commands.send(Command::Close);
                Ok(())
            }
            None => Err("Already closed".into()),
        }
    }

    fn drain(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            // A closed socket stops counting against the cap.
            if event.name == "WebsocketClosed" {
                if let Some(crate::events::Value::Int(id)) = event.args.first() {
                    self.sockets.remove(&(*id as i32));
                }
            }
            events.push(event);
        }
        events
    }

    fn reset(&mut self) {
        for commands in self.sockets.values() {
            let _ = commands.send(Command::Close);
        }
        self.sockets.clear();
        while self.rx.try_recv().is_ok() {}
    }

    fn live_sockets(&self) -> usize {
        self.sockets.len()
    }
}

fn spawn_request(
    tx: Sender<Event>,
    id: i32,
    method: &'static str,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
) {
    std::thread::spawn(move || {
        let mut request = ureq::request(method, &url).timeout(HTTP_TIMEOUT);
        for (name, value) in &headers {
            request = request.set(name, value);
        }
        let result = match &body {
            Some(body) => request.send_bytes(body),
            None => request.call(),
        };
        let event = match result {
            Ok(response) => collect(id, response),
            // A 4xx or 5xx is a response, not a transport failure.
            Err(ureq::Error::Status(_, response)) => collect(id, response),
            Err(ureq::Error::Transport(transport)) => {
                let (code, message) = transport_failure(&transport);
                response(id, code, message, Vec::new(), Vec::new())
            }
        };
        let _ = tx.send(event);
    });
}

fn collect(id: i32, response: ureq::Response) -> Event {
    let code = response.status();
    let headers = response
        .headers_names()
        .into_iter()
        .filter_map(|name| {
            response
                .header(&name)
                .map(|v| (name.clone(), v.to_string()))
        })
        .collect();
    let mut body = Vec::new();
    let _ = response.into_reader().take(MAX_BODY).read_to_end(&mut body);
    status_response(id, code, headers, body)
}

/// `httpErrorHandler`, which answers with an errno rather than a status.
fn transport_failure(transport: &ureq::Transport) -> (i32, &'static str) {
    let timed_out = transport.to_string().to_lowercase().contains("timed out");
    match transport.kind() {
        _ if timed_out => (110, "ETIMEDOUT"),
        ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed => (404, "NOT FOUND"),
        ureq::ErrorKind::InvalidUrl
        | ureq::ErrorKind::UnknownScheme
        | ureq::ErrorKind::BadHeader
        | ureq::ErrorKind::Io => (400, "BAD REQUEST"),
        _ => (0, "UNIDENTIFIED EXCEPTION"),
    }
}

fn socket_thread(
    tx: Sender<Event>,
    id: i32,
    url: String,
    headers: Vec<(String, String)>,
    inbox: Receiver<Command>,
) {
    let mut socket = match connect(&url, &headers) {
        Ok(socket) => socket,
        Err(_) => {
            let _ = tx.send(closed(id, 1006, "Abnormal Closure"));
            return;
        }
    };
    set_read_timeout(&socket, POLL);
    let _ = tx.send(super::opened(id));

    loop {
        loop {
            match inbox.try_recv() {
                Ok(Command::Send(data, binary)) => {
                    let frame = match binary {
                        true => Message::Binary(data.into()),
                        false => Message::Text(String::from_utf8_lossy(&data).into_owned().into()),
                    };
                    let event = match socket.send(frame) {
                        Ok(()) => send_success(id),
                        Err(_) => send_failure(id, "Line closed"),
                    };
                    let _ = tx.send(event);
                }
                Ok(Command::Close) => {
                    let _ = socket.close(None);
                    let _ = socket.flush();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    break;
                }
            }
        }

        match socket.read() {
            Ok(Message::Text(text)) => {
                let _ = tx.send(message(id, text.as_bytes().to_vec(), false));
            }
            Ok(Message::Binary(data)) => {
                let _ = tx.send(message(id, data.to_vec(), true));
            }
            Ok(Message::Close(frame)) => {
                let (code, reason) = match frame {
                    Some(frame) => (u16::from(frame.code) as i32, frame.reason.to_string()),
                    None => (1000, "Normal Closure".to_string()),
                };
                let _ = tx.send(closed(id, code, &reason));
                return;
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if would_block(&error) => {}
            Err(tungstenite::Error::ConnectionClosed) => {
                let _ = tx.send(closed(id, 1000, "Normal Closure"));
                return;
            }
            Err(_) => {
                let _ = tx.send(closed(id, 1006, "Abnormal Closure"));
                return;
            }
        }
    }
}

fn connect(
    url: &str,
    headers: &[(String, String)],
) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    for (name, value) in headers {
        let name = tungstenite::http::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| e.to_string())?;
        let value =
            tungstenite::http::header::HeaderValue::from_str(value).map_err(|e| e.to_string())?;
        request.headers_mut().insert(name, value);
    }
    let (socket, _) = tungstenite::connect(request).map_err(|e| e.to_string())?;
    Ok(socket)
}

/// A read timeout is what lets one thread serve both directions.
fn set_read_timeout(socket: &WebSocket<MaybeTlsStream<TcpStream>>, timeout: Duration) {
    match socket.get_ref() {
        MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(Some(timeout));
        }
        MaybeTlsStream::Rustls(stream) => {
            let _ = stream.get_ref().set_read_timeout(Some(timeout));
        }
        _ => {}
    }
}

fn would_block(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
    )
}
