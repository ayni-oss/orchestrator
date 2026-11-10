//! The build child has no stage capability or registry credentials. Retain its
//! sanitized output for operators, with room reserved for the final error.
use crate::client::WorkerClient;
use ayni_orchestrator_events::execution_diagnostics::{
    BuildOutput, MAX_OUTPUT_CHUNK_BYTES, OUTPUT_END_SEQUENCE, OUTPUT_PREFIX_CHUNKS,
    OUTPUT_TAIL_CHUNKS, OutputStream,
};
use std::{collections::VecDeque, mem, time::Duration};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, BufReader};

struct Capture<'a> {
    client: &'a WorkerClient,
    stream: OutputStream,
    next: u16,
    tail: VecDeque<String>,
    pending: String,
    truncated: bool,
    delivered: bool,
    private_key: bool,
}

impl Capture<'_> {
    async fn send(&mut self, text: String) {
        self.delivered &= self
            .client
            .build_output(BuildOutput {
                stream: self.stream,
                sequence: self.next,
                text,
                complete: false,
                truncated: false,
            })
            .await;
        self.next += 1;
    }

    async fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let text = mem::take(&mut self.pending);
        if self.next < OUTPUT_PREFIX_CHUNKS {
            self.send(text).await;
        } else {
            if self.tail.len() == usize::from(OUTPUT_TAIL_CHUNKS) {
                self.tail.pop_front();
                self.truncated = true;
            }
            self.tail.push_back(text);
        }
    }

    async fn push_line(&mut self, line: &[u8], oversized: bool) {
        let (text, omitted) = sanitize(line, oversized, &mut self.private_key);
        self.truncated |= omitted;
        if self.pending.len() + text.len() > MAX_OUTPUT_CHUNK_BYTES {
            self.flush().await;
        }
        self.pending.push_str(&text);
    }
}

pub(super) async fn capture(
    client: &WorkerClient,
    stream: OutputStream,
    pipe: impl AsyncRead + Unpin,
) {
    let mut output = Capture {
        client,
        stream,
        next: 0,
        tail: VecDeque::new(),
        pending: String::new(),
        truncated: false,
        delivered: true,
        private_key: false,
    };
    let mut reader = BufReader::new(pipe);
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut line = Vec::with_capacity(MAX_OUTPUT_CHUNK_BYTES);
    let mut oversized = false;
    let eof = loop {
        tokio::select! {
            _ = interval.tick() => output.flush().await,
            result = reader.fill_buf() => match result {
                Ok([]) => break true,
                Err(_) => break false,
                Ok(bytes) => {
                    let newline = bytes.iter().position(|&byte| byte == b'\n');
                    let count = newline.map_or(bytes.len(), |index| index + 1);
                    if line.len() + count <= MAX_OUTPUT_CHUNK_BYTES && !oversized {
                        line.extend_from_slice(&bytes[..count]);
                    } else {
                        oversized = true;
                    }
                    reader.consume(count);
                    if newline.is_some() {
                        output.push_line(&line, oversized).await;
                        line.clear();
                        oversized = false;
                    }
                }
            }
        }
    };
    if eof && (!line.is_empty() || oversized) {
        output.push_line(&line, oversized).await;
    }
    output.flush().await;
    while let Some(text) = output.tail.pop_front() {
        output.send(text).await;
    }
    let _ = client
        .build_output(BuildOutput {
            stream,
            sequence: OUTPUT_END_SEQUENCE,
            text: String::new(),
            complete: eof && output.delivered,
            truncated: output.truncated,
        })
        .await;
}

fn sanitize(line: &[u8], oversized: bool, private_key: &mut bool) -> (String, bool) {
    let line = String::from_utf8_lossy(line);
    let lower = line.to_ascii_lowercase();
    let begins_key = lower.contains("-----begin ") && lower.contains("private key-----");
    let ends_key = lower.contains("-----end ") && lower.contains("private key-----");
    let key_material = *private_key || begins_key;
    *private_key = key_material && !ends_key;
    let credential = key_material
        || [
            "github_pat_",
            "ghp_",
            "gho_",
            "ghs_",
            "ghu_",
            "ghr_",
            "password:",
            "token:",
            "client_secret",
            "password=",
            "token=",
            "secret=",
            "api_key=",
            "apikey=",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
        || lower.split("://").skip(1).any(|rest| {
            rest.split(['/', '?', '#', ' ', '\t', '\n'])
                .next()
                .is_some_and(|host| host.contains('@'))
        });
    if oversized {
        return ("[oversized output line omitted]\n".into(), true);
    }
    if credential {
        return ("[credential-like output line redacted]\n".into(), false);
    }
    let text = ayni_orchestrator_observability::redact(&line)
        .chars()
        .filter(|&ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect::<String>();
    if text.len() > MAX_OUTPUT_CHUNK_BYTES {
        return ("[oversized output line omitted]\n".into(), true);
    }
    (text, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_build_errors_and_drops_credential_like_and_oversized_lines() {
        let mut private_key = false;
        assert_eq!(
            sanitize(b"Error: package unavailable\n", false, &mut private_key),
            ("Error: package unavailable\n".into(), false)
        );
        assert_eq!(
            sanitize(b"token=example\n", false, &mut private_key),
            ("[credential-like output line redacted]\n".into(), false)
        );
        assert_eq!(
            sanitize(
                b"https://name:password@example.test/path\n",
                false,
                &mut private_key
            ),
            ("[credential-like output line redacted]\n".into(), false)
        );
        assert_eq!(
            sanitize(b"partial", true, &mut private_key),
            ("[oversized output line omitted]\n".into(), true)
        );
    }

    #[test]
    fn redacts_private_key_bodies_across_lines() {
        let mut private_key = false;
        for line in [
            "-----BEGIN PRIVATE KEY-----",
            "encoded-key-material",
            "-----END PRIVATE KEY-----",
        ] {
            assert_eq!(
                sanitize(line.as_bytes(), false, &mut private_key).0,
                "[credential-like output line redacted]\n",
            );
        }
        assert_eq!(sanitize(b"failure", false, &mut private_key).0, "failure");
    }
}
