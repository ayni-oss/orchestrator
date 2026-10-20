use super::*;

impl Ledger {
    /// Record an application decision before acknowledging its consumer delivery.
    /// Exact report bytes are removed only after this transaction commits.
    ///
    /// # Errors
    /// Returns missing reports, immutable conflicts, capacity, or SQL errors.
    #[cfg(test)]
    pub fn record_decision(
        &mut self,
        decision: &Decision<'_>,
    ) -> Result<StoreOutcome, LedgerError> {
        self.record_decision_with_consumer_sequence(decision, None)
    }

    /// Record a decision and the durable consumer delivery sequence needed to
    /// reconcile an acknowledged delivery after a process crash.
    ///
    /// # Errors
    /// Returns invalid sequences, missing reports, immutable conflicts, capacity,
    /// or SQL errors.
    pub fn record_decision_delivery(
        &mut self,
        decision: &Decision<'_>,
        consumer_sequence: u64,
    ) -> Result<StoreOutcome, LedgerError> {
        if consumer_sequence == 0 {
            return Err(LedgerError::Invalid);
        }
        self.record_decision_with_consumer_sequence(decision, Some(consumer_sequence))
    }

    fn record_decision_with_consumer_sequence(
        &mut self,
        decision: &Decision<'_>,
        consumer_sequence: Option<u64>,
    ) -> Result<StoreOutcome, LedgerError> {
        let instance_id = retained_instance_id(&self.connection)?;
        let (payload, accepted) = decode_targeted_decision(decision.exact_bytes, &instance_id)?;
        let decision_digest = exact_digest(decision.exact_bytes);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let retained = retained_decision_report(&transaction, &payload)?;
        let result = if accepted { "accepted" } else { "rejected" };
        if prior_decision_is_duplicate(&retained, result, &decision_digest, decision.exact_bytes)? {
            retain_decision_consumer_sequence(
                &transaction,
                &payload.report_source,
                &payload.report_id,
                consumer_sequence,
            )?;
            transaction.commit()?;
            return Ok(StoreOutcome::Duplicate);
        }
        let signing_public_key = validate_decision_binding(
            &transaction,
            &payload,
            &retained.report_digest,
            retained.report_bytes.as_deref(),
            &retained.command_source,
            &retained.command_event_id,
            retained.sequence,
            &instance_id,
        )?;
        reserve(
            &transaction,
            self.limits,
            0,
            decision.exact_bytes.len(),
            retained.report_bytes.as_deref().map_or(0, <[u8]>::len),
        )?;
        transaction.execute(
            "UPDATE reports SET decision = ?3, decision_digest = ?4, decision_bytes = ?5, \
             decision_consumer_sequence = ?6, event_bytes = NULL \
             WHERE source = ?1 AND event_id = ?2",
            params![
                payload.report_source,
                payload.report_id,
                result,
                decision_digest,
                decision.exact_bytes,
                consumer_sequence
                    .map(i64::try_from)
                    .transpose()
                    .map_err(|_| LedgerError::Invalid)?,
            ],
        )?;
        let (command_state, cleanup_command) = command_transition(retained.sequence, accepted)?;
        transaction.execute(
            "UPDATE commands SET state = ?3, signing_public_key = COALESCE(?5, signing_public_key), \
             event_bytes = CASE WHEN ?4 THEN NULL ELSE event_bytes END, \
             terminal_reserved_bytes = CASE WHEN ?4 THEN 0 ELSE terminal_reserved_bytes END \
             WHERE source = ?1 AND event_id = ?2",
            params![
                retained.command_source,
                retained.command_event_id,
                command_state,
                cleanup_command,
                signing_public_key,
            ],
        )?;
        transaction.commit()?;
        Ok(StoreOutcome::Stored)
    }

    /// Reconcile a decision ACK from the retained consumer sequence and the
    /// server-confirmed contiguous ACK floor of the retained instance's named
    /// consumer. This closes the crash window after the broker accepted an ACK but
    /// before the local ledger recorded it.
    ///
    /// # Errors
    /// Returns pending or missing acknowledgements, invalid sequences, or SQL errors.
    pub fn reconcile_decision_ack_floor(
        &self,
        report_source: &str,
        report_event_id: &str,
        consumer_name: &str,
        acknowledged_consumer_sequence: u64,
    ) -> Result<(), LedgerError> {
        let instance_id = retained_instance_id(&self.connection)?;
        if consumer_name != format!("executor-{instance_id}") {
            return Err(LedgerError::Invalid);
        }
        let retained: Option<i64> = self
            .connection
            .query_row(
                "SELECT decision_consumer_sequence FROM reports \
                 WHERE source = ?1 AND event_id = ?2 AND decision IS NOT NULL",
                params![report_source, report_event_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let retained = retained.ok_or(LedgerError::NotFound)?;
        let retained = u64::try_from(retained).map_err(|_| LedgerError::Invalid)?;
        if acknowledged_consumer_sequence < retained {
            return Err(LedgerError::DecisionAckPending);
        }
        self.mark_decision_acknowledged_at(report_source, report_event_id, unix_seconds()?)
    }

    /// Test-only direct acknowledgement after a simulated broker confirmation.
    ///
    /// # Errors
    /// Returns missing decisions, invalid clocks, or SQL failures.
    #[cfg(test)]
    pub fn acknowledge_decision(
        &self,
        report_source: &str,
        report_event_id: &str,
    ) -> Result<(), LedgerError> {
        self.mark_decision_acknowledged_at(report_source, report_event_id, unix_seconds()?)
    }

    /// Test-only direct acknowledgement at a deterministic fixture time.
    ///
    /// # Errors
    /// Returns missing decisions, invalid times, or SQL failures.
    #[cfg(test)]
    pub fn acknowledge_decision_at(
        &self,
        report_source: &str,
        report_event_id: &str,
        resolved_at_seconds: u64,
    ) -> Result<(), LedgerError> {
        self.mark_decision_acknowledged_at(report_source, report_event_id, resolved_at_seconds)
    }

    fn mark_decision_acknowledged_at(
        &self,
        report_source: &str,
        report_event_id: &str,
        resolved_at_seconds: u64,
    ) -> Result<(), LedgerError> {
        let changed = self.connection.execute(
            "UPDATE reports SET decision_acked = 1, decision_bytes = NULL, \
             resolved_at_seconds = ?3 \
             WHERE source = ?1 AND event_id = ?2 AND decision IS NOT NULL \
             AND decision_acked = 0",
            params![
                report_source,
                report_event_id,
                i64::try_from(resolved_at_seconds).map_err(|_| LedgerError::Invalid)?,
            ],
        )?;
        if changed == 0 {
            let already_acked: Option<bool> = self
                .connection
                .query_row(
                    "SELECT decision_acked FROM reports \
                     WHERE source = ?1 AND event_id = ?2 AND decision IS NOT NULL",
                    params![report_source, report_event_id],
                    |row| row.get(0),
                )
                .optional()?;
            if already_acked != Some(true) {
                return Err(LedgerError::NotFound);
            }
        }
        Ok(())
    }
}
