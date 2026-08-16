use std::collections::BTreeMap;

use crate::{ArtifactKind, ArtifactName, AttemptNumber, TaskSpec};

/// One normalized artifact reported by a backend before publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProducedArtifact {
    name: ArtifactName,
    kind: ArtifactKind,
}

impl ProducedArtifact {
    /// Creates a normalized artifact result.
    #[must_use]
    pub const fn new(name: ArtifactName, kind: ArtifactKind) -> Self {
        Self { name, kind }
    }

    /// Returns the output slot name.
    #[must_use]
    pub const fn name(&self) -> &ArtifactName {
        &self.name
    }

    /// Returns the observed logical kind.
    #[must_use]
    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }
}

/// Immutable input to a task's final publish gate.
#[derive(Clone, Copy, Debug)]
pub struct PublishRequest<'a> {
    task: &'a TaskSpec,
    attempt: AttemptNumber,
    produced: &'a BTreeMap<ArtifactName, ProducedArtifact>,
    verification_passed: bool,
}

impl<'a> PublishRequest<'a> {
    /// Creates a publish request from already normalized values.
    #[must_use]
    pub const fn new(
        task: &'a TaskSpec,
        attempt: AttemptNumber,
        produced: &'a BTreeMap<ArtifactName, ProducedArtifact>,
        verification_passed: bool,
    ) -> Self {
        Self {
            task,
            attempt,
            produced,
            verification_passed,
        }
    }

    /// Returns the task whose outputs are being considered.
    #[must_use]
    pub const fn task(self) -> &'a TaskSpec {
        self.task
    }

    /// Returns the one-based attempt number.
    #[must_use]
    pub const fn attempt(self) -> AttemptNumber {
        self.attempt
    }

    /// Returns normalized artifacts in canonical slot order.
    #[must_use]
    pub const fn produced(self) -> &'a BTreeMap<ArtifactName, ProducedArtifact> {
        self.produced
    }

    /// Returns whether all required deterministic checks passed.
    #[must_use]
    pub const fn verification_passed(self) -> bool {
        self.verification_passed
    }
}

/// Stable reason that a publish gate rejected candidate outputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishRejection {
    /// Deterministic verification did not pass.
    VerificationFailed,
    /// A required output slot was absent.
    MissingRequiredArtifact {
        /// Missing output slot.
        name: ArtifactName,
    },
    /// A produced output had the wrong logical kind.
    ArtifactKindMismatch {
        /// Mismatched output slot.
        name: ArtifactName,
        /// Kind declared by the task.
        expected: ArtifactKind,
        /// Kind reported by the backend.
        actual: ArtifactKind,
    },
}

/// Final result of a publish gate evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishDecision {
    /// The application may atomically publish and mark success.
    Approved,
    /// Publication is rejected with a stable machine reason.
    Rejected(PublishRejection),
}

/// Provider-independent policy port controlling final artifact publication.
pub trait PublishGate: Send + Sync {
    /// Evaluates already verified, normalized candidate outputs.
    ///
    /// Implementations must be deterministic for the same request and must not
    /// mutate the workspace. The application owns the actual publication step.
    fn evaluate(&self, request: PublishRequest<'_>) -> PublishDecision;
}

/// Minimal gate requiring verification and every required typed output.
#[derive(Clone, Copy, Debug, Default)]
pub struct RequiredOutputsGate;

impl PublishGate for RequiredOutputsGate {
    fn evaluate(&self, request: PublishRequest<'_>) -> PublishDecision {
        if !request.verification_passed() {
            return PublishDecision::Rejected(PublishRejection::VerificationFailed);
        }
        for (name, specification) in request.task().outputs() {
            let Some(produced) = request.produced().get(name) else {
                if specification.is_required() {
                    return PublishDecision::Rejected(PublishRejection::MissingRequiredArtifact {
                        name: name.clone(),
                    });
                }
                continue;
            };
            if produced.kind() != specification.kind() {
                return PublishDecision::Rejected(PublishRejection::ArtifactKindMismatch {
                    name: name.clone(),
                    expected: specification.kind(),
                    actual: produced.kind(),
                });
            }
        }
        PublishDecision::Approved
    }
}
