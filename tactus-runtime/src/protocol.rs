//! The small, language-neutral plugin protocol used by Tactus.

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
use thiserror::Error;

/// Wire-version understood by this runtime.
pub const PLUGIN_API: &str = "agenstro.plugin/v1";

/// A plugin request identifier. Booleans and floating-point numbers are not IDs.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RequestId {
    /// Human-readable or generated identifier.
    Text(String),
    /// Integer identifier used by simple clients.
    Integer(i64),
}

/// One request written to a plugin's standard input.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRequest {
    /// Protocol version.
    pub api: String,
    /// Correlation identifier copied into every response frame.
    pub id: RequestId,
    /// Plugin-defined operation.
    pub method: String,
    /// Plugin-defined JSON object.
    pub params: Map<String, Value>,
}

impl PluginRequest {
    /// Build a request using the current protocol version.
    pub fn new(
        id: impl Into<String>,
        method: impl Into<String>,
        params: Map<String, Value>,
    ) -> Result<Self, ProtocolFault> {
        let method = method.into();
        if method.is_empty() {
            return Err(ProtocolFault::EmptyMethod);
        }
        Ok(Self {
            api: PLUGIN_API.to_owned(),
            id: RequestId::Text(id.into()),
            method,
            params,
        })
    }

    /// Validate a request obtained from an external source.
    pub fn validate(&self) -> Result<(), ProtocolFault> {
        if self.api != PLUGIN_API {
            return Err(ProtocolFault::UnsupportedApi(self.api.clone()));
        }
        if self.method.is_empty() {
            return Err(ProtocolFault::EmptyMethod);
        }
        Ok(())
    }
}

/// An open plugin-defined event body.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PluginEvent {
    /// Stable event subtype chosen by the plugin.
    #[serde(rename = "type")]
    pub kind: String,
    /// Arbitrary event fields. They remain outside the workflow's typed result.
    #[serde(flatten)]
    pub payload: Map<String, Value>,
}

/// A structured failure returned by a plugin.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PluginFailure {
    /// Machine-readable failure category.
    pub code: String,
    /// Human-readable failure summary.
    pub message: String,
    /// Optional plugin-specific evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Presence-aware JSON field. Unlike `Option<Value>`, JSON null remains present.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum JsonField<T> {
    /// The object did not contain the field.
    #[default]
    Missing,
    /// The object contained the field, possibly with a JSON-null value.
    Present(T),
}

impl<T> JsonField<T> {
    fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl<T> Serialize for JsonField<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Present(value) => value.serialize(serializer),
            Self::Missing => serializer.serialize_unit(),
        }
    }
}

impl<'de, T> Deserialize<'de> for JsonField<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Present)
    }
}

/// A streamed event or the unique terminal result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginFrame {
    /// A non-terminal progress or diagnostic event.
    Event {
        /// Request correlation identifier.
        id: RequestId,
        /// Open event body.
        event: PluginEvent,
    },
    /// The unique terminal response.
    Result {
        /// Request correlation identifier.
        id: RequestId,
        /// Whether the plugin operation succeeded.
        ok: bool,
        /// Successful result value. JSON null is a valid value.
        #[serde(default, skip_serializing_if = "JsonField::is_missing")]
        value: JsonField<Value>,
        /// Structured failure when `ok` is false.
        #[serde(default, skip_serializing_if = "JsonField::is_missing")]
        error: JsonField<PluginFailure>,
    },
}

impl PluginFrame {
    /// Return the frame's request identifier.
    #[must_use]
    pub fn id(&self) -> &RequestId {
        match self {
            Self::Event { id, .. } | Self::Result { id, .. } => id,
        }
    }
}

/// Normalized terminal result retained in a process outcome and journal summary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TerminalResult {
    /// Plugin returned a value.
    Success {
        /// Any JSON value, including null.
        value: Value,
    },
    /// Plugin returned a structured failure.
    Failure {
        /// Stable plugin error.
        error: PluginFailure,
    },
}

/// A protocol violation. These failures are deterministic and safe to report.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProtocolFault {
    /// The request uses a protocol this runtime does not understand.
    #[error("unsupported plugin api {0:?}")]
    UnsupportedApi(String),
    /// The request method was empty.
    #[error("plugin method must be non-empty")]
    EmptyMethod,
    /// A stdout line was not UTF-8.
    #[error("plugin stdout was not UTF-8 at byte {valid_up_to}")]
    InvalidUtf8 {
        /// Number of valid bytes before the invalid sequence.
        valid_up_to: usize,
    },
    /// A stdout line was not a valid frame.
    #[error("invalid plugin JSONL frame: {0}")]
    InvalidJson(String),
    /// A frame did not belong to the active request.
    #[error("plugin frame id did not match the request")]
    MismatchedId,
    /// A plugin emitted an event after completing its request.
    #[error("plugin emitted a frame after its terminal result")]
    FrameAfterTerminal,
    /// The plugin emitted more than one terminal result.
    #[error("plugin emitted more than one terminal result")]
    DuplicateTerminal,
    /// The plugin exited without a terminal result.
    #[error("plugin exited without a terminal result")]
    MissingTerminal,
    /// A result's `ok`, `value`, and `error` fields disagreed.
    #[error("invalid terminal result: {0}")]
    InvalidTerminal(String),
}

/// Incremental validator for one request's stdout frames.
#[derive(Debug)]
pub struct FrameSequence {
    expected_id: RequestId,
    terminal: Option<TerminalResult>,
    frames_seen: u64,
}

impl FrameSequence {
    /// Start validating frames for `expected_id`.
    #[must_use]
    pub fn new(expected_id: RequestId) -> Self {
        Self {
            expected_id,
            terminal: None,
            frames_seen: 0,
        }
    }

    /// Validate and remember one frame.
    pub fn accept(&mut self, frame: &PluginFrame) -> Result<(), ProtocolFault> {
        if frame.id() != &self.expected_id {
            return Err(ProtocolFault::MismatchedId);
        }
        if self.terminal.is_some() {
            return match frame {
                PluginFrame::Result { .. } => Err(ProtocolFault::DuplicateTerminal),
                PluginFrame::Event { .. } => Err(ProtocolFault::FrameAfterTerminal),
            };
        }
        if let PluginFrame::Result {
            ok, value, error, ..
        } = frame
        {
            self.terminal = Some(match (*ok, value, error) {
                (true, JsonField::Present(value), JsonField::Missing) => TerminalResult::Success {
                    value: value.clone(),
                },
                (false, JsonField::Missing, JsonField::Present(error)) => TerminalResult::Failure {
                    error: error.clone(),
                },
                (true, _, _) => {
                    return Err(ProtocolFault::InvalidTerminal(
                        "successful result requires value and forbids error".to_owned(),
                    ));
                }
                (false, _, _) => {
                    return Err(ProtocolFault::InvalidTerminal(
                        "failed result requires error and forbids value".to_owned(),
                    ));
                }
            });
        }
        self.frames_seen += 1;
        Ok(())
    }

    /// Number of accepted frames, including the terminal result.
    #[must_use]
    pub fn frames_seen(&self) -> u64 {
        self.frames_seen
    }

    /// Finish the sequence, requiring exactly one terminal result.
    pub fn finish(self) -> Result<TerminalResult, ProtocolFault> {
        self.terminal.ok_or(ProtocolFault::MissingTerminal)
    }
}

/// Decode one UTF-8 JSONL payload into a typed frame.
pub fn decode_frame(bytes: &[u8]) -> Result<PluginFrame, ProtocolFault> {
    let raw = decode_json(bytes)?;
    let object = raw
        .as_object()
        .ok_or_else(|| ProtocolFault::InvalidJson("frame must be a JSON object".to_owned()))?;
    if object.get("type").and_then(Value::as_str) == Some("result") {
        match object.get("ok").and_then(Value::as_bool) {
            Some(true) if !object.contains_key("value") => {
                return Err(ProtocolFault::InvalidTerminal(
                    "successful result is missing value".to_owned(),
                ));
            }
            Some(false) if !object.contains_key("error") => {
                return Err(ProtocolFault::InvalidTerminal(
                    "failed result is missing error".to_owned(),
                ));
            }
            Some(_) => {}
            None => {
                return Err(ProtocolFault::InvalidTerminal(
                    "result is missing boolean ok".to_owned(),
                ));
            }
        }
    }
    serde_json::from_value(raw).map_err(|error| ProtocolFault::InvalidJson(error.to_string()))
}

/// Strictly decode one request, including recursive duplicate-key rejection.
pub fn decode_request(bytes: &[u8]) -> Result<PluginRequest, ProtocolFault> {
    let value = decode_json(bytes)?;
    let request: PluginRequest = serde_json::from_value(value)
        .map_err(|error| ProtocolFault::InvalidJson(error.to_string()))?;
    request.validate()?;
    Ok(request)
}

/// Strictly decode any JSON value, including recursive duplicate-key
/// rejection. Use this at every JSON ingestion boundary before converting to
/// a narrower typed value.
pub fn decode_json(bytes: &[u8]) -> Result<Value, ProtocolFault> {
    let text = std::str::from_utf8(bytes).map_err(|error| ProtocolFault::InvalidUtf8 {
        valid_up_to: error.valid_up_to(),
    })?;
    validate_json_number_domain(text)?;
    let mut decoder = serde_json::Deserializer::from_str(text);
    let value = StrictJson::deserialize(&mut decoder)
        .map_err(|error| ProtocolFault::InvalidJson(error.to_string()))?
        .0;
    decoder
        .end()
        .map_err(|error| ProtocolFault::InvalidJson(error.to_string()))?;
    Ok(value)
}

fn validate_json_number_domain(text: &str) -> Result<(), ProtocolFault> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte != b'-' && !byte.is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len()
            && matches!(bytes[index], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        {
            index += 1;
        }
        let token = &text[start..index];
        let mantissa = token.split(['e', 'E']).next().unwrap_or(token);
        let lexically_nonzero = mantissa.bytes().any(|digit| matches!(digit, b'1'..=b'9'));
        let parsed = token.parse::<f64>();
        if parsed.as_ref().is_ok_and(|value| !value.is_finite()) {
            return Err(ProtocolFault::InvalidJson(format!(
                "JSON number is outside the finite runtime domain: {token}"
            )));
        }
        if lexically_nonzero && parsed.is_ok_and(|value| value == 0.0) {
            return Err(ProtocolFault::InvalidJson(format!(
                "JSON number underflowed to zero: {token}"
            )));
        }
        if !token.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
            let exact = if token.starts_with('-') {
                token.parse::<i64>().is_ok()
            } else {
                token.parse::<u64>().is_ok()
            };
            if !exact {
                return Err(ProtocolFault::InvalidJson(format!(
                    "JSON integer is outside the i64/u64 runtime domain: {token}"
                )));
            }
        }
    }
    Ok(())
}

struct StrictJson(Value);

impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

struct StrictJsonVisitor;

impl<'de> Visitor<'de> for StrictJsonVisitor {
    type Value = StrictJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(StrictJson)
            .ok_or_else(|| E::custom("JSON number must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Null))
    }

    fn visit_seq<A>(self, mut values: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut decoded = Vec::new();
        while let Some(value) = values.next_element::<StrictJson>()? {
            decoded.push(value.0);
        }
        Ok(StrictJson(Value::Array(decoded)))
    }

    fn visit_map<A>(self, mut values: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut decoded = Map::new();
        while let Some(key) = values.next_key::<String>()? {
            if decoded.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate object key {key:?}")));
            }
            let value = values.next_value::<StrictJson>()?;
            decoded.insert(key, value.0);
        }
        Ok(StrictJson(Value::Object(decoded)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_unicode_event_and_terminal() {
        let event = decode_frame(
            br#"{"type":"event","id":"r1","event":{"type":"progress","message":"\u5b54\u6d1e"}}"#,
        )
        .expect("event");
        let terminal =
            decode_frame(br#"{"type":"result","id":"r1","ok":true,"value":{"holes":2}}"#)
                .expect("terminal");
        let mut sequence = FrameSequence::new(RequestId::Text("r1".to_owned()));
        sequence.accept(&event).expect("accept event");
        sequence.accept(&terminal).expect("accept terminal");
        assert_eq!(sequence.frames_seen(), 2);
        assert_eq!(
            sequence.finish().expect("finish"),
            TerminalResult::Success {
                value: serde_json::json!({"holes": 2})
            }
        );
    }

    #[test]
    fn rejects_frames_after_terminal() {
        let terminal =
            decode_frame(br#"{"type":"result","id":7,"ok":true,"value":null}"#).expect("terminal");
        let mut sequence = FrameSequence::new(RequestId::Integer(7));
        sequence.accept(&terminal).expect("first terminal");
        assert_eq!(
            sequence.accept(&terminal),
            Err(ProtocolFault::DuplicateTerminal)
        );
    }

    #[test]
    fn requires_terminal_payload_field() {
        assert!(matches!(
            decode_frame(br#"{"type":"result","id":"x","ok":true}"#),
            Err(ProtocolFault::InvalidTerminal(_))
        ));
    }

    #[test]
    fn rejects_duplicate_keys_recursively_in_frames_and_requests() {
        assert!(matches!(
            decode_frame(
                br#"{"type":"event","id":"x","event":{"type":"a","type":"b"}}"#
            ),
            Err(ProtocolFault::InvalidJson(message)) if message.contains("duplicate")
        ));
        assert!(matches!(
            decode_request(
                br#"{"api":"agenstro.plugin/v1","id":"x","method":"a","method":"b","params":{}}"#
            ),
            Err(ProtocolFault::InvalidJson(message)) if message.contains("duplicate")
        ));
    }

    #[test]
    fn rejects_overflow_underflow_and_duplicate_numbers_without_touching_strings() {
        for invalid in [
            br#"{"value":1e999}"#.as_slice(),
            br#"{"value":1e-999}"#.as_slice(),
            br#"{"value":18446744073709551616}"#.as_slice(),
            br#"{"value":-9223372036854775809}"#.as_slice(),
            br#"{"value":1e-10000,"value":0}"#.as_slice(),
        ] {
            assert!(
                matches!(decode_json(invalid), Err(ProtocolFault::InvalidJson(_))),
                "accepted {}",
                String::from_utf8_lossy(invalid)
            );
        }
        assert_eq!(
            decode_json(
                br#"{"zero":0e-999,"negativeZero":-0,"min":-9223372036854775808,"max":18446744073709551615,"text":"1e-999"}"#
            )
            .expect("valid numeric boundaries"),
            serde_json::json!({
                "zero":0.0,
                "negativeZero":-0.0,
                "min":i64::MIN,
                "max":u64::MAX,
                "text":"1e-999"
            })
        );
    }
}
