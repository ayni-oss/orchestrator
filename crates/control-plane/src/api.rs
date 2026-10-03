use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    component: &'static str,
    status: &'static str,
    durable_orchestration: bool,
    runtime_profiles: &'static str,
    provider_models: &'static str,
    clean_v1_contract: bool,
    clean_schema_version: u8,
}

impl HealthResponse {
    pub const fn live() -> Self {
        Self {
            component: "control-plane",
            status: "live",
            durable_orchestration: true,
            runtime_profiles: "durable",
            provider_models: "unconfigured",
            clean_v1_contract: true,
            clean_schema_version: 2,
        }
    }

    pub const fn ready(model_catalog_configured: bool, model_catalog_available: bool) -> Self {
        Self {
            component: "control-plane",
            status: "ready",
            durable_orchestration: true,
            runtime_profiles: "durable",
            provider_models: if model_catalog_available {
                "available"
            } else if model_catalog_configured {
                "unavailable"
            } else {
                "unconfigured"
            },
            clean_v1_contract: true,
            clean_schema_version: 2,
        }
    }
}
