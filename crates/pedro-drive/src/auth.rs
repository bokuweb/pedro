//! Signing in to Google, once, and staying signed in.
//!
//! This is the only place in pedro that asks a remote service who the reader
//! is. It follows Google's installed-application flow: a code exchanged in the
//! browser, bound to this run by PKCE, traded for a refresh token that goes
//! into the operating system's keychain. After the first time nothing opens a
//! browser again — a refresh token becomes an access token in one request.

use std::process::Command;

use base64::Engine as _;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::loopback::Loopback;
use crate::{DriveError, http};

const AUTHORIZE: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";

/// Read-only across the whole of the reader's Drive.
///
/// The narrower `drive.file` scope only reaches files chosen through Google's
/// own picker, which is a web component: it would need a browser embedded in
/// the application to show it. Pasting a link is what pedro has instead, and a
/// link can name any file, so the scope has to be able to as well.
const SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";

/// Where the refresh token is kept, as the keychain names it.
const SERVICE: &str = "pedro";
const ACCOUNT: &str = "google-drive";

/// The OAuth client this copy of pedro is.
///
/// Not built in. A client id identifies an application to Google and comes
/// with that application's consent screen and its quota, so it is something
/// whoever builds pedro creates for themselves — see `docs/GOOGLE_DRIVE.md`.
#[derive(Clone)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}

impl Credentials {
    /// The credentials in the environment, if they are there.
    ///
    /// The secret in an installed application is not a secret — it ships inside
    /// every copy, and Google says so — but it is still per-installation
    /// configuration rather than something to commit, so it is read rather than
    /// compiled in.
    pub fn from_env() -> Option<Self> {
        let client_id = non_empty("PEDRO_GOOGLE_CLIENT_ID")?;
        let client_secret = non_empty("PEDRO_GOOGLE_CLIENT_SECRET").unwrap_or_default();

        Some(Self {
            client_id,
            client_secret,
        })
    }
}

fn non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// An access token, good for about an hour and used immediately.
pub(crate) fn access_token(credentials: &Credentials) -> Result<String, DriveError> {
    if let Some(refresh) = stored_refresh_token()? {
        match refresh_access_token(credentials, &refresh) {
            Ok(access) => return Ok(access),
            // A refresh token stops working when it is revoked, when the
            // reader changes their password, or — the one that catches people
            // out — seven days after it was issued, while the OAuth client is
            // still in testing. None of those are worth an error: they are all
            // "ask again", and asking again is a browser window.
            Err(DriveError::SignInExpired) => {
                tracing::info!("the stored Google sign-in no longer works; asking again");
                forget()?;
            }
            Err(err) => return Err(err),
        }
    }

    // `invalid_grant` on a code that was just issued is not an expired
    // sign-in — it is a sign-in that did not work — and saying "expired" about
    // the first one of a session explains nothing.
    let granted = sign_in(credentials).map_err(|err| match err {
        DriveError::SignInExpired => DriveError::SignInRefused("the code was refused".into()),
        other => other,
    })?;
    if let Some(refresh) = &granted.refresh_token {
        store_refresh_token(refresh)?;
    }

    Ok(granted.access_token)
}

/// Forgets the stored sign-in, so the next fetch asks for a new one.
pub fn forget() -> Result<(), DriveError> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(DriveError::Keychain(err.to_string())),
    }
}

/// Whether a sign-in is already stored.
pub fn is_signed_in() -> bool {
    matches!(stored_refresh_token(), Ok(Some(_)))
}

/// The browser dance. Blocks until the reader comes back, or gives up.
fn sign_in(credentials: &Credentials) -> Result<Granted, DriveError> {
    let loopback = Loopback::bind()?;
    let redirect_uri = loopback.redirect_uri();

    // The verifier never leaves this process until the code has been received,
    // which is what stops another application on the machine racing to the
    // loopback port and redeeming the code it catches.
    let verifier = secret();
    let challenge = challenge_for(&verifier);
    let state = secret();

    let url = format!(
        "{AUTHORIZE}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &code_challenge={challenge}&code_challenge_method=S256&state={state}\
         &access_type=offline&prompt=consent",
        encode(&credentials.client_id),
        encode(&redirect_uri),
        encode(SCOPE),
    );

    open_in_browser(&url)?;
    let query = loopback.wait_for_redirect()?;

    let mut code = None;
    let mut returned_state = None;
    let mut denied = None;
    for (key, value) in parse_query(&query) {
        match key.as_str() {
            "code" => code = Some(value),
            "state" => returned_state = Some(value),
            "error" => denied = Some(value),
            _ => {}
        }
    }

    if let Some(denied) = denied {
        return Err(DriveError::SignInRefused(denied));
    }
    // A redirect carrying someone else's state is not the one that was asked
    // for, and its code is not one to redeem.
    if returned_state.as_deref() != Some(state.as_str()) {
        return Err(DriveError::SignInRefused("the reply did not match".into()));
    }
    let Some(code) = code else {
        return Err(DriveError::SignInRefused("no code came back".into()));
    };

    redeem(credentials, &code, &verifier, &redirect_uri)
}

/// Trades the authorization code for tokens.
fn redeem(
    credentials: &Credentials,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Granted, DriveError> {
    let response = http::agent()
        .post(TOKEN)
        .send_form([
            ("client_id", credentials.client_id.as_str()),
            ("client_secret", credentials.client_secret.as_str()),
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ])
        .map_err(DriveError::network)?;

    read_token_response(response)
}

fn refresh_access_token(
    credentials: &Credentials,
    refresh_token: &str,
) -> Result<String, DriveError> {
    let response = http::agent()
        .post(TOKEN)
        .send_form([
            ("client_id", credentials.client_id.as_str()),
            ("client_secret", credentials.client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .map_err(DriveError::network)?;

    Ok(read_token_response(response)?.access_token)
}

#[derive(Deserialize)]
struct Granted {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct Refused {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

fn read_token_response(
    mut response: ureq::http::Response<ureq::Body>,
) -> Result<Granted, DriveError> {
    let status = response.status();
    let body = response
        .body_mut()
        .read_to_vec()
        .map_err(DriveError::network)?;

    if status.is_success() {
        return serde_json::from_slice(&body)
            .map_err(|err| DriveError::Google(format!("the token reply made no sense: {err}")));
    }

    let refused: Refused = serde_json::from_slice(&body).unwrap_or(Refused {
        error: status.as_u16().to_string(),
        error_description: String::new(),
    });

    // `invalid_grant` is the one answer that means "this sign-in is over"
    // rather than "something went wrong", and it is the one the caller can do
    // something about without telling the reader anything.
    if refused.error == "invalid_grant" {
        return Err(DriveError::SignInExpired);
    }

    Err(DriveError::Google(
        match refused.error_description.is_empty() {
            true => refused.error,
            false => format!("{}: {}", refused.error, refused.error_description),
        },
    ))
}

fn stored_refresh_token() -> Result<Option<String>, DriveError> {
    match entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(DriveError::Keychain(err.to_string())),
    }
}

fn store_refresh_token(token: &str) -> Result<(), DriveError> {
    entry()?
        .set_password(token)
        .map_err(|err| DriveError::Keychain(err.to_string()))
}

fn entry() -> Result<keyring::Entry, DriveError> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|err| DriveError::Keychain(err.to_string()))
}

/// Something long and unguessable, in the alphabet PKCE allows.
///
/// Two v4 UUIDs written as hex: 64 characters, inside the 43–128 a verifier
/// may be, and nothing in it needs escaping in a URL.
fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn open_in_browser(url: &str) -> Result<(), DriveError> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };

    Command::new(opener)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|err| DriveError::NoBrowser(err.to_string()))
}

/// Percent-encodes everything that is not unreserved, which is enough for the
/// handful of values that go into the authorization URL.
fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());

    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    encoded
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or_default();
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        decoded.push(byte);
                        index += 3;
                    }
                    // Not an escape after all, so it is a literal per cent.
                    Err(_) => {
                        decoded.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (decode(key), decode(value)),
            None => (decode(pair), String::new()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example from RFC 7636, which is what every server checks
    /// against.
    #[test]
    fn the_challenge_is_the_one_pkce_specifies() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_verifier_is_long_enough_to_be_one() {
        let verifier = secret();

        assert!((43..=128).contains(&verifier.len()));
        assert!(verifier.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn a_redirect_uri_survives_being_put_in_a_url() {
        assert_eq!(
            encode("http://127.0.0.1:53017"),
            "http%3A%2F%2F127.0.0.1%3A53017"
        );
    }

    #[test]
    fn the_reply_is_read_back_as_it_was_sent() {
        let query = "state=abc&code=4%2F0AX4%2BsomeCode&scope=https%3A%2F%2Fexample.com";
        let parsed = parse_query(query);

        assert_eq!(parsed[0], ("state".to_owned(), "abc".to_owned()));
        assert_eq!(parsed[1], ("code".to_owned(), "4/0AX4+someCode".to_owned()));
        assert_eq!(
            parsed[2],
            ("scope".to_owned(), "https://example.com".to_owned())
        );
    }

    #[test]
    fn a_refusal_is_read_back_too() {
        let parsed = parse_query("error=access_denied&state=abc");

        assert_eq!(parsed[0], ("error".to_owned(), "access_denied".to_owned()));
    }
}
