use super::{Fixture, StoreError, TestResult};

#[tokio::test]
#[ignore = "requires disposable migrated PostgreSQL with runtime-role URL"]
async fn history_omits_current_before_paging_in_cursor_order() -> TestResult {
    let fixture = Fixture::new().await?;
    let mut ids = Vec::new();
    for _ in 0..12 {
        ids.push(fixture.start().await?.id);
    }
    ids.sort_unstable_by(|left, right| right.cmp(left));
    let excluded = ids.remove(2);
    let (first, cursor) = fixture
        .store
        .list_runtime_preparations(&fixture.repository, "", &excluded, 10)
        .await?;
    let (second, end) = fixture
        .store
        .list_runtime_preparations(&fixture.repository, &cursor, &excluded, 10)
        .await?;
    assert_eq!(first.len(), 10);
    assert_eq!(cursor, ids[9]);
    assert_eq!(second.len(), 1);
    assert!(end.is_empty());
    assert_eq!(
        first
            .into_iter()
            .chain(second)
            .map(|value| value.id)
            .collect::<Vec<_>>(),
        ids
    );
    assert!(matches!(
        fixture
            .store
            .list_runtime_preparations(&fixture.repository, "", "invalid", 10)
            .await,
        Err(StoreError::InvalidInput("exclude_id"))
    ));
    Ok(())
}
