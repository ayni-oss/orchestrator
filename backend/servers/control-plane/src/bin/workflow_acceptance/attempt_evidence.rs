//! Compare retained authority while excluding only the live observation clock.
use ayni_orchestrator_api::v1;
use prost::Message as _;

pub fn attempt(value: &v1::TaskAttempt) -> v1::TaskAttempt {
    let mut stable = value.clone();
    if let Some(timing) = stable.timing.as_mut() {
        timing.observed_at = None;
    }
    stable
}

pub fn response_bytes(value: &v1::GetTaskAttemptResponse) -> Vec<u8> {
    let mut stable = value.clone();
    stable.attempt = stable.attempt.as_ref().map(attempt);
    stable.encode_to_vec()
}

pub fn retained_bytes(bytes: &[u8]) -> super::Result<Vec<u8>> {
    Ok(response_bytes(&v1::GetTaskAttemptResponse::decode(bytes)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_orchestrator_api::Timestamp;

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp { seconds, nanos: 0 }
    }

    #[test]
    fn only_observation_clock_is_excluded_from_authority_evidence() -> super::super::Result {
        let original = v1::GetTaskAttemptResponse {
            attempt: Some(v1::TaskAttempt {
                attempt_id: "retained-attempt".into(),
                generation: 2,
                timing: Some(v1::TaskExecutionTiming {
                    observed_at: Some(timestamp(10)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        let mut later = original.clone();
        later
            .attempt
            .as_mut()
            .ok_or("fixture field missing")?
            .timing
            .as_mut()
            .ok_or("fixture field missing")?
            .observed_at = Some(timestamp(20));
        assert_ne!(original.encode_to_vec(), later.encode_to_vec());
        assert_eq!(response_bytes(&original), response_bytes(&later));
        assert_eq!(
            retained_bytes(&original.encode_to_vec())?,
            response_bytes(&later)
        );
        for index in 0..8 {
            let mut changed = later.clone();
            let timing = changed
                .attempt
                .as_mut()
                .ok_or("fixture field missing")?
                .timing
                .as_mut()
                .ok_or("fixture field missing")?;
            let fields = [
                &mut timing.queue_accepted_at,
                &mut timing.claim_accepted_at,
                &mut timing.coding_result_accepted_at,
                &mut timing.quality_result_accepted_at,
                &mut timing.execution_stopped_at,
                &mut timing.publication_authorized_at,
                &mut timing.publication_claim_accepted_at,
                &mut timing.publication_result_accepted_at,
            ];
            *fields
                .into_iter()
                .nth(index)
                .ok_or("fixture field missing")? = Some(timestamp(30));
            assert_ne!(
                response_bytes(&original),
                response_bytes(&changed),
                "boundary {index}"
            );
        }
        let mut changed = later.clone();
        changed
            .attempt
            .as_mut()
            .ok_or("fixture field missing")?
            .generation += 1;
        assert_ne!(response_bytes(&original), response_bytes(&changed));
        changed = later;
        changed
            .attempt
            .as_mut()
            .ok_or("fixture field missing")?
            .timing = None;
        assert_ne!(response_bytes(&original), response_bytes(&changed));
        Ok(())
    }
}
