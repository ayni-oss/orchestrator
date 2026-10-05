//! Public publication request mapping through the explicit retained-payload codec.
use super::v1;
use ayni_orchestrator_application::publication::PublishCandidate;

pub fn publication_command(key: &str, request: &v1::PublishCandidateRequest) -> PublishCandidate {
    crate::adapters::sqlite::publication::publication_command(key, request)
}
