impl Store {
    pub fn put_repository_binding(&self, binding: &RepositoryBinding) -> Result<(), StoreError> {
        binding.validate()?;
        self.connection.execute(
            "INSERT INTO repository_bindings(id, owner, repository, installation_id, base_branch, lock_sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET owner=excluded.owner, repository=excluded.repository,
             installation_id=excluded.installation_id, base_branch=excluded.base_branch,
             lock_sha256=excluded.lock_sha256",
            params![binding.id, binding.owner, binding.repository,
                binding.installation_id, binding.base_branch, binding.lock_sha256],
        ).map_err(StoreError::Database)?;
        Ok(())
    }

    /// Saves a repository enrollment with durable request idempotency.
    pub fn save_repository(
        &mut self,
        key: &IdempotencyKey,
        binding: &RepositoryBinding,
    ) -> Result<crate::preparation::RepositoryView, StoreError> {
        binding.validate()?;
        let snapshot = serde_json::to_string(binding)
            .map_err(|_| StoreError::InvalidInput("could not serialize repository"))?;
        if let Some(existing) = self
            .connection
            .query_row(
                "SELECT binding_snapshot FROM repository_actions WHERE action_key=?1",
                [key.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::Database)?
        {
            let saved: RepositoryBinding = serde_json::from_str(&existing)
                .map_err(|_| StoreError::CorruptState("invalid persisted repository action"))?;
            if saved != *binding {
                return Err(StoreError::ActionKeyReused(key.as_str().to_owned()));
            }
            let mut view = self.repository_view(&saved.id)?;
            view.binding = saved;
            return Ok(view);
        }
        let now = super::orchestration::unix_seconds()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO repository_bindings(id, owner, repository, installation_id, base_branch, lock_sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET owner=excluded.owner, repository=excluded.repository,
                 installation_id=excluded.installation_id, base_branch=excluded.base_branch,
                 lock_sha256=excluded.lock_sha256",
                params![binding.id, binding.owner, binding.repository, binding.installation_id,
                    binding.base_branch, binding.lock_sha256],
            )
            .map_err(StoreError::Database)?;
        transaction
            .execute(
                "INSERT INTO repository_actions(action_key, repository_id, binding_snapshot, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![key.as_str(), binding.id, snapshot, now],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        self.repository_view(&binding.id)
    }

    pub fn put_agent_profile(&self, profile: &AgentProfile) -> Result<(), StoreError> {
        profile.validate()?;
        self.connection
            .execute(
                "INSERT INTO agent_profiles(id, provider, model, secret_ref)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET provider=excluded.provider, model=excluded.model,
             secret_ref=excluded.secret_ref",
                params![
                    profile.id,
                    profile.provider,
                    profile.model,
                    profile.secret_ref
                ],
            )
            .map_err(StoreError::Database)?;
        Ok(())
    }

    pub(crate) fn development_action(
        &self,
        key: &IdempotencyKey,
    ) -> Result<Option<(String, TaskId)>, StoreError> {
        self.connection
            .query_row(
                "SELECT action_kind, task_id FROM development_actions WHERE action_key=?1",
                [key.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(StoreError::Database)?
            .map(|(kind, id)| {
                TaskId::parse(&id)
                    .map(|id| (kind, id))
                    .map_err(StoreError::InvalidLifecycleData)
            })
            .transpose()
    }

}
