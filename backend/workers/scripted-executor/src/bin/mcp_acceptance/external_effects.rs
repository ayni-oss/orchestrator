//! Exercise ambiguous MCP effects through the real deployed retained worker ledger.
use super::{DriverResult, id, prepare_fixture, read_stats, require_result, send_request};
use serde_json::{Value, json};

pub async fn profiles(
    client: &reqwest::Client,
    worker: &str,
    fixture: &str,
    provider: &str,
    replay: bool,
) -> DriverResult<Vec<Value>> {
    let mut results = Vec::new();
    for (mode, seed, counter) in [
        ("fixture:mcp-lost", 60, "mcpLostEffects"),
        ("fixture:mcp-cancel", 80, "mcpCancelledEffects"),
    ] {
        let before = read_stats(client, fixture).await?;
        let (prepared, command_digest) =
            prepare_fixture(client, worker, provider, mode, seed).await?;
        let result = if replay {
            require_result(&prepared)?;
            prepared["result"].clone()
        } else if seed == 80 {
            cancel_after_effect(client, worker, fixture, &prepared, &command_digest).await?
        } else {
            send_request(client, worker, &prepared, seed)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?
        };
        validate_result(&result, seed)?;
        let (retained, _) = prepare_fixture(client, worker, provider, mode, seed).await?;
        if retained["result"] != result {
            return Err("external-effect result changed on exact command replay".into());
        }
        // Exercise send replay as well as preparation replay; neither may resend.
        let sent: Value = send_request(client, worker, &retained, seed)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if sent != result {
            return Err("external-effect send replay changed retained result".into());
        }
        let after = read_stats(client, fixture).await?;
        if after[counter] != 1 || (replay && before != after) {
            return Err("external-effect replay contacted provider or repeated mutation".into());
        }
        results.push(json!({"command_id":id(seed),"command_digest":command_digest,"mode":mode,"result":result,"effect_counter":counter,"external_effects":after[counter]}));
    }
    Ok(results)
}

fn validate_result(result: &Value, seed: u8) -> DriverResult<()> {
    let expected = if seed == 60 {
        "provider_tool_uncertain"
    } else {
        "cancelled"
    };
    let tools = result["provider_call"]["toolInvocations"]
        .as_array()
        .ok_or("external-effect tool evidence missing")?;
    if result["failure_reason"] != expected
        || result["response_content"] != ""
        || tools.len() != 1
        || tools[0]["state"] != "uncertain"
    {
        return Err("ambiguous external effect lost its uncertainty or exposed output".into());
    }
    Ok(())
}

async fn cancel_after_effect(
    client: &reqwest::Client,
    worker: &str,
    fixture: &str,
    prepared: &Value,
    command_digest: &str,
) -> DriverResult<Value> {
    let request = send_request(client, worker, prepared, 80);
    let sending = tokio::spawn(async move {
        request
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if read_stats(client, fixture).await?["mcpCancelledEffects"] == 1 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err("in-flight MCP effect was not observed".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    client
        .post(format!("{worker}/v1/cancel"))
        .bearer_auth(super::TOKEN)
        .json(&json!({"command_id":id(80),"command_digest":command_digest}))
        .send()
        .await?
        .error_for_status()?;
    Ok(sending.await??)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_effect_requires_retained_uncertainty_and_no_output() {
        let mut result = json!({"failure_reason":"provider_tool_uncertain","response_content":"","provider_call":{"toolInvocations":[{"state":"uncertain"}]}});
        assert!(validate_result(&result, 60).is_ok());
        result["provider_call"]["toolInvocations"][0]["state"] = json!("completed");
        assert!(validate_result(&result, 60).is_err());
        result["provider_call"]["toolInvocations"][0]["state"] = json!("uncertain");
        result["failure_reason"] = json!("cancelled");
        assert!(validate_result(&result, 80).is_ok());
        result["response_content"] = json!("late output");
        assert!(validate_result(&result, 80).is_err());
    }
}
