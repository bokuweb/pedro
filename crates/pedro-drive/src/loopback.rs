//! The other half of the browser dance: a listener on localhost that catches
//! the one redirect Google sends back, and then stops.
//!
//! Google's installed-application flow has no other way home. There is no
//! server to redirect to, and pasting a code out of a browser window was
//! retired. So the application briefly *is* the server: one port, one request,
//! one page saying it worked, and then the socket closes.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::DriveError;

/// How long the reader has to finish signing in before the listener gives up.
///
/// Long enough to find the right Google account and read a consent screen;
/// short enough that a window closed halfway through does not leave a port
/// open for the rest of the session.
const PATIENCE: Duration = Duration::from_secs(300);

/// How often the listener looks for a connection while it waits.
const POLL: Duration = Duration::from_millis(50);

/// A port on localhost, waiting for the redirect.
pub(crate) struct Loopback {
    listener: TcpListener,
    port: u16,
}

impl Loopback {
    /// Takes whichever port the operating system has free.
    ///
    /// The port is part of the redirect URI, so it cannot be chosen ahead of
    /// time and written into the Google console — which is why Google exempts
    /// loopback redirects from having to match a registered one exactly.
    pub(crate) fn bind() -> Result<Self, DriveError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        Ok(Self { listener, port })
    }

    pub(crate) fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Waits for the redirect and hands back its query string.
    ///
    /// Browsers ask for more than they are sent to — a favicon, a speculative
    /// preconnect — so anything without a query is answered and ignored rather
    /// than mistaken for the redirect.
    pub(crate) fn wait_for_redirect(&self) -> Result<String, DriveError> {
        let deadline = Instant::now() + PATIENCE;

        while Instant::now() < deadline {
            let mut stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(POLL);
                    continue;
                }
                Err(err) => return Err(err.into()),
            };

            let Some(query) = read_query(&mut stream) else {
                answer(&mut stream, NOT_IT);
                continue;
            };

            answer(&mut stream, DONE);
            return Ok(query);
        }

        Err(DriveError::SignInTimedOut)
    }
}

/// The query string of the request line, if it has one.
///
/// A connection that says nothing is as uninteresting as one that asks for a
/// favicon, and browsers open both — so a read that times out is `None` rather
/// than an error that would take the whole sign-in down with it.
fn read_query(stream: &mut TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(POLL * 20)).ok()?;

    let mut request_line = String::new();
    BufReader::new(&*stream).read_line(&mut request_line).ok()?;

    // `GET /?code=… HTTP/1.1`, and the middle field is the only one that
    // matters. The body is never read: the redirect is a GET, and the socket
    // is closed the moment the page is written.
    request_line
        .split_whitespace()
        .nth(1)
        .and_then(|target| target.split_once('?'))
        .map(|(_, query)| query.to_owned())
}

fn answer(stream: &mut TcpStream, page: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );

    // Nothing downstream can act on a page that failed to reach a browser the
    // reader is about to close anyway; the code has already been captured.
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

const DONE: &str = "<!doctype html><meta charset=utf-8><title>pedro</title>\
    <body style=\"font:16px system-ui;display:grid;place-items:center;height:90vh;margin:0\">\
    <p>Signed in. You can close this tab and go back to pedro.</p>";

const NOT_IT: &str = "<!doctype html><meta charset=utf-8><title>pedro</title>";
