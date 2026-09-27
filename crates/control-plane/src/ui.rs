use axum::response::Html;

pub async fn index() -> Html<&'static str> {
    Html(include_str!("../ui/index.html"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn task_cards_render_lifecycle_duration_and_partial_history() {
        let html = include_str!("../ui/index.html");
        assert!(html.contains("Lifecycle timeline"));
        assert!(html.contains("lifecycle_complete"));
        assert!(html.contains("partial history from before lifecycle instrumentation"));
        assert!(html.contains("reason_code"));
        assert!(html.contains("Date.now()"));
        assert!(html.contains("Provider usage"));
        assert!(html.contains("usage unavailable"));
        assert!(html.contains("Costs are estimates, not provider billing"));
        assert!(html.contains("cached_input_tokens"));
        assert!(html.contains("pricing_version"));
        assert!(html.contains("resolved_model"));
        assert!(html.contains("input_rate_micros_per_million"));
        assert!(html.contains("OpenAI model"));
        assert!(html.contains("/v1/provider-models"));
        assert!(html.contains("Select a model"));
        assert!(html.contains("t.agent_profile.model"));
        assert!(!html.contains("select name=\"profile_id\""));
    }
}
