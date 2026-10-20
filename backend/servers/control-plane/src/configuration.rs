use std::{
    env, fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Configuration {
    http_bind_address: SocketAddr,
    grpc_bind_address: SocketAddr,
    migration_database_url_path: PathBuf,
    runtime_database_url_path: PathBuf,
    runtime_database_role: String,
}

impl Configuration {
    pub const DEFAULT_HTTP_BIND_ADDRESS: &'static str = "0.0.0.0:8080";
    pub const DEFAULT_GRPC_BIND_ADDRESS: &'static str = "127.0.0.1:50051";
    pub const DEFAULT_MIGRATION_DATABASE_URL_PATH: &'static str =
        "/var/run/secrets/ayni/workflow-postgres-migrator/url";
    pub const DEFAULT_RUNTIME_DATABASE_URL_PATH: &'static str =
        "/var/run/secrets/ayni/workflow-postgres-runtime/url";
    pub const DEFAULT_RUNTIME_DATABASE_ROLE: &'static str = "ayni_runtime";

    /// Read and validate the replacement control-plane configuration.
    ///
    /// # Errors
    /// Returns a safe configuration error for malformed addresses or secret paths.
    pub fn from_environment() -> Result<Self, ConfigurationError> {
        Self::from_values(
            &env::var("ORCHESTRATOR_HTTP_BIND_ADDRESS")
                .unwrap_or_else(|_| Self::DEFAULT_HTTP_BIND_ADDRESS.to_owned()),
            &env::var("ORCHESTRATOR_GRPC_BIND_ADDRESS")
                .unwrap_or_else(|_| Self::DEFAULT_GRPC_BIND_ADDRESS.to_owned()),
            PathBuf::from(
                env::var("ORCHESTRATOR_WORKFLOW_MIGRATION_DATABASE_URL_PATH")
                    .unwrap_or_else(|_| Self::DEFAULT_MIGRATION_DATABASE_URL_PATH.to_owned()),
            ),
            PathBuf::from(
                env::var("ORCHESTRATOR_WORKFLOW_DATABASE_URL_PATH")
                    .unwrap_or_else(|_| Self::DEFAULT_RUNTIME_DATABASE_URL_PATH.to_owned()),
            ),
            &env::var("ORCHESTRATOR_WORKFLOW_DATABASE_ROLE")
                .unwrap_or_else(|_| Self::DEFAULT_RUNTIME_DATABASE_ROLE.to_owned()),
        )
    }

    /// Construct explicit configuration for tests and local embedding.
    ///
    /// # Errors
    /// Returns a safe configuration error for malformed addresses or secret paths.
    pub fn from_values(
        http_bind_address: &str,
        grpc_bind_address: &str,
        migration_database_url_path: PathBuf,
        runtime_database_url_path: PathBuf,
        runtime_database_role: &str,
    ) -> Result<Self, ConfigurationError> {
        let http_bind_address = SocketAddr::from_str(http_bind_address)
            .map_err(|_| ConfigurationError::InvalidHttpAddress)?;
        let grpc_bind_address = SocketAddr::from_str(grpc_bind_address)
            .map_err(|_| ConfigurationError::InvalidGrpcAddress)?;
        if !grpc_bind_address.ip().is_loopback() || grpc_bind_address.port() == 0 {
            return Err(ConfigurationError::UnsafeGrpcAddress);
        }
        if http_bind_address.port() == grpc_bind_address.port() {
            return Err(ConfigurationError::DuplicateAddress);
        }
        validate_secret_path(&migration_database_url_path)?;
        validate_secret_path(&runtime_database_url_path)?;
        if migration_database_url_path == runtime_database_url_path {
            return Err(ConfigurationError::DuplicateDatabaseSecretPath);
        }
        validate_database_role(runtime_database_role)?;
        Ok(Self {
            http_bind_address,
            grpc_bind_address,
            migration_database_url_path,
            runtime_database_url_path,
            runtime_database_role: runtime_database_role.to_owned(),
        })
    }

    #[must_use]
    pub const fn http_bind_address(&self) -> SocketAddr {
        self.http_bind_address
    }

    #[must_use]
    pub const fn grpc_bind_address(&self) -> SocketAddr {
        self.grpc_bind_address
    }

    /// Load the migration-owner connection string from its mounted Secret file.
    ///
    /// # Errors
    /// Returns an I/O error without including secret contents.
    pub fn migration_database_url(&self) -> Result<String, io::Error> {
        read_secret(&self.migration_database_url_path)
    }

    /// Load the runtime-role connection string from its mounted Secret file.
    ///
    /// # Errors
    /// Returns an I/O error without including secret contents.
    pub fn runtime_database_url(&self) -> Result<String, io::Error> {
        read_secret(&self.runtime_database_url_path)
    }

    #[must_use]
    pub fn runtime_database_role(&self) -> &str {
        &self.runtime_database_role
    }
}

fn validate_secret_path(path: &Path) -> Result<(), ConfigurationError> {
    if !path.is_absolute() || path.as_os_str().is_empty() {
        Err(ConfigurationError::InvalidSecretPath)
    } else {
        Ok(())
    }
}

fn validate_database_role(role: &str) -> Result<(), ConfigurationError> {
    if role.is_empty()
        || role.len() > 63
        || !role.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_lowercase() || (index > 0 && byte.is_ascii_digit())
        })
    {
        Err(ConfigurationError::InvalidDatabaseRole)
    } else {
        Ok(())
    }
}

fn read_secret(path: &Path) -> Result<String, io::Error> {
    let value = fs::read_to_string(path)
        .map_err(|_| io::Error::other("workflow database Secret is unavailable"))?;
    let value = value.trim();
    if value.is_empty() || value.contains(['\n', '\r']) {
        return Err(io::Error::other("workflow database Secret is invalid"));
    }
    Ok(value.to_owned())
}

#[derive(Debug, Error)]
pub enum ConfigurationError {
    #[error("ORCHESTRATOR_HTTP_BIND_ADDRESS is invalid")]
    InvalidHttpAddress,
    #[error("ORCHESTRATOR_GRPC_BIND_ADDRESS is invalid")]
    InvalidGrpcAddress,
    #[error("the trusted workflow gRPC listener must use loopback and a nonzero port")]
    UnsafeGrpcAddress,
    #[error("HTTP and gRPC listener ports must differ")]
    DuplicateAddress,
    #[error("workflow database Secret paths must be absolute")]
    InvalidSecretPath,
    #[error("migration and runtime database Secret paths must differ")]
    DuplicateDatabaseSecretPath,
    #[error("ORCHESTRATOR_WORKFLOW_DATABASE_ROLE is invalid")]
    InvalidDatabaseRole,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_listener_requires_loopback() {
        assert!(matches!(
            Configuration::from_values(
                "0.0.0.0:8080",
                "0.0.0.0:50051",
                PathBuf::from("/migration"),
                PathBuf::from("/runtime"),
                "ayni_runtime",
            ),
            Err(ConfigurationError::UnsafeGrpcAddress)
        ));
        assert!(matches!(
            Configuration::from_values(
                "0.0.0.0:50051",
                "127.0.0.1:50051",
                PathBuf::from("/migration"),
                PathBuf::from("/runtime"),
                "ayni_runtime",
            ),
            Err(ConfigurationError::DuplicateAddress)
        ));
    }

    #[test]
    fn database_roles_and_secret_paths_remain_separate() {
        assert!(matches!(
            Configuration::from_values(
                "0.0.0.0:8080",
                "127.0.0.1:50051",
                PathBuf::from("/shared"),
                PathBuf::from("/shared"),
                "ayni_runtime",
            ),
            Err(ConfigurationError::DuplicateDatabaseSecretPath)
        ));
        assert!(matches!(
            Configuration::from_values(
                "0.0.0.0:8080",
                "127.0.0.1:50051",
                PathBuf::from("/migration"),
                PathBuf::from("/runtime"),
                "Migrator;DROP ROLE",
            ),
            Err(ConfigurationError::InvalidDatabaseRole)
        ));
    }
}
