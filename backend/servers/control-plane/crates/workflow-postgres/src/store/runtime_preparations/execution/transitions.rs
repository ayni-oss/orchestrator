use super::*;
pub(in crate::store::runtime_preparations) struct Transition {
    pub stage: String,
    pub state: String,
    pub terminal: String,
    pub reason: String,
}
fn advance(stage: &str) -> Transition {
    Transition {
        stage: stage.into(),
        state: if stage == "cleanup" {
            "cleanup_pending"
        } else {
            "pending"
        }
        .into(),
        terminal: if stage == "cleanup" { "ready" } else { "" }.into(),
        reason: String::new(),
    }
}
fn cleanup(state: &str, reason: &str) -> Transition {
    Transition {
        stage: "cleanup".into(),
        state: "cleanup_pending".into(),
        terminal: state.into(),
        reason: reason.into(),
    }
}
fn required(value: &str) -> Result<(), StoreError> {
    if validation::sha(value) {
        Ok(())
    } else {
        Err(StoreError::InvalidInput("required runtime evidence"))
    }
}
fn evidence_scope(stage: &str, e: &Evidence) -> Result<(), StoreError> {
    let invalid = (!e.managed_check_digest.is_empty() && stage != "verify_compatibility")
        || (!e.unsigned_manifest_digest.is_empty() && stage != "freeze_candidate")
        || (!e.certification_layer_digest.is_empty() && stage != "certify")
        || (!e.source_digest.is_empty() && stage != "acquire_source")
        || (!e.protected_content_root.is_empty()
            && !matches!(stage, "freeze_candidate" | "verify_content"))
        || (!e.certificate_digest.is_empty() && !matches!(stage, "certify" | "verify_content"))
        || (!e.expected_manifest_digest.is_empty() && !matches!(stage, "finalize" | "publish"))
        || (!e.candidate_image.is_empty()
            && !matches!(
                stage,
                "discover" | "publish" | "pull_existing" | "pull_final"
            ))
        || (!e.image_digest.is_empty()
            && !matches!(
                stage,
                "discover"
                    | "publish"
                    | "pull_existing"
                    | "pull_final"
                    | "verify_content"
                    | "verify_compatibility"
            ));
    if invalid {
        Err(StoreError::InvalidInput("stage evidence scope"))
    } else {
        Ok(())
    }
}
pub(in crate::store::runtime_preparations) fn accept_evidence(
    op: &mut Operation,
    report: &StageReport,
) -> Result<(), StoreError> {
    let e = &report.evidence;
    evidence_scope(&op.stage, e)?;
    if !e.details.is_empty()
        || e.content_verified && op.stage != "verify_content"
        || e.managed_compatible && op.stage != "verify_compatibility"
        || e.cleanup_complete && op.stage != "cleanup"
    {
        return Err(StoreError::InvalidInput("stage evidence scope"));
    }
    if !e.artifact_reference.is_empty() {
        let expected = format!(
            "runtime://{}/{}/{}/{}",
            op.id,
            op.stage,
            op.generation,
            e.artifact_digest.trim_start_matches("sha256:")
        );
        if e.artifact_reference != expected {
            return Err(StoreError::InvalidInput("runtime artifact reference"));
        }
    }
    if !e.candidate_image.is_empty()
        && !validation::image(&e.candidate_image, Some(&op.frozen.registry_destination))
    {
        return Err(StoreError::InvalidInput("runtime image destination"));
    }
    if report.outcome != "completed" {
        return Ok(());
    }
    validate_completed(op, e)?;
    merge_evidence(op, e)
}
fn merge_evidence(op: &mut Operation, e: &Evidence) -> Result<(), StoreError> {
    let old = &mut op.evidence;
    for (previous, incoming) in [
        (&mut old.protected_content_root, &e.protected_content_root),
        (&mut old.managed_check_digest, &e.managed_check_digest),
        (
            &mut old.unsigned_manifest_digest,
            &e.unsigned_manifest_digest,
        ),
        (
            &mut old.certification_layer_digest,
            &e.certification_layer_digest,
        ),
        (&mut old.certificate_digest, &e.certificate_digest),
        (
            &mut old.expected_manifest_digest,
            &e.expected_manifest_digest,
        ),
    ] {
        if !incoming.is_empty() {
            if !previous.is_empty() && *previous != *incoming {
                return Err(StoreError::InvalidInput(
                    "changed immutable runtime evidence",
                ));
            }
            previous.clone_from(incoming);
        }
    }
    if !e.source_digest.is_empty() {
        old.source_digest.clone_from(&e.source_digest);
    }
    if !e.artifact_digest.is_empty() {
        old.artifact_digest.clone_from(&e.artifact_digest);
    }
    if !e.artifact_reference.is_empty() {
        old.artifact_reference.clone_from(&e.artifact_reference);
    }
    if !e.candidate_image.is_empty() {
        if !old.candidate_image.is_empty()
            && old.candidate_image != e.candidate_image
            && op.stage != "publish"
        {
            return Err(StoreError::InvalidInput("changed candidate image"));
        }
        old.candidate_image.clone_from(&e.candidate_image);
    }
    if !e.image_digest.is_empty() {
        let expected = old.expected_manifest_digest.as_str();
        if !expected.is_empty() && expected != e.image_digest {
            return Err(StoreError::InvalidInput(
                "image digest differs from publication",
            ));
        }
        old.image_digest.clone_from(&e.image_digest);
    }
    if old.image_digest.is_empty() && !old.candidate_image.is_empty() {
        old.image_digest = old
            .candidate_image
            .rsplit_once('@')
            .map(|(_, d)| d.to_owned())
            .unwrap_or_default();
    }
    if op.stage == "publish" {
        old.candidate_image = format!("{}@{}", op.frozen.registry_destination, old.image_digest);
    }
    if !old.candidate_image.is_empty()
        && !old.image_digest.is_empty()
        && old
            .candidate_image
            .rsplit_once('@')
            .is_none_or(|(_, hash)| hash != old.image_digest)
    {
        return Err(StoreError::InvalidInput("candidate manifest identity"));
    }
    old.content_verified |= e.content_verified;
    old.managed_compatible |= e.managed_compatible;
    old.cleanup_complete |= e.cleanup_complete;
    Ok(())
}
pub(in crate::store::runtime_preparations) fn after_stop(
    op: &Operation,
    report: Option<&StageReport>,
    terminal: &str,
) -> Result<Transition, StoreError> {
    if op.state == "cancelling" {
        return Ok(cleanup(
            if terminal.is_empty() {
                "cancelled"
            } else {
                terminal
            },
            &op.safe_reason,
        ));
    }
    let Some(report) = report else {
        return Ok(cleanup(
            if op.stage == "cleanup" && !terminal.is_empty() {
                terminal
            } else {
                "unavailable"
            },
            "workload_interrupted",
        ));
    };
    if op.stage == "cleanup" {
        return complete_cleanup(op, report, terminal);
    }
    if report.outcome == "missing" && op.stage == "discover" {
        if !op.evidence.expected_manifest_digest.is_empty() {
            return Ok(cleanup("publication_uncertain", "publication_uncertain"));
        }
        return Ok(if op.allow_build {
            advance("build")
        } else {
            cleanup("blocked", "image_missing")
        });
    }
    if report.outcome == "invalid" && op.stage == "verify_content" {
        return Ok(if op.allow_replace && op.allow_build {
            advance("build")
        } else {
            cleanup("blocked", "replacement_required")
        });
    }
    if report.outcome != "completed" {
        return Ok(failed_stage(op, report));
    }
    let next = match op.stage.as_str() {
        "acquire_source" => "discover",
        "discover" => "pull_existing",
        "pull_existing" | "pull_final" => "verify_content",
        "build" => "freeze_candidate",
        "freeze_candidate" => "certify",
        "certify" => "finalize",
        "finalize" => "publish",
        "publish" => "pull_final",
        "verify_content" => "verify_compatibility",
        "verify_compatibility" => "cleanup",
        _ => return Err(StoreError::InvalidInput("runtime transition")),
    };
    Ok(advance(next))
}

fn validate_completed(op: &Operation, e: &Evidence) -> Result<(), StoreError> {
    match op.stage.as_str() {
        "acquire_source" => required(&e.source_digest)?,
        "discover" => {
            if e.candidate_image.is_empty() {
                return Err(StoreError::InvalidInput("discovered image"));
            }
        }
        "build" => required(&e.artifact_digest)?,
        "freeze_candidate" => validate_frozen_candidate(op, e)?,
        "certify" => {
            required(&e.certificate_digest)?;
            required(&e.certification_layer_digest)?;
            if e.certification_layer_digest != e.artifact_digest {
                return Err(StoreError::InvalidInput("certification layer identity"));
            }
        }
        "finalize" => required(&e.expected_manifest_digest)?,
        "publish" => {
            required(&op.evidence.expected_manifest_digest)?;
            if op.evidence.expected_manifest_digest != e.image_digest {
                return Err(StoreError::InvalidInput("published manifest identity"));
            }
        }
        "verify_content" => {
            if !e.content_verified {
                return Err(StoreError::InvalidInput("content verification evidence"));
            }
            required(&e.protected_content_root)?;
            required(&e.certificate_digest)?;
        }
        "verify_compatibility" => {
            required(&e.managed_check_digest)?;
            if !op.evidence.content_verified || !e.managed_compatible {
                return Err(StoreError::InvalidInput("managed compatibility evidence"));
            }
        }
        "cleanup" => {
            if !e.cleanup_complete {
                return Err(StoreError::InvalidInput("cleanup evidence"));
            }
        }
        "pull_existing" | "pull_final" => {}
        _ => return Err(StoreError::InvalidInput("runtime stage")),
    }
    Ok(())
}

fn complete_cleanup(
    op: &Operation,
    report: &StageReport,
    terminal: &str,
) -> Result<Transition, StoreError> {
    Ok(
        if report.outcome == "completed" && op.evidence.cleanup_complete {
            let state = if terminal.is_empty() {
                "failed"
            } else {
                terminal
            };
            if state == "ready"
                && (!op.evidence.content_verified
                    || !op.evidence.managed_compatible
                    || !validation::sha(&op.evidence.image_digest)
                    || !validation::sha(&op.evidence.managed_check_digest))
            {
                return Err(StoreError::InvalidInput("incomplete runtime admission"));
            }
            Transition {
                stage: "cleanup".into(),
                state: state.into(),
                terminal: terminal.into(),
                reason: op.safe_reason.clone(),
            }
        } else {
            cleanup(
                if terminal.is_empty() {
                    "failed"
                } else {
                    terminal
                },
                "cleanup_pending",
            )
        },
    )
}

fn failed_stage(op: &Operation, report: &StageReport) -> Transition {
    let state = match report.outcome.as_str() {
        "unavailable" => "unavailable",
        "publication_uncertain" if op.stage == "publish" => "publication_uncertain",
        "invalid" => "blocked",
        _ => "failed",
    };
    cleanup(
        state,
        if report.safe_reason.is_empty() {
            "stage_failed"
        } else {
            &report.safe_reason
        },
    )
}

fn validate_frozen_candidate(op: &Operation, e: &Evidence) -> Result<(), StoreError> {
    required(&e.unsigned_manifest_digest)?;
    if e.unsigned_manifest_digest != e.artifact_digest {
        return Err(StoreError::InvalidInput("unsigned manifest identity"));
    }
    required(&e.protected_content_root)?;
    required(&e.artifact_digest)?;
    if op.evidence.artifact_digest != e.artifact_digest {
        return Err(StoreError::InvalidInput("frozen candidate identity"));
    }
    Ok(())
}
