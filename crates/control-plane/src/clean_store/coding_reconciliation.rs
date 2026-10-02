impl CleanStore {
    pub(crate) fn coding_candidate_exists(&self, attempt_id: &str) -> Result<bool, CleanStoreError> {
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM candidates WHERE development_attempt_id=?1)", [attempt_id], |row| row.get(0)).map_err(CleanStoreError::Database)
    }
    pub(crate) fn coding_attempts(&self, profile_id: &str) -> Result<Vec<v1::DevelopmentAttempt>, CleanStoreError> {
        let mut statement = self.connection.prepare("SELECT a.payload FROM development_attempts a JOIN development_tasks t ON t.id=a.development_task_id WHERE t.runtime_profile_id=?1 AND NOT EXISTS (SELECT 1 FROM reconciliation_cursors c WHERE c.name='coding-done:'||a.id) ORDER BY a.id LIMIT 128").map_err(CleanStoreError::Database)?;
        let mut rows = statement.query([profile_id]).map_err(CleanStoreError::Database)?;
        collect_bounded_payloads(&mut rows)?.into_iter().map(|bytes| decode(&bytes)).collect()
    }

    pub(crate) fn coding_cleanup_complete(&mut self, attempt_id: &str) -> Result<(), CleanStoreError> {
        self.connection.execute("INSERT OR IGNORE INTO reconciliation_cursors(name,value,resource_version,updated_at) VALUES (?1,'{}',1,?2)", params![format!("coding-done:{attempt_id}"),unix_seconds()?]).map_err(CleanStoreError::Database)?;
        Ok(())
    }

    pub(crate) fn coding_details(&mut self, attempt_id: &str, requested_input: Option<String>, usage: v1::AttemptModelUsage) -> Result<(), CleanStoreError> {
        if requested_input.as_ref().is_some_and(|value| value.len() > 1024) || !valid_model_usage(Some(&usage)) { return Err(CleanStoreError::BoundExceeded); }
        let mut attempt = self.development_attempt(attempt_id)?;
        let task = self.development_task(&attempt.development_task_id)?;
        if attempt.requested_input == requested_input && attempt.model_usage.as_ref() == Some(&usage) { return Ok(()); }
        let previous = attempt.resource_version;
        attempt.requested_input = requested_input;
        attempt.model_usage = Some(usage);
        attempt.resource_version += 1;
        attempt.updated_at = Some(timestamp(unix_seconds()?));
        let transaction = self.connection.transaction().map_err(CleanStoreError::Database)?;
        persist_attempt_update(&transaction, &attempt, previous, &task)?;
        transaction.commit().map_err(CleanStoreError::Database)
    }
}
