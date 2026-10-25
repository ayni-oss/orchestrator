//! Bounded SSE decoding. Only public assistant text is eligible for preview.
use crate::responses::{ResponseError, ResponseEvidence};
use serde_json::{Value, json};

const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = 256 * 1024;

pub struct Decoder {
    bytes: Vec<u8>,
    total: usize,
    provider: String,
    text: String,
    message: Option<Value>,
    arguments: Vec<String>,
    open_block: Option<usize>,
    terminal: Option<ResponseEvidence>,
}

impl Decoder {
    pub fn new(provider: &str) -> Self {
        Self {
            bytes: Vec::new(),
            total: 0,
            provider: provider.into(),
            text: String::new(),
            message: None,
            arguments: Vec::new(),
            open_block: None,
            terminal: None,
        }
    }

    pub fn push(
        &mut self,
        chunk: &[u8],
        on_text: &mut impl FnMut(&str) -> Result<(), ResponseError>,
    ) -> Result<(), ResponseError> {
        self.total = self
            .total
            .checked_add(chunk.len())
            .ok_or(ResponseError::Uncertain)?;
        if self.total > MAX_STREAM_BYTES {
            return Err(ResponseError::Uncertain);
        }
        self.bytes.extend_from_slice(chunk);
        loop {
            let boundary = [
                self.bytes
                    .windows(2)
                    .position(|w| w == b"\n\n")
                    .map(|n| (n, 2)),
                self.bytes
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|n| (n, 4)),
            ]
            .into_iter()
            .flatten()
            .min_by_key(|(position, _)| *position);
            let Some((end, delimiter)) = boundary else {
                break;
            };
            if end > MAX_FRAME_BYTES {
                return Err(ResponseError::Uncertain);
            }
            let frame = self.bytes.drain(..end + delimiter).collect::<Vec<_>>();
            self.frame(&frame[..end], on_text)?;
        }
        if self.bytes.len() > MAX_FRAME_BYTES {
            return Err(ResponseError::Uncertain);
        }
        Ok(())
    }

    fn frame(
        &mut self,
        frame: &[u8],
        on_text: &mut impl FnMut(&str) -> Result<(), ResponseError>,
    ) -> Result<(), ResponseError> {
        let frame = std::str::from_utf8(frame).map_err(|_| ResponseError::Uncertain)?;
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            return Ok(());
        }
        let value: Value = serde_json::from_str(&data).map_err(|_| ResponseError::Uncertain)?;
        if self.terminal.is_some() {
            return Err(ResponseError::Uncertain);
        }
        if self.provider == "anthropic" {
            self.anthropic(&value, on_text)
        } else {
            self.openai(&value, on_text)
        }
    }

    fn append(
        &mut self,
        delta: &str,
        on_text: &mut impl FnMut(&str) -> Result<(), ResponseError>,
    ) -> Result<(), ResponseError> {
        if self.text.len() + delta.len() > 16384
            || delta
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(ResponseError::Uncertain);
        }
        self.text.push_str(delta);
        on_text(&self.text)
    }

    fn openai(
        &mut self,
        value: &Value,
        on_text: &mut impl FnMut(&str) -> Result<(), ResponseError>,
    ) -> Result<(), ResponseError> {
        match value["type"].as_str() {
            Some("response.output_text.delta") => self.append(
                value["delta"].as_str().ok_or(ResponseError::Uncertain)?,
                on_text,
            ),
            Some("response.completed" | "response.incomplete" | "response.failed") => {
                self.terminal = Some(crate::responses::parse_response(
                    &serde_json::to_vec(&value["response"])
                        .map_err(|_| ResponseError::Uncertain)?,
                )?);
                Ok(())
            }
            Some("error") => Err(ResponseError::Uncertain),
            // Reasoning, tool arguments and unknown future metadata are never previewed.
            _ => Ok(()),
        }
    }

    fn anthropic(
        &mut self,
        value: &Value,
        on_text: &mut impl FnMut(&str) -> Result<(), ResponseError>,
    ) -> Result<(), ResponseError> {
        match value["type"].as_str() {
            Some("message_start") => {
                if self.message.is_some() {
                    return Err(ResponseError::Uncertain);
                }
                self.message = Some(value["message"].clone());
            }
            Some("content_block_start") => {
                if self.open_block.is_some() {
                    return Err(ResponseError::Uncertain);
                }
                let blocks = self
                    .message
                    .as_mut()
                    .and_then(|m| m["content"].as_array_mut())
                    .ok_or(ResponseError::Uncertain)?;
                if value["index"].as_u64() != Some(blocks.len() as u64) || blocks.len() >= 16 {
                    return Err(ResponseError::Uncertain);
                }
                if !matches!(
                    value["content_block"]["type"].as_str(),
                    Some("text" | "tool_use")
                ) {
                    return Err(ResponseError::Uncertain);
                }
                self.open_block = Some(blocks.len());
                blocks.push(value["content_block"].clone());
                self.arguments.push(String::new());
            }
            Some("content_block_delta") => self.block_delta(value, on_text)?,
            Some("content_block_stop") => self.stop_block(value)?,
            Some("message_delta") => self.message_delta(value)?,
            Some("message_stop") => {
                if self.open_block.is_some() {
                    return Err(ResponseError::Uncertain);
                }
                let message = self.message.as_ref().ok_or(ResponseError::Uncertain)?;
                self.terminal = Some(crate::anthropic::response(
                    &serde_json::to_vec(message).map_err(|_| ResponseError::Uncertain)?,
                )?);
            }
            Some("error") => return Err(ResponseError::Uncertain),
            _ => {}
        }
        Ok(())
    }

    fn block_delta(
        &mut self,
        value: &Value,
        on_text: &mut impl FnMut(&str) -> Result<(), ResponseError>,
    ) -> Result<(), ResponseError> {
        let index = value["index"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(ResponseError::Uncertain)?;
        if self.open_block != Some(index) {
            return Err(ResponseError::Uncertain);
        }
        let block = self
            .message
            .as_mut()
            .and_then(|m| m["content"].as_array_mut())
            .and_then(|blocks| blocks.get_mut(index))
            .ok_or(ResponseError::Uncertain)?;
        match value["delta"]["type"].as_str() {
            Some("text_delta") if block["type"] == "text" => {
                let text = value["delta"]["text"]
                    .as_str()
                    .ok_or(ResponseError::Uncertain)?;
                let target = block["text"]
                    .as_str()
                    .ok_or(ResponseError::Uncertain)?
                    .to_owned()
                    + text;
                block["text"] = json!(target);
                self.append(text, on_text)?;
            }
            Some("input_json_delta") if block["type"] == "tool_use" => {
                let input = self
                    .arguments
                    .get_mut(index)
                    .ok_or(ResponseError::Uncertain)?;
                input.push_str(
                    value["delta"]["partial_json"]
                        .as_str()
                        .ok_or(ResponseError::Uncertain)?,
                );
                if input.len() > 40960 {
                    return Err(ResponseError::Uncertain);
                }
            }
            _ => return Err(ResponseError::Uncertain),
        }
        Ok(())
    }

    fn stop_block(&mut self, value: &Value) -> Result<(), ResponseError> {
        let index = value["index"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(ResponseError::Uncertain)?;
        if self.open_block != Some(index) {
            return Err(ResponseError::Uncertain);
        }
        let block = self
            .message
            .as_mut()
            .and_then(|m| m["content"].as_array_mut())
            .and_then(|blocks| blocks.get_mut(index))
            .ok_or(ResponseError::Uncertain)?;
        if block["type"] == "tool_use" && !self.arguments[index].is_empty() {
            block["input"] = serde_json::from_str(&self.arguments[index])
                .map_err(|_| ResponseError::Uncertain)?;
        }
        self.open_block = None;
        Ok(())
    }

    fn message_delta(&mut self, value: &Value) -> Result<(), ResponseError> {
        if self.open_block.is_some() {
            return Err(ResponseError::Uncertain);
        }
        let message = self.message.as_mut().ok_or(ResponseError::Uncertain)?;
        if let Some(delta) = value["delta"].as_object() {
            for (key, value) in delta {
                message[key] = value.clone();
            }
        }
        if let Some(usage) = value["usage"].as_object() {
            for (key, value) in usage {
                message["usage"][key] = value.clone();
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<ResponseEvidence, ResponseError> {
        if !self.bytes.iter().all(u8::is_ascii_whitespace) {
            return Err(ResponseError::Uncertain);
        }
        self.terminal.ok_or(ResponseError::Uncertain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(value: &Value) -> Vec<u8> {
        format!("data: {value}\n\n").into_bytes()
    }
    fn completed() -> Value {
        json!({"type":"response.completed","response":{"id":"r1","model":"fixture","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"héllo"}]}],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}})
    }
    #[test]
    fn fragmented_utf8_preview_requires_terminal_evidence_and_omits_reasoning()
    -> Result<(), ResponseError> {
        let mut decoder = Decoder::new("openai");
        let mut observations = Vec::new();
        let mut observe = |text: &str| {
            observations.push(text.to_owned());
            Ok(())
        };
        let events = [
            event(&json!({"type":"response.reasoning_text.delta","delta":"private reasoning"})),
            event(&json!({"type":"response.output_text.delta","delta":"hé"})),
            event(&json!({"type":"response.output_text.delta","delta":"llo"})),
            event(&completed()),
        ]
        .concat();
        for byte in events {
            decoder.push(&[byte], &mut observe)?;
        }
        let accepted = decoder.finish()?;
        assert_eq!(observations, ["hé", "héllo"]);
        assert_eq!(accepted.content, "héllo");
        assert_eq!(accepted.usage.total_tokens, Some(3));
        Ok(())
    }
    #[test]
    fn unfinished_anthropic_tool_block_cannot_become_execution_evidence() {
        let values = [
            json!({"type":"message_start","message":{"type":"message","role":"assistant","id":"m1","model":"fixture","content":[],"usage":{}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"read_project_status","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"scope\":"}}),
            json!({"type":"message_stop"}),
        ];
        let mut decoder = Decoder::new("anthropic");
        let result = values
            .iter()
            .try_for_each(|value| decoder.push(&event(value), &mut |_| Ok(())));
        assert_eq!(result.err(), Some(ResponseError::Uncertain));
    }
    #[test]
    fn mixed_line_endings_use_the_first_frame_boundary() -> Result<(), ResponseError> {
        let mut decoder = Decoder::new("openai");
        let first = format!(
            "data: {}\r\n\r\n",
            json!({"type":"response.output_text.delta","delta":"héllo"})
        );
        let combined = [first.as_bytes(), &event(&completed())].concat();
        let mut preview = String::new();
        for chunk in combined.chunks(3) {
            decoder.push(chunk, &mut |text| {
                preview = text.into();
                Ok(())
            })?;
        }
        assert_eq!(decoder.finish()?.content, "héllo");
        assert_eq!(preview, "héllo");
        Ok(())
    }
    #[test]
    fn interrupted_malformed_and_oversized_streams_are_uncertain() {
        for bytes in [
            event(&json!({"type":"response.output_text.delta","delta":"unfinished"})),
            b"data: malformed\n\n".to_vec(),
            event(&json!({"type":"response.output_text.delta","delta":"x".repeat(16385)})),
        ] {
            let mut decoder = Decoder::new("openai");
            let result = decoder
                .push(&bytes, &mut |_| Ok(()))
                .and_then(|()| decoder.finish());
            assert_eq!(result.err(), Some(ResponseError::Uncertain));
        }
    }
    #[test]
    fn anthropic_text_and_tool_arguments_reconstruct_only_after_message_stop()
    -> Result<(), ResponseError> {
        let mut decoder = Decoder::new("anthropic");
        let mut preview = String::new();
        let values = [
            json!({"type":"message_start","message":{"type":"message","role":"assistant","id":"m1","model":"fixture","content":[],"usage":{"input_tokens":4,"output_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Public"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool1","name":"read_project_status","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":3}}),
            json!({"type":"message_stop"}),
        ];
        for value in values {
            decoder.push(&event(&value), &mut |text| {
                preview = text.into();
                Ok(())
            })?;
        }
        let accepted = decoder.finish()?;
        assert_eq!(preview, "Public");
        assert_eq!(accepted.tool_calls[0].arguments, "{}");
        assert_eq!(accepted.usage.total_tokens, Some(7));
        Ok(())
    }
}
