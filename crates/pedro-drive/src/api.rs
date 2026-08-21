//! The two Drive calls this needs: what a file is, and its bytes.

use serde::Deserialize;

use crate::{DriveError, http};

const FILES: &str = "https://www.googleapis.com/drive/v3/files";

/// Google's own formats — a Doc, a Sheet, a Slide deck — hold no bytes to
/// download. They are converted on the way out instead.
const NATIVE: &str = "application/vnd.google-apps.";

const PDF: &str = "application/pdf";

/// What a file is, before deciding how to get it.
#[derive(Deserialize)]
pub(crate) struct Metadata {
    pub(crate) name: String,
    #[serde(rename = "mimeType", default)]
    pub(crate) mime_type: String,
}

pub(crate) fn metadata(access_token: &str, file_id: &str) -> Result<Metadata, DriveError> {
    let response = http::agent()
        .get(&format!(
            "{FILES}/{file_id}?fields=name,mimeType&supportsAllDrives=true"
        ))
        .header("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(DriveError::network)?;

    let body = read(response, file_id)?;
    serde_json::from_slice(&body)
        .map_err(|err| DriveError::Google(format!("the file description made no sense: {err}")))
}

/// The file as a PDF, converting it if that is what it takes.
pub(crate) fn download(
    access_token: &str,
    file_id: &str,
    metadata: &Metadata,
) -> Result<Vec<u8>, DriveError> {
    let url = if metadata.mime_type.starts_with(NATIVE) {
        // Not every Google format exports as a PDF — a Form or a Site does not
        // — but the ones that do all export the same way, and the ones that do
        // not are refused by Drive with a message worth passing on.
        format!("{FILES}/{file_id}/export?mimeType={PDF}")
    } else if metadata.mime_type == PDF {
        format!("{FILES}/{file_id}?alt=media&supportsAllDrives=true")
    } else {
        return Err(DriveError::NotAPdf {
            name: metadata.name.clone(),
            mime_type: metadata.mime_type.clone(),
        });
    };

    let response = http::agent()
        .get(&url)
        .header("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(DriveError::network)?;

    read(response, file_id)
}

/// The body, or what Drive said instead.
fn read(
    mut response: ureq::http::Response<ureq::Body>,
    file_id: &str,
) -> Result<Vec<u8>, DriveError> {
    let status = response.status();
    let body = response
        .body_mut()
        .with_config()
        // A book is easily larger than the limit a body read defaults to, and
        // the only thing on the other end of this is Google.
        .limit(u64::MAX)
        .read_to_vec()
        .map_err(DriveError::network)?;

    if status.is_success() {
        return Ok(body);
    }

    // 404 is also what Drive says when the file exists but this account cannot
    // see it, so the two are reported as one thing the reader can act on.
    if status.as_u16() == 404 {
        return Err(DriveError::NoSuchFile(file_id.to_owned()));
    }

    Err(DriveError::Google(explain(&body).unwrap_or_else(|| {
        format!("Drive answered {}", status.as_u16())
    })))
}

#[derive(Deserialize)]
struct Failure {
    error: FailureBody,
}

#[derive(Deserialize)]
struct FailureBody {
    #[serde(default)]
    message: String,
}

fn explain(body: &[u8]) -> Option<String> {
    let failure: Failure = serde_json::from_slice(body).ok()?;
    Some(failure.error.message).filter(|message| !message.is_empty())
}
