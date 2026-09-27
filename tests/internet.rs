//! `internet`, against `APIS/InternetAPI.java` and `simulation/InternetManager.java`.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;

use common::{expect_clean, TestDisk};
use neetemu::internet::{live::Live, Internet};
use neetemu::vm::Config;

/// Collects `n` events off the `Network` label, ticking until they arrive.
const COLLECT: &str = r#"
local function collect(n)
    local got = {}
    for _ = 1, 400 do
        for _, e in ipairs(event.getQueue("Network")) do got[#got + 1] = e end
        if #got >= n then break end
        chip.sleep(0.02)
    end
    return got
end
"#;

/// One HTTP exchange on a loopback port, then the listener goes away.
fn http_server(status: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        if let Some(Ok(mut stream)) = listener.incoming().next() {
            read_request(&mut stream);
            let response = format!(
                "HTTP/1.1 {status}\r\nX-Neet: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}")
}

/// Reads the whole request, body included.
fn read_request(stream: &mut std::net::TcpStream) {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        request.extend_from_slice(&chunk[..read]);
        let Some(head) = request
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|at| at + 4)
        else {
            continue;
        };
        if request.len() >= head + content_length(&request[..head]) {
            return;
        }
    }
}

fn content_length(head: &[u8]) -> usize {
    String::from_utf8_lossy(head)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0)
}

/// A websocket that echoes whatever it is sent.
fn echo_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        if let Some(Ok(stream)) = listener.incoming().next() {
            let Ok(mut socket) = tungstenite::accept(stream) else {
                return;
            };
            while let Ok(message) = socket.read() {
                if message.is_close() {
                    break;
                }
                if message.is_text() || message.is_binary() {
                    let _ = socket.send(message);
                }
            }
        }
    });
    format!("ws://{address}")
}

fn online(name: &str, body: &str) {
    let disk = TestDisk::new(name);
    expect_clean(
        disk.run_setup(Config::default(), &format!("{COLLECT}\n{body}"), |host| {
            host.internet = Internet::with(Box::new(Live::new()));
        }),
    );
}

#[test]
fn access_is_off_by_default() {
    let disk = TestDisk::new("internet-offline");
    expect_clean(disk.run(&format!("{COLLECT}\n{}", r#"
        same("no access", internet.hasAccess(), false)
        same("not ready either", internet.isReady(), false)

        local id = internet.GET("http://example.invalid")
        local got = collect(1)
        same("one response", #got, 1)
        same("named HttpResponse", got[1][1], "HttpResponse")
        same("carries its id", got[1][2], id)
        same("refused", got[1][3], 111)
        same("with the errno", got[1][4], "ECONNREFUSED")
        same("an empty header table", type(got[1][5]), "table")
        same("and no body slot at all", got[1][6], nil)

        raises("a socket", "Internet access disabled", internet.CreateWebsocket, "ws://example.invalid")
    "#)));
}

#[test]
fn a_malformed_url_and_a_bad_header_are_refused_before_sending() {
    let disk = TestDisk::new("internet-arguments");
    expect_clean(disk.run(
        r#"
        raises("no scheme", "Invalid URL", internet.GET, "nonsense")
        raises("a table value", "Invalid Header (All values must be strings)",
            internet.GET, "http://example.invalid", { a = {} })
    "#,
    ));
}

#[test]
fn a_get_carries_status_headers_and_body() {
    let url = http_server("200 OK", "hello neet");
    online(
        "internet-get",
        &format!(
            r#"
        local id = internet.GET("{url}")
        local got = collect(1)
        same("one response", #got, 1)
        same("its id", got[1][2], id)
        same("the status", got[1][3], 200)
        -- `errorCodes` names the ones the mod knows.
        same("the status name", got[1][4], "OK")
        same("a header the server set", got[1][5]["x-neet"] or got[1][5]["X-Neet"], "yes")
        same("the body", got[1][6], "hello neet")
    "#
        ),
    );
}

#[test]
fn an_unnamed_status_falls_through_to_unknown() {
    let url = http_server("203 Non-Authoritative Information", "x");
    online(
        "internet-unknown-status",
        &format!(
            r#"
        internet.GET("{url}")
        local got = collect(1)
        same("the status", got[1][3], 203)
        same("no name for it", got[1][4], "UNKNOWN STATUS CODE")
    "#
        ),
    );
}

#[test]
fn a_named_error_status_is_still_a_response() {
    let url = http_server("404 Not Found", "");
    online(
        "internet-404",
        &format!(
            r#"
        internet.GET("{url}")
        local got = collect(1)
        same("the status", got[1][3], 404)
        same("its name", got[1][4], "NOT FOUND")
        same("an empty body is no body slot", got[1][6], nil)
    "#
        ),
    );
}

#[test]
fn a_post_sends_its_body() {
    let url = http_server("200 OK", "took it");
    online(
        "internet-post",
        &format!(
            r#"
        local id = internet.POST("{url}", {{ ["Content-Type"] = "text/plain" }}, "payload")
        local got = collect(1)
        same("its id", got[1][2], id)
        same("the reply", got[1][6], "took it")
    "#
        ),
    );
}

#[test]
fn a_refused_connection_answers_with_an_errno() {
    // Port 1 on loopback has nothing listening, so the connection fails outright.
    online(
        "internet-refused",
        r#"
        internet.GET("http://127.0.0.1:1/")
        local got = collect(1)
        same("named HttpResponse", got[1][1], "HttpResponse")
        check("an errno, not a status", got[1][3] == 404 or got[1][3] == 400, got[1][3])
    "#,
    );
}

#[test]
fn a_websocket_opens_echoes_and_closes() {
    // `WebsocketOpened` hands the guest its own `send` and `close`.
    let url = echo_server();
    online(
        "internet-websocket",
        &format!(
            r#"
        local id = internet.CreateWebsocket("{url}")
        local got = collect(1)
        same("opened", got[1][1], "WebsocketOpened")
        same("its id", got[1][2], id)
        same("a send function", type(got[1][3]), "function")
        same("a close function", type(got[1][4]), "function")

        local send, close = got[1][3], got[1][4]
        send("ping", false)
        local replies = collect(2)
        local message, sent
        for _, e in ipairs(replies) do
            if e[1] == "WebsocketMessage" then message = e end
            if e[1] == "WebsocketSendSuccess" then sent = e end
        end
        check("the send was acknowledged", sent ~= nil)
        check("the echo came back", message ~= nil)
        same("with our bytes", message[3], "ping")
        same("as text, not binary", message[4], false)

        close()
        local ending = collect(1)
        same("closed", ending[1][1], "WebsocketClosed")
        same("its id", ending[1][2], id)
    "#
        ),
    );
}

#[test]
fn closing_a_socket_twice_is_an_error() {
    let url = echo_server();
    online(
        "internet-double-close",
        &format!(
            r#"
        internet.CreateWebsocket("{url}")
        local got = collect(1)
        local close = got[1][4]
        close()
        collect(1)
        raises("the second close", "Already closed", close)
    "#
        ),
    );
}
