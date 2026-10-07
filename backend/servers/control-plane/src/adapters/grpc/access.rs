use crate::process::metrics::record_grpc_admission;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tonic::{Request, Status, metadata::KeyAndValueRef};
use tracing::{info, warn};

pub const MAX_METADATA_BYTES: usize = 8 * 1024;
pub const MAX_HTTP2_HEADER_LIST_BYTES: u32 = 16 * 1024;
const MAX_METADATA_ENTRIES: usize = 32;
pub const MAX_CONCURRENT_STREAMS_PER_CONNECTION: u32 = 32;
const MAX_REQUESTS_PER_MINUTE: u32 = 120;
const MAX_UNARY_DEADLINE: Duration = Duration::from_secs(30);
const MAX_WATCH_DEADLINE: Duration = Duration::from_secs(5 * 60);
const RATE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct CustomClientAccess {
    certificate_pem: Arc<[u8]>,
    private_key_pem: Arc<[u8]>,
    authorizer: Authorizer,
}

impl CustomClientAccess {
    /// Loads the PEM identity and bounded client digest configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be read or its contents violate the
    /// supported identity configuration.
    pub fn load(
        certificate_path: &Path,
        private_key_path: &Path,
        clients_path: &Path,
    ) -> Result<Self, AccessConfigurationError> {
        let certificate_pem = fs::read(certificate_path)
            .map_err(|source| AccessConfigurationError::ReadCertificate { source })?;
        let private_key_pem = fs::read(private_key_path)
            .map_err(|source| AccessConfigurationError::ReadPrivateKey { source })?;
        let clients = fs::read(clients_path)
            .map_err(|source| AccessConfigurationError::ReadClients { source })?;
        if certificate_pem.is_empty() || private_key_pem.is_empty() {
            return Err(AccessConfigurationError::EmptyTlsIdentity);
        }
        Ok(Self {
            certificate_pem: certificate_pem.into(),
            private_key_pem: private_key_pem.into(),
            authorizer: Authorizer::from_json(&clients)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_parts_for_test(
        certificate_pem: Vec<u8>,
        private_key_pem: Vec<u8>,
        clients_json: &[u8],
    ) -> Result<Self, AccessConfigurationError> {
        Ok(Self {
            certificate_pem: certificate_pem.into(),
            private_key_pem: private_key_pem.into(),
            authorizer: Authorizer::from_json(clients_json)?,
        })
    }

    pub(crate) fn certificate_pem(&self) -> &[u8] {
        &self.certificate_pem
    }
    pub(crate) fn private_key_pem(&self) -> &[u8] {
        &self.private_key_pem
    }
    pub(crate) fn guard(&self) -> CustomClientGuard {
        CustomClientGuard {
            authorizer: self.authorizer.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum AccessConfigurationError {
    #[error("could not read the custom-client TLS certificate")]
    ReadCertificate {
        #[source]
        source: io::Error,
    },
    #[error("could not read the custom-client TLS private key")]
    ReadPrivateKey {
        #[source]
        source: io::Error,
    },
    #[error("could not read the custom-client identity configuration")]
    ReadClients {
        #[source]
        source: io::Error,
    },
    #[error("the custom-client TLS identity is empty")]
    EmptyTlsIdentity,
    #[error("the custom-client identity configuration is invalid: {0}")]
    InvalidClients(&'static str),
    #[error("the custom-client identity configuration is not valid JSON")]
    DecodeClients(#[source] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct ClientIdentity {
    id: Arc<str>,
    request_id: Option<Arc<str>>,
}
impl ClientIdentity {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
}

#[derive(Clone)]
pub struct CustomClientGuard {
    authorizer: Authorizer,
}

impl CustomClientGuard {
    pub(crate) fn authorize<T>(
        &self,
        request: &Request<T>,
        rpc: &'static str,
    ) -> Result<ClientIdentity, Status> {
        let operation = operation(rpc).ok_or_else(|| {
            Status::permission_denied("the RPC is not available to custom clients")
        })?;
        let request_id = bounded_request_id(request);
        match self.authorizer.authorize(request, operation) {
            Ok(mut identity) => {
                record_grpc_admission(operation.rpc, "accepted", tonic::Code::Ok);
                info!(
                    client_id = identity.id(),
                    rpc = operation.rpc,
                    request_id,
                    decision = "accepted",
                    "custom-client gRPC request authorized"
                );
                identity.request_id = Some(Arc::from(request_id));
                Ok(identity)
            }
            Err(failure) => {
                record_grpc_admission(operation.rpc, "rejected", failure.status.code());
                warn!(client_id = failure.client_id.as_ref(), rpc = operation.rpc, request_id,
                    decision = "rejected", reason = failure.reason, status = ?failure.status.code(),
                    "custom-client gRPC request rejected");
                let mut status = failure.status;
                if let Ok(value) = request_id.parse() {
                    status.metadata_mut().insert("x-request-id", value);
                }
                Err(status)
            }
        }
    }
}

#[derive(Clone)]
struct Authorizer {
    clients: Arc<Vec<Client>>,
    rate: Arc<Mutex<HashMap<Arc<str>, RateState>>>,
}

impl Authorizer {
    fn from_json(value: &[u8]) -> Result<Self, AccessConfigurationError> {
        let file: ClientFile =
            serde_json::from_slice(value).map_err(AccessConfigurationError::DecodeClients)?;
        if file.version != 1 || file.clients.is_empty() || file.clients.len() > 64 {
            return Err(AccessConfigurationError::InvalidClients(
                "version must be 1 and clients must contain 1 through 64 identities",
            ));
        }
        let mut ids = HashSet::new();
        let mut digests = HashSet::new();
        let mut clients = Vec::with_capacity(file.clients.len());
        for configured in file.clients {
            if !valid_client_id(&configured.id) || !ids.insert(configured.id.clone()) {
                return Err(AccessConfigurationError::InvalidClients(
                    "client IDs must be unique 1-64 byte safe ASCII values",
                ));
            }
            let digest = decode_digest(&configured.token_sha256).ok_or(
                AccessConfigurationError::InvalidClients(
                    "token_sha256 must contain 64 lowercase hexadecimal characters",
                ),
            )?;
            if !digests.insert(digest) || configured.expires_at_unix == 0 {
                return Err(AccessConfigurationError::InvalidClients(
                    "token digests must be unique and expiry must be positive",
                ));
            }
            if configured.capabilities.is_empty() || configured.capabilities.len() > 11 {
                return Err(AccessConfigurationError::InvalidClients(
                    "capabilities must contain 1 through 11 values",
                ));
            }
            let mut capabilities = HashSet::new();
            for value in configured.capabilities {
                let capability = Capability::parse(&value).ok_or(
                    AccessConfigurationError::InvalidClients("a capability is not recognized"),
                )?;
                if !capabilities.insert(capability) {
                    return Err(AccessConfigurationError::InvalidClients(
                        "capabilities must be unique",
                    ));
                }
            }
            clients.push(Client {
                id: Arc::from(configured.id),
                token_sha256: digest,
                expires_at_unix: configured.expires_at_unix,
                capabilities,
            });
        }
        Ok(Self {
            clients: Arc::new(clients),
            rate: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn authorize<T>(
        &self,
        request: &Request<T>,
        operation: Operation,
    ) -> Result<ClientIdentity, AuthorizationFailure> {
        if metadata_size(request) > MAX_METADATA_BYTES
            || request.metadata().len() > MAX_METADATA_ENTRIES
        {
            return Err(AuthorizationFailure::unknown(
                "metadata_limit",
                Status::resource_exhausted(
                    "gRPC metadata exceeds the supported 8 KiB or 32-entry limit",
                ),
            ));
        }
        let values = request.metadata().get_all("authorization");
        let mut values = values.iter();
        let authorization = values.next().ok_or_else(|| {
            AuthorizationFailure::unknown(
                "credential_missing",
                Status::unauthenticated("authorization metadata is required"),
            )
        })?;
        if values.next().is_some() {
            return Err(AuthorizationFailure::unknown(
                "credential_ambiguous",
                Status::unauthenticated("exactly one authorization metadata value is required"),
            ));
        }
        let authorization = authorization.to_str().map_err(|_| {
            AuthorizationFailure::unknown(
                "credential_invalid",
                Status::unauthenticated("authorization metadata is invalid"),
            )
        })?;
        let token = authorization.strip_prefix("Bearer ").ok_or_else(|| {
            AuthorizationFailure::unknown(
                "credential_scheme",
                Status::unauthenticated("authorization must use the Bearer scheme"),
            )
        })?;
        if token.len() < 32
            || token.len() > 512
            || !token.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(AuthorizationFailure::unknown(
                "credential_invalid",
                Status::unauthenticated("the bearer credential is invalid"),
            ));
        }
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut matched = None;
        for client in self.clients.iter() {
            if bool::from(client.token_sha256.ct_eq(&digest)) {
                matched = Some(client);
            }
        }
        let client = matched.ok_or_else(|| {
            AuthorizationFailure::unknown(
                "credential_invalid",
                Status::unauthenticated("the bearer credential is invalid"),
            )
        })?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_secs());
        if client.expires_at_unix <= now {
            return Err(AuthorizationFailure::client(
                client,
                "credential_expired",
                Status::unauthenticated("the bearer credential has expired"),
            ));
        }
        if let Some(capability) = operation
            .required
            .iter()
            .find(|capability| !client.capabilities.contains(capability))
        {
            return Err(AuthorizationFailure::client(
                client,
                "capability_missing",
                Status::permission_denied(format!(
                    "the client lacks the required {} capability",
                    capability.as_str()
                )),
            ));
        }
        validate_deadline(request, operation)
            .map_err(|status| AuthorizationFailure::client(client, "deadline_invalid", status))?;
        self.check_rate(client)?;
        Ok(ClientIdentity {
            id: Arc::clone(&client.id),
            request_id: None,
        })
    }

    fn check_rate(&self, client: &Client) -> Result<(), AuthorizationFailure> {
        let now = Instant::now();
        let mut rates = self.rate.lock().map_err(|_| {
            AuthorizationFailure::client(
                client,
                "rate_state_unavailable",
                Status::unavailable("request admission is temporarily unavailable"),
            )
        })?;
        let state = rates.entry(Arc::clone(&client.id)).or_insert(RateState {
            started: now,
            count: 0,
        });
        if now.duration_since(state.started) >= RATE_WINDOW {
            state.started = now;
            state.count = 0;
        }
        if state.count >= MAX_REQUESTS_PER_MINUTE {
            return Err(AuthorizationFailure::client(
                client,
                "rate_limit",
                Status::resource_exhausted(
                    "the per-client request rate limit was exceeded; retry after the current window",
                ),
            ));
        }
        state.count += 1;
        drop(rates);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Capability {
    RepositoriesRead,
    RepositoriesWrite,
    ProfilesRead,
    ProfilesWrite,
    ProviderModelsRead,
    TasksRead,
    TasksCreate,
    TasksCancel,
    TasksRetry,
}

impl Capability {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "repositories.read" => Self::RepositoriesRead,
            "repositories.write" => Self::RepositoriesWrite,
            "profiles.read" => Self::ProfilesRead,
            "profiles.write" => Self::ProfilesWrite,
            "provider_models.read" => Self::ProviderModelsRead,
            "tasks.read" => Self::TasksRead,
            "tasks.create" => Self::TasksCreate,
            "tasks.cancel" => Self::TasksCancel,
            "tasks.retry" => Self::TasksRetry,
            _ => return None,
        })
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::RepositoriesRead => "repositories.read",
            Self::RepositoriesWrite => "repositories.write",
            Self::ProfilesRead => "profiles.read",
            Self::ProfilesWrite => "profiles.write",
            Self::ProviderModelsRead => "provider_models.read",
            Self::TasksRead => "tasks.read",
            Self::TasksCreate => "tasks.create",
            Self::TasksCancel => "tasks.cancel",
            Self::TasksRetry => "tasks.retry",
        }
    }
}

#[derive(Clone, Copy)]
struct Operation {
    rpc: &'static str,
    required: &'static [Capability],
    deadline: Duration,
}

const REPOSITORIES_READ: &[Capability] = &[Capability::RepositoriesRead];
const REPOSITORIES_WRITE: &[Capability] = &[Capability::RepositoriesWrite];
const PROFILES_READ: &[Capability] = &[Capability::ProfilesRead];
const PROFILES_WRITE: &[Capability] = &[Capability::ProfilesWrite];
const PROVIDER_MODELS_READ: &[Capability] = &[Capability::ProviderModelsRead];
const TASKS_READ: &[Capability] = &[Capability::TasksRead];
const TASKS_CREATE: &[Capability] = &[Capability::TasksCreate];
const TASKS_CANCEL: &[Capability] = &[Capability::TasksCancel];
const TASKS_RETRY: &[Capability] = &[Capability::TasksRetry];

fn operation(rpc: &'static str) -> Option<Operation> {
    let (required, deadline) = match rpc {
        "ListProjects"
        | "GetProject"
        | "ListRepositoryConnections"
        | "GetRepositoryConnection"
        | "ListRepositorySnapshots"
        | "GetRepositorySnapshot" => (REPOSITORIES_READ, MAX_UNARY_DEADLINE),
        "WatchRepositories" => (REPOSITORIES_READ, MAX_WATCH_DEADLINE),
        "CreateProject"
        | "UpdateProject"
        | "ArchiveProject"
        | "CreateRepositoryConnection"
        | "ConnectGithubRepository"
        | "SetRepositoryRegistry" => (REPOSITORIES_WRITE, MAX_UNARY_DEADLINE),
        "ListRuntimeProfiles"
        | "GetRuntimeProfile"
        | "ListRuntimeProfileRevisions"
        | "GetRuntimeProfileRevision"
        | "ListRuntimeArtifacts"
        | "GetRuntimeArtifact"
        | "ListAutomationInstallations"
        | "GetAutomationInstallation" => (PROFILES_READ, MAX_UNARY_DEADLINE),
        "WatchRuntimeProfiles" => (PROFILES_READ, MAX_WATCH_DEADLINE),
        "InspectAyniRepository"
        | "CreateRuntimeProfile"
        | "InstallRepositoryAutomation"
        | "ReconcileRuntimeProfile" => (PROFILES_WRITE, MAX_UNARY_DEADLINE),
        "ListProviderModels" => (PROVIDER_MODELS_READ, MAX_UNARY_DEADLINE),
        "ListDevelopmentTasks"
        | "GetDevelopmentTask"
        | "ListDevelopmentAttempts"
        | "GetDevelopmentAttempt" => (TASKS_READ, MAX_UNARY_DEADLINE),
        "WatchDevelopmentTask" => (TASKS_READ, MAX_WATCH_DEADLINE),
        "CreateDevelopmentTask" => (TASKS_CREATE, MAX_UNARY_DEADLINE),
        "CancelDevelopmentTask" => (TASKS_CANCEL, MAX_UNARY_DEADLINE),
        "RetryDevelopmentTask" | "RetryTaskPublication" => (TASKS_RETRY, MAX_UNARY_DEADLINE),
        _ => return None,
    };
    Some(Operation {
        rpc,
        required,
        deadline,
    })
}

fn validate_deadline<T>(request: &Request<T>, operation: Operation) -> Result<(), Status> {
    let value = request
        .metadata()
        .get("grpc-timeout")
        .ok_or_else(|| Status::invalid_argument("custom-client RPCs require a bounded deadline"))?
        .to_str()
        .map_err(|_| Status::invalid_argument("grpc-timeout metadata is invalid"))?;
    let duration = parse_timeout(value)
        .ok_or_else(|| Status::invalid_argument("grpc-timeout metadata is invalid"))?;
    if duration.is_zero() {
        return Err(Status::deadline_exceeded(
            "the request deadline has elapsed",
        ));
    }
    if duration > operation.deadline {
        return Err(Status::invalid_argument(
            if operation.rpc.starts_with("Watch") {
                "watch deadlines must not exceed 5 minutes"
            } else {
                "unary deadlines must not exceed 30 seconds"
            },
        ));
    }
    Ok(())
}

fn parse_timeout(value: &str) -> Option<Duration> {
    if value.len() < 2 || value.len() > 9 {
        return None;
    }
    let (digits, unit) = value.split_at(value.len() - 1);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let amount = digits.parse::<u64>().ok()?;
    match unit {
        "H" => Some(Duration::from_secs(amount.checked_mul(3_600)?)),
        "M" => Some(Duration::from_secs(amount.checked_mul(60)?)),
        "S" => Some(Duration::from_secs(amount)),
        "m" => Some(Duration::from_millis(amount)),
        "u" => Some(Duration::from_micros(amount)),
        "n" => Some(Duration::from_nanos(amount)),
        _ => None,
    }
}

fn metadata_size<T>(request: &Request<T>) -> usize {
    request.metadata().iter().fold(0usize, |size, entry| {
        let (key, value) = match entry {
            KeyAndValueRef::Ascii(key, value) => {
                (key.as_str().len(), value.as_encoded_bytes().len())
            }
            KeyAndValueRef::Binary(key, value) => {
                (key.as_str().len(), value.as_encoded_bytes().len())
            }
        };
        size.saturating_add(key).saturating_add(value)
    })
}

fn bounded_request_id<T>(request: &Request<T>) -> String {
    let mut values = request.metadata().get_all("x-request-id").iter();
    let value = values.next().and_then(|value| value.to_str().ok());
    if values.next().is_none()
        && let Some(value) = value.filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return value.to_owned();
    }
    uuid::Uuid::now_v7().to_string()
}

fn valid_client_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn decode_digest(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
    }
    Some(digest)
}

struct Client {
    id: Arc<str>,
    token_sha256: [u8; 32],
    expires_at_unix: u64,
    capabilities: HashSet<Capability>,
}
struct RateState {
    started: Instant,
    count: u32,
}
struct AuthorizationFailure {
    client_id: Arc<str>,
    reason: &'static str,
    status: Status,
}
impl AuthorizationFailure {
    fn unknown(reason: &'static str, status: Status) -> Self {
        Self {
            client_id: Arc::from("unknown"),
            reason,
            status,
        }
    }
    fn client(client: &Client, reason: &'static str, status: Status) -> Self {
        Self {
            client_id: Arc::clone(&client.id),
            reason,
            status,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientFile {
    version: u32,
    clients: Vec<ConfiguredClient>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfiguredClient {
    id: String,
    token_sha256: String,
    expires_at_unix: u64,
    capabilities: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;
    const TOKEN: &str = "unit-test-custom-client-token-value-123456789";

    fn clients(expires_at_unix: u64, capabilities: &[&str]) -> Vec<u8> {
        let digest = Sha256::digest(TOKEN.as_bytes());
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "clients": [{
                "id": "ci-client", "token_sha256": format!("{digest:x}"),
                "expires_at_unix": expires_at_unix, "capabilities": capabilities,
            }]
        }))
        .unwrap_or_default()
    }

    fn request(token: Option<&str>, timeout: &str) -> Request<()> {
        let mut request = Request::new(());
        if let Some(token) = token
            && let Ok(value) = format!("Bearer {token}").parse()
        {
            request.metadata_mut().insert("authorization", value);
        }
        if let Ok(value) = timeout.parse() {
            request.metadata_mut().insert("grpc-timeout", value);
        }
        request
    }

    fn guard(expires_at_unix: u64, capabilities: &[&str]) -> CustomClientGuard {
        CustomClientAccess::from_parts_for_test(
            b"certificate".to_vec(),
            b"private-key".to_vec(),
            &clients(expires_at_unix, capabilities),
        )
        .unwrap_or_else(|_| unreachable!())
        .guard()
    }

    #[test]
    fn authenticates_and_authorizes_operation_capabilities() {
        let access = guard(u64::MAX, &["tasks.create"]);
        let accepted = access.authorize(&request(Some(TOKEN), "30S"), "CreateDevelopmentTask");
        assert_eq!(
            accepted.ok().map(|identity| identity.id().to_owned()),
            Some("ci-client".to_owned())
        );
    }

    #[test]
    fn every_clean_v1_rpc_uses_its_documented_capability() {
        let cases = [
            ("ListRepositoryConnections", Capability::RepositoriesRead),
            ("GetRepositoryConnection", Capability::RepositoriesRead),
            ("ListRepositorySnapshots", Capability::RepositoriesRead),
            ("GetRepositorySnapshot", Capability::RepositoriesRead),
            ("WatchRepositories", Capability::RepositoriesRead),
            ("CreateRepositoryConnection", Capability::RepositoriesWrite),
            ("CreateProject", Capability::RepositoriesWrite),
            ("UpdateProject", Capability::RepositoriesWrite),
            ("ArchiveProject", Capability::RepositoriesWrite),
            ("ListProjects", Capability::RepositoriesRead),
            ("GetProject", Capability::RepositoriesRead),
            ("ListRuntimeProfiles", Capability::ProfilesRead),
            ("GetRuntimeProfile", Capability::ProfilesRead),
            ("ListRuntimeProfileRevisions", Capability::ProfilesRead),
            ("GetRuntimeProfileRevision", Capability::ProfilesRead),
            ("ListRuntimeArtifacts", Capability::ProfilesRead),
            ("GetRuntimeArtifact", Capability::ProfilesRead),
            ("ListAutomationInstallations", Capability::ProfilesRead),
            ("GetAutomationInstallation", Capability::ProfilesRead),
            ("WatchRuntimeProfiles", Capability::ProfilesRead),
            ("CreateRuntimeProfile", Capability::ProfilesWrite),
            ("InstallRepositoryAutomation", Capability::ProfilesWrite),
            ("ReconcileRuntimeProfile", Capability::ProfilesWrite),
            ("ListProviderModels", Capability::ProviderModelsRead),
            ("ListDevelopmentTasks", Capability::TasksRead),
            ("GetDevelopmentTask", Capability::TasksRead),
            ("ListDevelopmentAttempts", Capability::TasksRead),
            ("GetDevelopmentAttempt", Capability::TasksRead),
            ("WatchDevelopmentTask", Capability::TasksRead),
            ("CreateDevelopmentTask", Capability::TasksCreate),
            ("CancelDevelopmentTask", Capability::TasksCancel),
            ("RetryDevelopmentTask", Capability::TasksRetry),
        ];
        for (rpc, capability) in cases {
            let Some(operation) = operation(rpc) else {
                panic!("{rpc} has no custom-client authorization mapping");
            };
            assert_eq!(operation.required, [capability], "{rpc}");
            assert_eq!(
                operation.deadline,
                if rpc.starts_with("Watch") {
                    MAX_WATCH_DEADLINE
                } else {
                    MAX_UNARY_DEADLINE
                },
                "{rpc}"
            );
        }

        let all_capabilities = [
            "repositories.read",
            "repositories.write",
            "profiles.read",
            "profiles.write",
            "provider_models.read",
            "tasks.read",
            "tasks.create",
            "tasks.cancel",
            "tasks.retry",
        ];
        assert!(
            CustomClientAccess::from_parts_for_test(
                b"certificate".to_vec(),
                b"private-key".to_vec(),
                &clients(u64::MAX, &all_capabilities),
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_missing_invalid_forbidden_and_expired_credentials() {
        let access = guard(u64::MAX, &["tasks.read"]);
        for (token, expected) in [
            (None, Code::Unauthenticated),
            (Some("x"), Code::Unauthenticated),
            (Some(TOKEN), Code::PermissionDenied),
        ] {
            assert_eq!(
                access
                    .authorize(&request(token, "30S"), "CreateDevelopmentTask")
                    .err()
                    .map(|status| status.code()),
                Some(expected)
            );
        }
        let expired = guard(1, &["tasks.create"]);
        assert_eq!(
            expired
                .authorize(&request(Some(TOKEN), "30S"), "CreateDevelopmentTask")
                .err()
                .map(|status| status.code()),
            Some(Code::Unauthenticated)
        );
    }

    #[test]
    fn enforces_deadlines_rate_and_configuration_bounds() {
        let access = guard(u64::MAX, &["tasks.create"]);
        assert_eq!(
            access
                .authorize(&request(Some(TOKEN), "31S"), "CreateDevelopmentTask")
                .err()
                .map(|status| status.code()),
            Some(Code::InvalidArgument)
        );
        let mut oversized_metadata = request(Some(TOKEN), "30S");
        oversized_metadata.metadata_mut().insert(
            "x-padding",
            "x".repeat(MAX_METADATA_BYTES)
                .parse()
                .unwrap_or_else(|_| unreachable!()),
        );
        assert_eq!(
            access
                .authorize(&oversized_metadata, "CreateDevelopmentTask")
                .err()
                .map(|status| status.code()),
            Some(Code::ResourceExhausted)
        );
        for _ in 0..MAX_REQUESTS_PER_MINUTE {
            assert!(
                access
                    .authorize(&request(Some(TOKEN), "30S"), "CreateDevelopmentTask")
                    .is_ok()
            );
        }
        assert_eq!(
            access
                .authorize(&request(Some(TOKEN), "30S"), "CreateDevelopmentTask")
                .err()
                .map(|status| status.code()),
            Some(Code::ResourceExhausted)
        );
        let unknown = br#"{"version":1,"clients":[{"id":"client","token_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","expires_at_unix":10,"capabilities":["unknown"]}]}"#;
        assert!(matches!(
            Authorizer::from_json(unknown),
            Err(AccessConfigurationError::InvalidClients(_))
        ));
    }
}
