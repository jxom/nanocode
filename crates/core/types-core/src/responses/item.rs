use std::{fmt, ops::Deref};

use serde::{Deserialize, Serialize};

use super::FunctionOutputContent;
use super::{
    AgentMessageContent, ContentItem, FunctionOutputBody, InternalMessageMetadata, ItemStatus,
    JsonValue, LocalShellAction, LocalShellStatus, MessagePhase, MessageRole, ReasoningContent,
    ReasoningSummary, ToolCaller, ToolDefinition, WebSearchAction,
};
use crate::Thinking;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct ConfigurationUpdateReasoning {
    pub effort: Thinking,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ResponseItemId(Box<str>);

impl ResponseItemId {
    #[must_use]
    pub fn with_suffix(prefix: &str, suffix: impl fmt::Display) -> Self {
        Self(format!("{prefix}_{suffix}").into_boxed_str())
    }

    #[must_use]
    pub fn from_server(value: impl Into<Box<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn is_prefixed(&self) -> bool {
        self.split_once('_')
            .is_some_and(|(prefix, suffix)| !prefix.is_empty() && !suffix.is_empty())
    }
}

impl Deref for ResponseItemId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for ResponseItemId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ResponseItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<String> for ResponseItemId {
    fn from(value: String) -> Self {
        Self(value.into_boxed_str())
    }
}

impl From<&str> for ResponseItemId {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseItem {
    AdditionalTools {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        role: MessageRole,
        tools: Vec<ToolDefinition>,
    },
    Message {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        role: MessageRole,
        content: Vec<ContentItem>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ItemStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<MessagePhase>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    AgentMessage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        author: Box<str>,
        recipient: Box<str>,
        content: Vec<AgentMessageContent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    Reasoning {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        #[serde(default)]
        summary: Vec<ReasoningSummary>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ReasoningContent>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ItemStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    LocalShellCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<Box<str>>,
        status: LocalShellStatus,
        action: LocalShellAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    FunctionCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        name: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<Box<str>>,
        arguments: Box<str>,
        #[serde(default, rename = "async", skip_serializing_if = "is_false")]
        asynchronous: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_function_args: Option<Vec<Box<str>>>,
        call_id: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller: Option<ToolCaller>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ItemStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_by: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    FunctionCallOutput {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        call_id: Box<str>,
        output: FunctionOutputBody,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller: Option<ToolCaller>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ItemStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_by: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    ToolSearchCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<Box<str>>,
        execution: Box<str>,
        arguments: JsonValue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    CustomToolCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ItemStatus>,
        call_id: Box<str>,
        name: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<Box<str>>,
        input: Box<str>,
        #[serde(default, rename = "async", skip_serializing_if = "is_false")]
        asynchronous: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller: Option<ToolCaller>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_by: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    CustomToolCallOutput {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        call_id: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<Box<str>>,
        output: FunctionOutputBody,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller: Option<ToolCaller>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ItemStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_by: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    ToolSearchOutput {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_id: Option<Box<str>>,
        status: Box<str>,
        execution: Box<str>,
        tools: Vec<JsonValue>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    WebSearchCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<WebSearchAction>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    ImageGenerationCall {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        status: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revised_prompt: Option<Box<str>>,
        result: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    #[serde(alias = "compaction_summary")]
    Compaction {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        encrypted_content: Box<str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_by: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    CompactionTrigger {},
    ContextCompaction {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<ResponseItemId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<Box<str>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        internal_chat_message_metadata_passthrough: Option<InternalMessageMetadata>,
    },
    ConfigurationUpdate {
        reasoning: ConfigurationUpdateReasoning,
    },
    #[serde(untagged)]
    Other(JsonValue),
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(target_family = "wasm")]
impl<'de> Deserialize<'de> for ResponseItem {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(Self::from_fields(value.clone()).unwrap_or_else(|_| Self::Other(value.into())))
    }
}

#[cfg(target_family = "wasm")]
impl ResponseItem {
    fn from_fields(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let mut fields = crate::fields::Fields::new(value, "internally tagged enum ResponseItem")?;
        let tag = fields.tag()?;
        let metadata = "internal_chat_message_metadata_passthrough";
        Ok(match tag.as_str() {
            "additional_tools" => Self::AdditionalTools {
                id: fields.optional("id")?,
                role: fields.take("role")?,
                tools: fields.take("tools")?,
            },
            "message" => Self::Message {
                id: fields.optional("id")?,
                role: fields.take("role")?,
                content: fields.take("content")?,
                status: fields.optional("status")?,
                phase: fields.optional("phase")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "agent_message" => Self::AgentMessage {
                id: fields.optional("id")?,
                author: fields.take("author")?,
                recipient: fields.take("recipient")?,
                content: fields.take("content")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "reasoning" => Self::Reasoning {
                id: fields.optional("id")?,
                summary: fields.or_default("summary")?,
                content: fields.optional("content")?,
                encrypted_content: fields.optional("encrypted_content")?,
                status: fields.optional("status")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "local_shell_call" => Self::LocalShellCall {
                id: fields.optional("id")?,
                call_id: fields.optional("call_id")?,
                status: fields.take("status")?,
                action: fields.take("action")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "function_call" => Self::FunctionCall {
                id: fields.optional("id")?,
                name: fields.take("name")?,
                namespace: fields.optional("namespace")?,
                arguments: fields.take("arguments")?,
                asynchronous: fields.or_default("async")?,
                encrypted_function_args: fields.optional("encrypted_function_args")?,
                call_id: fields.take("call_id")?,
                caller: fields.optional("caller")?,
                status: fields.optional("status")?,
                created_by: fields.optional("created_by")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "function_call_output" => Self::FunctionCallOutput {
                id: fields.optional("id")?,
                call_id: fields.take("call_id")?,
                output: fields.take("output")?,
                caller: fields.optional("caller")?,
                status: fields.optional("status")?,
                created_by: fields.optional("created_by")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "tool_search_call" => Self::ToolSearchCall {
                id: fields.optional("id")?,
                call_id: fields.optional("call_id")?,
                status: fields.optional("status")?,
                execution: fields.take("execution")?,
                arguments: fields.take("arguments")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "custom_tool_call" => Self::CustomToolCall {
                id: fields.optional("id")?,
                status: fields.optional("status")?,
                call_id: fields.take("call_id")?,
                name: fields.take("name")?,
                namespace: fields.optional("namespace")?,
                input: fields.take("input")?,
                asynchronous: fields.or_default("async")?,
                caller: fields.optional("caller")?,
                created_by: fields.optional("created_by")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "custom_tool_call_output" => Self::CustomToolCallOutput {
                id: fields.optional("id")?,
                call_id: fields.take("call_id")?,
                name: fields.optional("name")?,
                output: fields.take("output")?,
                caller: fields.optional("caller")?,
                status: fields.optional("status")?,
                created_by: fields.optional("created_by")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "tool_search_output" => Self::ToolSearchOutput {
                id: fields.optional("id")?,
                call_id: fields.optional("call_id")?,
                status: fields.take("status")?,
                execution: fields.take("execution")?,
                tools: fields.take("tools")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "web_search_call" => Self::WebSearchCall {
                id: fields.optional("id")?,
                status: fields.optional("status")?,
                action: fields.optional("action")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "image_generation_call" => Self::ImageGenerationCall {
                id: fields.optional("id")?,
                status: fields.take("status")?,
                revised_prompt: fields.optional("revised_prompt")?,
                result: fields.take("result")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "compaction" | "compaction_summary" => Self::Compaction {
                id: fields.optional("id")?,
                encrypted_content: fields.take("encrypted_content")?,
                created_by: fields.optional("created_by")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "compaction_trigger" => Self::CompactionTrigger {},
            "context_compaction" => Self::ContextCompaction {
                id: fields.optional("id")?,
                encrypted_content: fields.optional("encrypted_content")?,
                internal_chat_message_metadata_passthrough: fields.optional(metadata)?,
            },
            "configuration_update" => Self::ConfigurationUpdate {
                reasoning: fields.take("reasoning")?,
            },
            _ => return Err(serde::de::Error::unknown_variant(&tag, &[])),
        })
    }
}

impl ResponseItem {
    #[doc(hidden)]
    pub fn strip_image_details(&mut self) {
        match self {
            Self::Message { content, .. } => {
                for item in content {
                    if let ContentItem::InputImage { detail, .. }
                    | ContentItem::InputImageFile { detail, .. } = item
                    {
                        *detail = None;
                    }
                }
            }
            Self::FunctionCallOutput { output, .. } | Self::CustomToolCallOutput { output, .. } => {
                if let FunctionOutputBody::Content(content) = output {
                    for item in content {
                        if let FunctionOutputContent::InputImage { detail, .. }
                        | FunctionOutputContent::InputImageFile { detail, .. } = item
                        {
                            *detail = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    #[must_use]
    pub const fn additional_tools(tools: Vec<ToolDefinition>) -> Self {
        Self::AdditionalTools {
            id: None,
            role: MessageRole::Developer,
            tools,
        }
    }

    #[must_use]
    pub fn message(role: MessageRole, content: impl IntoIterator<Item = ContentItem>) -> Self {
        Self::Message {
            id: None,
            role,
            content: content.into_iter().collect(),
            status: None,
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }
    }

    #[must_use]
    pub fn custom_tool_output(
        call_id: String,
        name: Option<String>,
        output: FunctionOutputBody,
    ) -> Self {
        Self::CustomToolCallOutput {
            id: None,
            call_id: call_id.into_boxed_str(),
            name: name.map(String::into_boxed_str),
            output,
            caller: None,
            status: None,
            created_by: None,
            internal_chat_message_metadata_passthrough: None,
        }
    }

    #[must_use]
    pub fn function_call_output(call_id: String, output: FunctionOutputBody) -> Self {
        Self::FunctionCallOutput {
            id: None,
            call_id: call_id.into_boxed_str(),
            output,
            caller: None,
            status: None,
            created_by: None,
            internal_chat_message_metadata_passthrough: None,
        }
    }

    #[must_use]
    pub const fn compaction_trigger() -> Self {
        Self::CompactionTrigger {}
    }

    #[must_use]
    pub const fn configuration_update(effort: Thinking) -> Self {
        Self::ConfigurationUpdate {
            reasoning: ConfigurationUpdateReasoning { effort },
        }
    }

    #[must_use]
    pub const fn is_user_message(&self) -> bool {
        matches!(
            self,
            Self::Message {
                role: MessageRole::User,
                ..
            }
        )
    }

    #[must_use]
    pub const fn id(&self) -> Option<&ResponseItemId> {
        match self {
            Self::AdditionalTools { id, .. }
            | Self::Message { id, .. }
            | Self::AgentMessage { id, .. }
            | Self::Reasoning { id, .. }
            | Self::LocalShellCall { id, .. }
            | Self::FunctionCall { id, .. }
            | Self::FunctionCallOutput { id, .. }
            | Self::ToolSearchCall { id, .. }
            | Self::CustomToolCall { id, .. }
            | Self::CustomToolCallOutput { id, .. }
            | Self::ToolSearchOutput { id, .. }
            | Self::WebSearchCall { id, .. }
            | Self::ImageGenerationCall { id, .. }
            | Self::Compaction { id, .. }
            | Self::ContextCompaction { id, .. } => id.as_ref(),
            Self::CompactionTrigger {} | Self::ConfigurationUpdate { .. } | Self::Other(_) => None,
        }
    }

    pub fn set_id(&mut self, new_id: Option<ResponseItemId>) {
        match self {
            Self::AdditionalTools { id, .. }
            | Self::Message { id, .. }
            | Self::AgentMessage { id, .. }
            | Self::Reasoning { id, .. }
            | Self::LocalShellCall { id, .. }
            | Self::FunctionCall { id, .. }
            | Self::FunctionCallOutput { id, .. }
            | Self::ToolSearchCall { id, .. }
            | Self::CustomToolCall { id, .. }
            | Self::CustomToolCallOutput { id, .. }
            | Self::ToolSearchOutput { id, .. }
            | Self::WebSearchCall { id, .. }
            | Self::ImageGenerationCall { id, .. }
            | Self::Compaction { id, .. }
            | Self::ContextCompaction { id, .. } => *id = new_id,
            Self::CompactionTrigger {} | Self::ConfigurationUpdate { .. } | Self::Other(_) => {}
        }
    }

    #[must_use]
    pub const fn id_prefix(&self) -> Option<&'static str> {
        match self {
            Self::AdditionalTools { .. } => Some("at"),
            Self::Message { .. } => Some("msg"),
            Self::AgentMessage { .. } => Some("amsg"),
            Self::Reasoning { .. } => Some("rs"),
            Self::LocalShellCall { .. } => Some("lsh"),
            Self::FunctionCall { .. } => Some("fc"),
            Self::FunctionCallOutput { .. } => Some("fco"),
            Self::ToolSearchCall { .. } => Some("tsc"),
            Self::CustomToolCall { .. } => Some("ctc"),
            Self::CustomToolCallOutput { .. } => Some("ctco"),
            Self::ToolSearchOutput { .. } => Some("tso"),
            Self::WebSearchCall { .. } => Some("ws"),
            Self::ImageGenerationCall { .. } => Some("ig"),
            Self::Compaction { .. } | Self::ContextCompaction { .. } => Some("cmp"),
            Self::CompactionTrigger {} | Self::ConfigurationUpdate { .. } | Self::Other(_) => None,
        }
    }

    pub fn strip_id(&mut self) {
        self.set_id(None);
    }

    pub fn strip_unbound_id(&mut self) {
        let provider_bound = matches!(
            self,
            Self::Reasoning {
                encrypted_content: Some(_),
                ..
            } | Self::FunctionCall {
                encrypted_function_args: Some(_),
                ..
            } | Self::Compaction { .. }
                | Self::ContextCompaction {
                    encrypted_content: Some(_),
                    ..
                }
        );
        if !provider_bound {
            self.strip_id();
        }
    }
}
