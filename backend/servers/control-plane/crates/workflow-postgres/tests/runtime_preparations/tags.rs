use super::*;

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn runtime_tags_identify_architecture_and_lock_without_operation_ids() -> TestResult {
    for architecture in ["arm64", "amd64"] {
        let f = Fixture::new().await?;
        let platform = format!("linux/{architecture}");
        let mut freeze = f.freeze.clone();
        let mut profile: RuntimeProfile = serde_json::from_str(&freeze.profile_json)?;
        profile.platform = platform.clone();
        freeze.profile_json = serde_json::to_string(&profile)?;
        let first_key = key();
        let first = f
            .store
            .start_runtime_preparation(
                Start {
                    platform: &platform,
                    ..f.input(&first_key)
                },
                freeze.clone(),
            )
            .await?;
        let next_key = key();
        let next = f
            .store
            .start_runtime_preparation(
                Start {
                    platform: &platform,
                    ..f.input(&next_key)
                },
                freeze,
            )
            .await?;
        assert_ne!(first.id, next.id);
        let expected = format!(
            "ayni-{architecture}-{}",
            hash("fingerprint").trim_start_matches("sha256:")
        );
        for operation in [first, next] {
            assert_eq!(operation.frozen.discovery_tag, expected);
            assert_eq!(operation.frozen.publication_tag, expected);
        }
    }
    Ok(())
}
