use super::*;

#[test]
fn environment_proposals_require_visible_impact_before_review() -> Result<(), CleanStoreError> {
    let (mut store, c, run, profile) = setup()?;
    store.send_project_message("environment-message", &message(&c), run.clone())?;
    let scope = ayni_orchestrator_domain::EnvironmentChangeAuthorization {
        dependency_paths: vec!["Cargo.toml".into(), "Cargo.lock".into()],
    };
    assert!(matches!(
        complete_with_environment(&mut store, &c, &run, &profile, Some(scope.clone()), ""),
        Err(CleanStoreError::InvalidTransition)
    ));
    assert!(store.project_conversation(&c.id)?.proposals.is_empty());
    complete_with_environment(
        &mut store,
        &c,
        &run,
        &profile,
        Some(scope),
        "Update native dependencies and regenerate .ayni.lock",
    )?;
    let reviewed = store.project_conversation(&c.id)?;
    let constraints = reviewed.proposals[0]
        .request
        .as_ref()
        .and_then(|r| r.change_constraints.as_ref())
        .ok_or(CleanStoreError::CorruptState)?;
    assert_eq!(
        constraints
            .environment_change
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .dependency_paths,
        ["Cargo.toml", "Cargo.lock"]
    );
    assert!(
        !reviewed
            .plan
            .as_ref()
            .ok_or(CleanStoreError::CorruptState)?
            .versions[0]
            .dependency_environment_impact
            .is_empty()
    );
    Ok(())
}
