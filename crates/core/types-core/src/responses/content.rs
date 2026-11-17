use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[cfg(target_family = "wasm")]
use crate::fields::unknown;
use crate::{ImageDetail, fields::Fields};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    Developer,

    User,

    Assistant,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagePhase {
    Commentary,

    FinalAnswer,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    InProgress,

    Completed,

    Incomplete,

    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InternalMessageMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<Box<str>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentItem {
    InputText {
        text: Box<str>,
    },

    InputImage {
        image_url: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetail>,
    },

    #[serde(rename = "input_image")]
    InputImageFile {
        #[serde(serialize_with = "crate::responses::image_reference::serialize_file_id")]
        file_id: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetail>,
    },

    InputAudio {
        audio_url: Box<str>,
    },

    OutputText {
        text: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        annotations: Option<Vec<OutputTextAnnotation>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<OutputTextLogprob>>,
    },
}

impl ContentItem {
    #[must_use]
    pub fn input_text(text: impl Into<Box<str>>) -> Self {
        Self::InputText { text: text.into() }
    }

    #[must_use]
    pub fn input_image(image_url: impl Into<Box<str>>) -> Self {
        Self::InputImage {
            image_url: image_url.into(),
            detail: None,
        }
    }

    #[must_use]
    pub fn input_image_with_detail(image_url: impl Into<Box<str>>, detail: ImageDetail) -> Self {
        Self::InputImage {
            image_url: image_url.into(),
            detail: Some(detail),
        }
    }

    #[must_use]
    pub fn input_audio(audio_url: impl Into<Box<str>>) -> Self {
        Self::InputAudio {
            audio_url: audio_url.into(),
        }
    }

    #[must_use]
    pub fn output_text(text: impl Into<Box<str>>) -> Self {
        Self::OutputText {
            text: text.into(),
            annotations: None,
            logprobs: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalShellStatus {
    Completed,

    InProgress,

    Incomplete,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalShellAction {
    Exec(LocalShellExecAction),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LocalShellExecAction {
    pub command: Vec<String>,

    pub timeout_ms: Option<u64>,

    pub working_directory: Option<Box<str>>,

    pub env: Option<HashMap<Box<str>, Box<str>>>,

    pub user: Option<Box<str>>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSearchAction {
    Search {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queries: Option<Vec<Box<str>>>,
    },

    OpenPage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<Box<str>>,
    },

    FindInPage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<Box<str>>,
    },

    #[cfg_attr(not(target_family = "wasm"), serde(other))]
    Other,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputTextAnnotation {
    FileCitation {
        file_id: Box<str>,
        filename: Box<str>,
        index: u64,
    },
    UrlCitation {
        end_index: u64,
        start_index: u64,
        title: Box<str>,
        url: Box<str>,
    },
    ContainerFileCitation {
        container_id: Box<str>,
        end_index: u64,
        file_id: Box<str>,
        filename: Box<str>,
        start_index: u64,
    },
    FilePath {
        file_id: Box<str>,
        index: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OutputTextLogprob {
    pub token: Box<str>,

    pub bytes: Vec<u8>,

    pub logprob: f64,

    pub top_logprobs: Vec<OutputTextTopLogprob>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OutputTextTopLogprob {
    pub token: Box<str>,

    pub bytes: Vec<u8>,

    pub logprob: f64,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCaller {
    Direct,

    Program { caller_id: Box<str> },
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessageContent {
    InputText { text: Box<str> },

    EncryptedContent { encrypted_content: Box<str> },
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningSummary {
    SummaryText { text: Box<str> },
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningContent {
    ReasoningText { text: Box<str> },

    Text { text: Box<str> },
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(untagged)]
pub enum FunctionOutputBody {
    Text(Box<str>),

    Content(Vec<FunctionOutputContent>),
}

#[cfg(target_family = "wasm")]
impl<'de> Deserialize<'de> for FunctionOutputBody {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = FunctionOutputBody;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("untagged enum FunctionOutputBody")
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<FunctionOutputBody, E> {
                Ok(FunctionOutputBody::Text(text.into()))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                seq: A,
            ) -> Result<FunctionOutputBody, A::Error> {
                Vec::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))
                    .map(FunctionOutputBody::Content)
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FunctionOutputContent {
    InputText {
        text: Box<str>,
    },

    InputImage {
        image_url: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetail>,
    },

    #[serde(rename = "input_image")]
    InputImageFile {
        #[serde(serialize_with = "crate::responses::image_reference::serialize_file_id")]
        file_id: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetail>,
    },

    InputAudio {
        audio_url: Box<str>,
    },

    EncryptedContent {
        encrypted_content: Box<str>,
    },
}

fn input_image(
    value: serde_json::Value,
) -> Result<(crate::responses::ImageReference, Option<ImageDetail>), String> {
    let serde_json::Value::Object(mut object) = value else {
        return Err("expected image object".to_owned());
    };
    object.remove("type");
    let detail = object.remove("detail");
    let reference = serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|error| error.to_string())?;
    let detail = serde_json::from_value(detail.unwrap_or(serde_json::Value::Null))
        .map_err(|error| error.to_string())?;
    Ok((reference, detail))
}

impl<'de> Deserialize<'de> for ContentItem {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("type").and_then(serde_json::Value::as_str) == Some("input_image") {
            let (reference, detail) = input_image(value).map_err(serde::de::Error::custom)?;
            return Ok(match reference {
                crate::responses::ImageReference::Inline { image_url } => Self::InputImage {
                    image_url: image_url.into(),
                    detail,
                },
                crate::responses::ImageReference::File { file_id } => Self::InputImageFile {
                    file_id: file_id.into(),
                    detail,
                },
            });
        }
        Self::from_fields(value).map_err(serde::de::Error::custom)
    }
}

impl ContentItem {
    fn from_fields(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let mut fields = Fields::new(value, "internally tagged enum ContentItem")?;
        let tag = fields.tag()?;
        Ok(match tag.as_str() {
            "input_text" => Self::InputText {
                text: fields.take("text")?,
            },
            "input_audio" => Self::InputAudio {
                audio_url: fields.take("audio_url")?,
            },
            "output_text" => Self::OutputText {
                text: fields.take("text")?,
                annotations: fields.optional("annotations")?,
                logprobs: fields.optional("logprobs")?,
            },
            _ => {
                return Err(serde::de::Error::unknown_variant(
                    &tag,
                    &["input_text", "input_image", "input_audio", "output_text"],
                ));
            }
        })
    }
}

impl<'de> Deserialize<'de> for FunctionOutputContent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("type").and_then(serde_json::Value::as_str) == Some("input_image") {
            let (reference, detail) = input_image(value).map_err(serde::de::Error::custom)?;
            return Ok(match reference {
                crate::responses::ImageReference::Inline { image_url } => Self::InputImage {
                    image_url: image_url.into(),
                    detail,
                },
                crate::responses::ImageReference::File { file_id } => Self::InputImageFile {
                    file_id: file_id.into(),
                    detail,
                },
            });
        }
        Self::from_fields(value).map_err(serde::de::Error::custom)
    }
}

impl FunctionOutputContent {
    fn from_fields(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let mut fields = Fields::new(value, "internally tagged enum FunctionOutputContent")?;
        let tag = fields.tag()?;
        Ok(match tag.as_str() {
            "input_text" => Self::InputText {
                text: fields.take("text")?,
            },
            "input_audio" => Self::InputAudio {
                audio_url: fields.take("audio_url")?,
            },
            "encrypted_content" => Self::EncryptedContent {
                encrypted_content: fields.take("encrypted_content")?,
            },
            _ => {
                return Err(serde::de::Error::unknown_variant(
                    &tag,
                    &[
                        "input_text",
                        "input_image",
                        "input_audio",
                        "encrypted_content",
                    ],
                ));
            }
        })
    }
}

#[cfg(target_family = "wasm")]
tagged!(LocalShellAction, |fields, tag| match tag.as_str() {
    "exec" => Ok(Self::Exec(fields.rest()?)),
    _ => Err(unknown(&tag, &["exec"])),
});

#[cfg(target_family = "wasm")]
tagged!(WebSearchAction, |fields, tag| Ok(match tag.as_str() {
    "search" => Self::Search {
        query: fields.optional("query")?,
        queries: fields.optional("queries")?,
    },
    "open_page" => Self::OpenPage {
        url: fields.optional("url")?,
    },
    "find_in_page" => Self::FindInPage {
        url: fields.optional("url")?,
        pattern: fields.optional("pattern")?,
    },
    _ => Self::Other,
}));

#[cfg(target_family = "wasm")]
tagged!(OutputTextAnnotation, |fields, tag| Ok(match tag.as_str() {
    "file_citation" => Self::FileCitation {
        file_id: fields.take("file_id")?,
        filename: fields.take("filename")?,
        index: fields.take("index")?,
    },
    "url_citation" => Self::UrlCitation {
        end_index: fields.take("end_index")?,
        start_index: fields.take("start_index")?,
        title: fields.take("title")?,
        url: fields.take("url")?,
    },
    "container_file_citation" => Self::ContainerFileCitation {
        container_id: fields.take("container_id")?,
        end_index: fields.take("end_index")?,
        file_id: fields.take("file_id")?,
        filename: fields.take("filename")?,
        start_index: fields.take("start_index")?,
    },
    "file_path" => Self::FilePath {
        file_id: fields.take("file_id")?,
        index: fields.take("index")?,
    },
    _ => {
        return Err(unknown(
            &tag,
            &[
                "file_citation",
                "url_citation",
                "container_file_citation",
                "file_path",
            ],
        ));
    }
}));

#[cfg(target_family = "wasm")]
tagged!(ToolCaller, |fields, tag| Ok(match tag.as_str() {
    "direct" => Self::Direct,
    "program" => Self::Program {
        caller_id: fields.take("caller_id")?,
    },
    _ => return Err(unknown(&tag, &["direct", "program"])),
}));

#[cfg(target_family = "wasm")]
tagged!(AgentMessageContent, |fields, tag| Ok(match tag.as_str() {
    "input_text" => Self::InputText {
        text: fields.take("text")?,
    },
    "encrypted_content" => Self::EncryptedContent {
        encrypted_content: fields.take("encrypted_content")?,
    },
    _ => return Err(unknown(&tag, &["input_text", "encrypted_content"])),
}));

#[cfg(target_family = "wasm")]
tagged!(ReasoningSummary, |fields, tag| match tag.as_str() {
    "summary_text" => Ok(Self::SummaryText {
        text: fields.take("text")?,
    }),
    _ => Err(unknown(&tag, &["summary_text"])),
});

#[cfg(target_family = "wasm")]
tagged!(ReasoningContent, |fields, tag| Ok(match tag.as_str() {
    "reasoning_text" => Self::ReasoningText {
        text: fields.take("text")?,
    },
    "text" => Self::Text {
        text: fields.take("text")?,
    },
    _ => return Err(unknown(&tag, &["reasoning_text", "text"])),
}));
