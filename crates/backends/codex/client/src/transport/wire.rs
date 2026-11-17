use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::value::RawValue;

use super::ResponsesError;

pub struct EncodedRequest(Arc<String>);

#[cfg(not(target_family = "wasm"))]
struct Shared(Arc<String>);

#[cfg(not(target_family = "wasm"))]
impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl EncodedRequest {
    pub fn new<T: Serialize + ?Sized>(request: &T) -> Result<Self, ResponsesError> {
        serde_json::to_string(request)
            .map(|json| Self(Arc::new(json)))
            .map_err(ResponsesError::EncodeRequest)
    }

    pub(crate) fn create(
        request: &crate::client::responses::ResponseCreate<'_>,
    ) -> Result<Self, ResponsesError> {
        request
            .encode()
            .map(|json| Self(Arc::new(json)))
            .map_err(ResponsesError::EncodeRequest)
    }

    #[must_use]
    pub fn json(&self) -> &str {
        &self.0
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn body(&self) -> nanocode_bytes::Bytes {
        nanocode_bytes::Bytes::from_owner(Shared(Arc::clone(&self.0)))
    }

    #[must_use]
    pub fn into_string(self) -> String {
        Arc::unwrap_or_clone(self.0)
    }
}

pub(crate) struct DecodedEvent<'a, T> {
    pub(crate) raw: &'a str,
    pub(crate) decoded: Result<T, ResponsesError>,
}

pub(crate) fn decode_event<T: DeserializeOwned>(
    text: &str,
) -> Result<DecodedEvent<'_, T>, ResponsesError> {
    let raw = text.trim_matches([' ', '\t', '\n', '\r']);
    let decoded = match serde_json::from_str(raw) {
        Ok(event) => Ok(event),
        Err(source) => {
            serde_json::from_str::<&RawValue>(text).map_err(ResponsesError::InvalidJson)?;
            Err(ResponsesError::InvalidPayload {
                source,
                event: raw.to_owned(),
            })
        }
    };
    Ok(DecodedEvent { raw, decoded })
}

pub(crate) fn turn_state_from_event(text: &str) -> Option<String> {
    if text.starts_with(r#"{"type":""#) && !text.starts_with(r#"{"type":"response.metadata""#) {
        return None;
    }
    metadata_headers(text)?
        .into_iter()
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case(TURN_STATE_HEADER)
                .then_some(value)
        })
}

const TURN_STATE_HEADER: &str = "x-codex-turn-state";

#[cfg(not(target_family = "wasm"))]
fn metadata_headers(text: &str) -> Option<HashMap<String, String>> {
    match serde_json::from_str(text) {
        Ok(MetadataEvent::Metadata { headers }) => Some(headers),
        _ => None,
    }
}

#[cfg(target_family = "wasm")]
fn metadata_headers(text: &str) -> Option<HashMap<String, String>> {
    let mut event: serde_json::Map<String, serde_json::Value> = serde_json::from_str(text).ok()?;
    if event.get("type")?.as_str()? != "response.metadata" {
        return None;
    }
    event
        .remove("headers")
        .map_or(Ok(HashMap::new()), HashMap::deserialize)
        .ok()
}

#[cfg(not(target_family = "wasm"))]
#[derive(Deserialize)]
#[serde(tag = "type")]
enum MetadataEvent {
    #[serde(rename = "response.metadata")]
    Metadata {
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    #[serde(other)]
    Other,
}
