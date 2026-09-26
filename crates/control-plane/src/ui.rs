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
    }
}
