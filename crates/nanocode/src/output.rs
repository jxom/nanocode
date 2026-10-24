//! Structured output for root agent turns.
//!
//! [`Outputs`] is an opt-in registry the host installs as the `submit_output`
//! tool. [`Outputs::prompt`] arms a JSON Schema contract for the agent's
//! session, sends the prompt with completion instructions, and returns the
//! value the model submitted, validated against the schema and decoded into
//! the requested type. A turn that ends without an accepted submission fails
//! with [`OutputError::Missing`], as a subagent turn without `submit_result`
//! does.

use std::{collections::HashMap, sync::Arc};

use nanocode_agent::input::{PromptInput, UserInput};
use nanocode_agent::{Agent, NanocodeError, PromptRequest, TurnResult};
use nanocode_schema::Validator;
use nanocode_sync::Atom;
use nanocode_tools::__private::{schemars::JsonSchema, serde::de::DeserializeOwned};
use nanocode_tools::{
    Tool, ToolContext, ToolDefinition, ToolInput, ToolOutput, ToolResult, schema_for,
};
use serde_json::{Value, json};

/// The name of the tool [`Outputs::tool`] registers.
pub const SUBMIT_OUTPUT_TOOL: &str = "submit_output";

/// A type an agent turn can return: anything with a JSON Schema that decodes
/// from JSON. `#[nanocode::output]` derives both.
pub trait Output: DeserializeOwned + JsonSchema {}

impl<T: DeserializeOwned + JsonSchema> Output for T {}

/// The JSON Schema [`Outputs::prompt`] gives the model and validates
/// submissions against for `T`.
#[must_use]
pub fn schema<T: Output>() -> Value {
    schema_for::<T>()
}

/// Output contracts armed per agent session, shared by the host and the
/// `submit_output` tool. Clones share one registry.
#[derive(Clone, Default)]
pub struct Outputs {
    contracts: Arc<Atom<HashMap<String, Contract>>>,
}

#[derive(Clone)]
struct Contract {
    schema: Arc<str>,
    validator: Arc<Validator>,
    submitted: Option<Value>,
}

/// A turn's decoded output and the turn's result.
pub struct OutputTurn<T> {
    pub output: T,
    pub result: TurnResult,
}

#[derive(Debug)]
pub enum OutputError {
    /// The schema derived for the output type does not compile.
    Schema(String),
    /// Another output turn is armed on this agent's session.
    Busy { session_id: String },
    /// The prompt or its turn failed.
    Turn(NanocodeError),
    /// The turn ended without an accepted `submit_output` call.
    Missing { result: TurnResult },
    /// The accepted value matched the schema but did not decode into the type.
    Decode(serde_json::Error),
}

impl std::fmt::Display for OutputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema(error) => write!(formatter, "invalid output schema: {error}"),
            Self::Busy { session_id } => write!(
                formatter,
                "session {session_id} already has an output turn in progress"
            ),
            Self::Turn(error) => write!(formatter, "{error}"),
            Self::Missing { .. } => {
                write!(formatter, "turn ended without a valid submit_output call")
            }
            Self::Decode(error) => write!(formatter, "could not decode submitted output: {error}"),
        }
    }
}

impl std::error::Error for OutputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Turn(error) => Some(error),
            Self::Decode(error) => Some(error),
            _ => None,
        }
    }
}

impl From<NanocodeError> for OutputError {
    fn from(error: NanocodeError) -> Self {
        Self::Turn(error)
    }
}

impl Outputs {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The `submit_output` tool for this registry. Register it with
    /// `Tools::builder().tool(...)` on the runtime whose agents prompt
    /// through [`Self::prompt`].
    #[must_use]
    pub fn tool(&self) -> SubmitOutput {
        SubmitOutput {
            outputs: self.clone(),
        }
    }

    /// Runs one turn on `agent` that must end with a `T` submitted through
    /// `submit_output`. The contract binds to the agent's session until the
    /// turn ends, so only one output turn runs per session at a time.
    pub async fn prompt<T: Output>(
        &self,
        agent: &Agent,
        request: impl Into<PromptRequest>,
    ) -> Result<OutputTurn<T>, OutputError> {
        let schema = schema::<T>();
        let validator = Arc::new(
            nanocode_schema::validator_for(&schema)
                .map_err(|error| OutputError::Schema(error.to_string()))?,
        );
        let schema: Arc<str> = schema.to_string().into();
        let session_id = agent.session_id().to_owned();
        self.contracts.modify(|contracts| {
            if contracts.contains_key(&session_id) {
                return Err(OutputError::Busy {
                    session_id: session_id.clone(),
                });
            }
            contracts.insert(
                session_id.clone(),
                Contract {
                    schema: Arc::clone(&schema),
                    validator: Arc::clone(&validator),
                    submitted: None,
                },
            );
            Ok(())
        })?;
        let armed = Armed {
            outputs: self,
            session_id: &session_id,
        };

        let mut request = request.into();
        append_instructions(
            &mut request.prompt.instruction,
            &completion_instructions(&schema),
        );
        let result = agent.prompt(request).await?.result().await?;

        let Some(value) = armed.take() else {
            return Err(OutputError::Missing { result });
        };
        let output = serde_json::from_value(value).map_err(OutputError::Decode)?;
        Ok(OutputTurn { output, result })
    }

    fn submit(&self, session_id: &str, output: Value) -> Result<bool, Value> {
        self.contracts.modify(|contracts| {
            let Some(contract) = contracts.get_mut(session_id) else {
                return Err(refusal(
                    "no output requested",
                    "This turn did not request structured output. Answer as assistant text.",
                    Vec::new(),
                ));
            };
            if contract.submitted.is_some() {
                return Err(refusal(
                    "already accepted",
                    "submit_output already accepted one result for this turn. Finish the turn.",
                    Vec::new(),
                ));
            }
            let (value, decoded) = validate(&contract.validator, &contract.schema, output.clone())?;
            contract.submitted = Some(value);
            Ok(decoded)
        })
    }
}

/// Removes a session's contract when its output turn ends, including when the
/// turn fails or the caller drops the future.
struct Armed<'a> {
    outputs: &'a Outputs,
    session_id: &'a str,
}

impl Armed<'_> {
    fn take(&self) -> Option<Value> {
        self.outputs.contracts.update(|contracts| {
            contracts
                .get_mut(self.session_id)
                .and_then(|contract| contract.submitted.take())
        })
    }
}

impl Drop for Armed<'_> {
    fn drop(&mut self) {
        self.outputs.contracts.update(|contracts| {
            contracts.remove(self.session_id);
        });
    }
}

/// The tool through which a model submits a turn's structured output.
pub struct SubmitOutput {
    outputs: Outputs,
}

#[cfg_attr(not(target_family = "wasm"), nanocode_tools::__private::async_trait)]
#[cfg_attr(target_family = "wasm", nanocode_tools::__private::async_trait(?Send))]
impl Tool for SubmitOutput {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::function(
            SUBMIT_OUTPUT_TOOL,
            "Submits this turn's final JSON output when the prompt asks for structured output. Supply only output matching the output schema in the prompt. Invalid values can be corrected and retried. After an accepted receipt, send a brief final assistant message with no further tool calls.",
            json!({
                "type": "object",
                "properties": {
                    "output": {
                        "anyOf": [
                            { "type": "object" }, { "type": "array", "items": {} },
                            { "type": "string" }, { "type": "number" },
                            { "type": "boolean" }, { "type": "null" }
                        ],
                        "description": "The final JSON value required by the output schema. Pass objects and arrays directly, not as JSON-encoded strings."
                    }
                },
                "required": ["output"],
                "additionalProperties": false
            }),
        )
    }

    async fn execute(&self, input: ToolInput, context: ToolContext<'_>) -> ToolResult {
        #[derive(nanocode_tools::__private::serde::Deserialize)]
        #[serde(crate = "nanocode_tools::__private::serde", deny_unknown_fields)]
        struct Args {
            output: Value,
        }
        let Args { output } = input.decode_json()?;
        let receipt = match self.outputs.submit(context.session_id(), output) {
            Ok(false) => json!({ "accepted": true }),
            Ok(true) => json!({ "accepted": true, "decoded_json_text": true }),
            Err(refusal) => return Ok(ToolOutput::from_json(refusal, false)),
        };
        Ok(ToolOutput::from_json(receipt, true))
    }
}

fn completion_instructions(schema: &str) -> String {
    format!(
        "Before finishing, call `{SUBMIT_OUTPUT_TOOL}` with `{{ output: ... }}` and a JSON value \
         matching the output schema below. Pass objects and arrays directly as JSON values. Use \
         the callable entry in your actual tool catalog. If validation rejects the value, correct \
         it and retry. Finish after acceptance. A turn that ends without an accepted result \
         fails.\n\nOutput schema:\n{schema}"
    )
}

fn append_instructions(instruction: &mut PromptInput, instructions: &str) {
    match instruction {
        PromptInput::Text(text) => {
            text.push_str("\n\n");
            text.push_str(instructions);
        }
        PromptInput::Content(items) => items.push(UserInput::Text {
            text: instructions.to_owned(),
        }),
    }
}

/// Accepts `output`, or a JSON-encoded object or array that decodes to a
/// valid value, as subagent `submit_result` does. Returns whether it decoded.
fn validate(validator: &Validator, schema: &str, output: Value) -> Result<(Value, bool), Value> {
    if validator.is_valid(&output) {
        return Ok((output, false));
    }
    if let Value::String(text) = &output
        && let Ok(decoded) = serde_json::from_str::<Value>(text)
        && matches!(decoded, Value::Object(_) | Value::Array(_))
        && validator.is_valid(&decoded)
    {
        return Ok((decoded, true));
    }
    let mut details = validator
        .iter_errors(&output)
        .take(4)
        .map(|issue| {
            format!(
                "instance {}: schema {}",
                issue.instance_path(),
                issue.schema_path()
            )
        })
        .collect::<Vec<_>>();
    details.push(format!("required schema: {schema}"));
    Err(refusal(
        "schema validation",
        "Correct the output using the required schema and retry submit_output within this turn; do not repeat task side effects.",
        details,
    ))
}

fn refusal(code: &str, recovery: &str, details: Vec<String>) -> Value {
    json!({
        "accepted": false,
        "code": code,
        "recovery": recovery,
        "details": details,
    })
}
