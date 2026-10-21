//! Process-local operational counters and fixed-bucket timing observations.
//!
//! Call sites use static names only: resource IDs, credentials and content must never
//! become metric names or labels. Durable pressure gauges come from `PostgreSQL`.
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    sync::{Mutex, OnceLock},
    time::Duration,
};

#[derive(Default)]
struct Measurements {
    gauges: BTreeMap<(&'static str, &'static str), u64>,
    counters: BTreeMap<&'static str, u64>,
    timings: BTreeMap<&'static str, Timing>,
}
#[derive(Default)]
struct Timing {
    count: u64,
    seconds: f64,
    buckets: [u64; 9],
}
const BOUNDS: [f64; 9] = [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 2.0];
static MEASUREMENTS: OnceLock<Mutex<Measurements>> = OnceLock::new();

/// Increment one safe, statically named process counter.
pub fn increment(name: &'static str) {
    if let Ok(mut state) = MEASUREMENTS.get_or_init(Mutex::default).lock() {
        let value = state.counters.entry(name).or_default();
        *value = value.saturating_add(1);
    }
}

/// Set one statically named gauge with a fixed transport-lane label.
pub fn gauge(name: &'static str, lane: &'static str, value: u64) {
    if let Ok(mut state) = MEASUREMENTS.get_or_init(Mutex::default).lock() {
        state.gauges.insert((name, lane), value);
    }
}

/// Record an owned or dependency duration in seconds with cumulative buckets.
pub fn observe(name: &'static str, duration: Duration) {
    if let Ok(mut state) = MEASUREMENTS.get_or_init(Mutex::default).lock() {
        let timing = state.timings.entry(name).or_default();
        timing.count = timing.count.saturating_add(1);
        timing.seconds += duration.as_secs_f64();
        for (bucket, bound) in timing.buckets.iter_mut().zip(BOUNDS) {
            if duration.as_secs_f64() <= bound {
                *bucket = bucket.saturating_add(1);
            }
        }
    }
}

/// Render the current safe process counters in Prometheus text exposition format.
#[must_use]
pub fn render() -> String {
    let mut output = String::new();
    if let Ok(state) = MEASUREMENTS.get_or_init(Mutex::default).lock() {
        for ((name, lane), value) in &state.gauges {
            let _ = writeln!(output, "{name}{{lane=\"{lane}\"}} {value}");
        }
        for (name, value) in &state.counters {
            let _ = writeln!(output, "{name} {value}");
        }
        for (name, timing) in &state.timings {
            for (bound, count) in BOUNDS.into_iter().zip(timing.buckets) {
                let _ = writeln!(output, "{name}_bucket{{le=\"{bound}\"}} {count}");
            }
            let _ = writeln!(output, "{name}_bucket{{le=\"+Inf\"}} {}", timing.count);
            let _ = writeln!(output, "{name}_count {}", timing.count);
            let _ = writeln!(output, "{name}_sum {}", timing.seconds);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn histogram_buckets_are_cumulative() {
        observe("workflow_test_duration_seconds", Duration::from_millis(20));
        let result = render();
        assert!(result.contains("workflow_test_duration_seconds_bucket{le=\"0.01\"} 0"));
        assert!(result.contains("workflow_test_duration_seconds_bucket{le=\"0.025\"} 1"));
        assert!(result.contains("workflow_test_duration_seconds_bucket{le=\"+Inf\"} 1"));
    }
}
