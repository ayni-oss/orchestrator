//! Durable runtime preparation state independent of profile, provider and storage codecs.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize)]
pub struct RuntimePreparation<P, I, T> {
    pub builder: BuilderResolution,
    pub sequence: u64,
    pub next_at: u64,
    pub last_success: Option<u64>,
    pub failures: u32,
    #[serde(default)]
    pub accounting: FailureAccounting,
    pub pending: Option<P>,
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub preparation: Option<I>,
    pub status: BTreeMap<String, String>,
    #[serde(default)]
    pub force_rebuild: bool,
    #[serde(default)]
    pub retry_trust: bool,
    #[serde(default)]
    pub trust: Option<T>,
}

impl<P, I, T> Default for RuntimePreparation<P, I, T> {
    fn default() -> Self {
        Self {
            builder: BuilderResolution::default(),
            sequence: 0,
            next_at: 0,
            last_success: None,
            failures: 0,
            accounting: FailureAccounting::default(),
            pending: None,
            bindings: BTreeMap::new(),
            preparation: None,
            status: BTreeMap::new(),
            force_rebuild: false,
            retry_trust: false,
            trust: None,
        }
    }
}

/// Authoritative execution phase. Intent phases are committed before creating workloads.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImagePreparationPhase {
    #[default]
    Discovering,
    DiscoveringStarted,
    Building,
    BuildingStarted,
    Publishing,
    PublicationReadback,
    PublicationReadbackStarted,
    Verifying,
    Activating,
}

impl ImagePreparationPhase {
    #[must_use]
    pub const fn discovering(self) -> bool {
        matches!(
            self,
            Self::Discovering
                | Self::DiscoveringStarted
                | Self::PublicationReadback
                | Self::PublicationReadbackStarted
        )
    }
    #[must_use]
    pub const fn discovery_started(self) -> bool {
        matches!(
            self,
            Self::DiscoveringStarted | Self::PublicationReadbackStarted
        )
    }
    #[must_use]
    pub const fn publication_observed(self) -> bool {
        matches!(
            self,
            Self::PublicationReadback
                | Self::PublicationReadbackStarted
                | Self::Verifying
                | Self::Activating
        )
    }
    #[must_use]
    pub const fn building(self) -> bool {
        matches!(
            self,
            Self::Building | Self::BuildingStarted | Self::Publishing
        )
    }
    #[must_use]
    pub const fn build_started(self) -> bool {
        !matches!(self, Self::Building)
    }
    pub const fn start_discovery(&mut self) {
        *self = if matches!(
            self,
            Self::PublicationReadback | Self::PublicationReadbackStarted
        ) {
            Self::PublicationReadbackStarted
        } else {
            Self::DiscoveringStarted
        };
    }
}

/// Frozen references retain credential revisions until all resumable work releases them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CredentialReferences {
    pub secrets: BTreeSet<String>,
    pub certificates: BTreeSet<String>,
}

pub trait RetainedCredentials {
    fn credential_references(&self) -> CredentialReferences;
}

impl<P: RetainedCredentials, I: RetainedCredentials, T: RetainedCredentials> RetainedCredentials
    for RuntimePreparation<P, I, T>
{
    fn credential_references(&self) -> CredentialReferences {
        let mut references = CredentialReferences::default();
        for refs in [
            self.pending
                .as_ref()
                .map(RetainedCredentials::credential_references),
            self.preparation
                .as_ref()
                .map(RetainedCredentials::credential_references),
            self.trust
                .as_ref()
                .map(RetainedCredentials::credential_references),
        ]
        .into_iter()
        .flatten()
        {
            references.secrets.extend(refs.secrets);
            references.certificates.extend(refs.certificates);
        }
        references
    }
}

/// Official builder lookup has one durable operation and one frozen resolution.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuilderResolution {
    pub contract: Option<String>,
    pub image: Option<String>,
    pub job: Option<String>,
    pub started_at: Option<u64>,
}
#[derive(Deserialize)]
#[serde(bound(deserialize = "P: Deserialize<'de>, I: Deserialize<'de>, T: Deserialize<'de>"))]
struct PreparationWire<P, I, T> {
    #[serde(default)]
    pub builder: Option<BuilderResolution>,
    pub sequence: u64,
    pub next_at: u64,
    pub last_success: Option<u64>,
    pub failures: u32,
    #[serde(default)]
    pub accounting: Option<FailureAccounting>,
    pub pending: Option<P>,
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub preparation: Option<I>,
    pub status: BTreeMap<String, String>,
    #[serde(default)]
    pub force_rebuild: bool,
    #[serde(default)]
    pub retry_trust: bool,
    #[serde(default)]
    pub trust: Option<T>,
}

impl<'de, P: Deserialize<'de>, I: Deserialize<'de>, T: Deserialize<'de>> Deserialize<'de>
    for RuntimePreparation<P, I, T>
{
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut wire = PreparationWire::deserialize(deserializer)?;
        let legacy = BuilderResolution {
            contract: wire.status.remove("builder-contract"),
            image: wire.status.remove("builder-image"),
            job: wire.status.remove("builder-job"),
            started_at: wire
                .status
                .remove("builder-started")
                .map(|value| value.parse::<u64>())
                .transpose()
                .map_err(|_| serde::de::Error::custom("invalid builder start timestamp"))?,
        };
        let builder = wire.builder.unwrap_or(legacy);
        if builder.started_at.is_some() && builder.job.is_none() {
            return Err(serde::de::Error::custom(
                "builder timestamp requires a durable job",
            ));
        }
        Ok(Self {
            builder,
            sequence: wire.sequence,
            next_at: wire.next_at,
            last_success: wire.last_success,
            failures: wire.failures,
            accounting: wire.accounting.unwrap_or_else(|| FailureAccounting {
                legacy_failures: wire.failures,
                ..FailureAccounting::default()
            }),
            pending: wire.pending,
            bindings: wire.bindings,
            preparation: wire.preparation,
            status: wire.status,
            force_rebuild: wire.force_rebuild,
            retry_trust: wire.retry_trust,
            trust: wire.trust,
        })
    }
}

/// Execution outcomes survive new retry admissions; operational errors never spend that budget.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FailureAccounting {
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub legacy_failures: u32,
    pub executions: BTreeMap<String, String>,
    pub admissions: BTreeSet<String>,
    pub operational: BTreeMap<FailureClass, u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Execution,
    Publication,
    Observation,
    Reporting,
    Cleanup,
}
impl FailureClass {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::Publication => "publication",
            Self::Observation => "observation",
            Self::Reporting => "reporting",
            Self::Cleanup => "cleanup",
        }
    }
}
