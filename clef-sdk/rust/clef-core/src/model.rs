use std::{collections::BTreeMap, fmt};

use agentro_contracts::CapabilityName;

use crate::{ArtifactName, DomainFunctionName, ProjectPath, TaskId, WorkflowId};

/// Maximum tasks accepted by one in-memory vertical-slice workflow.
pub const MAX_TASKS: usize = 1_024;
/// Maximum explicit artifact bindings accepted by one workflow.
pub const MAX_BINDINGS: usize = 4_096;
/// Maximum input or output slots accepted by one task.
pub const MAX_ARTIFACTS_PER_TASK: usize = 64;
const MAX_CAPABILITY_REQUIREMENTS: usize = 64;
const MAX_EFFECT_RULES: usize = 128;
const MAX_INSTRUCTION_BYTES: usize = 64 * 1_024;

/// Stable normalized capability required by an explicit logical effort route.
pub const REASONING_EFFORT_CAPABILITY: &str = "agent.reasoning-effort";

/// Major/minor schema version attached to Clef domain values.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SchemaVersion {
    major: u16,
    minor: u16,
}

impl SchemaVersion {
    /// The initial greenfield Clef schema.
    pub const V1: Self = Self { major: 1, minor: 0 };
    /// The thin Python builder schema used by the alpha integration DTO.
    pub const V2: Self = Self { major: 2, minor: 0 };

    /// Creates a schema version with a non-zero major.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::InvalidSchemaVersion`] when `major` is zero.
    pub const fn new(major: u16, minor: u16) -> Result<Self, ModelError> {
        if major == 0 {
            return Err(ModelError::InvalidSchemaVersion);
        }
        Ok(Self { major, minor })
    }

    /// Returns the breaking schema major.
    #[must_use]
    pub const fn major(self) -> u16 {
        self.major
    }

    /// Returns the additive schema minor.
    #[must_use]
    pub const fn minor(self) -> u16 {
        self.minor
    }
}

/// Provider-independent logical model route selected by a task.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Effort {
    /// Highest logical Clef route.
    Xhigh,
    /// High logical Clef route.
    High,
    /// Medium logical Clef route.
    Medium,
    /// Low logical Clef route.
    Low,
}

impl Effort {
    /// Returns the stable route name, independent of provider-native variants.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Xhigh => "xhigh",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// Stable logical form of an artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ArtifactKind {
    /// One physical file.
    File,
    /// One physical directory tree.
    Directory,
    /// UTF-8 text content.
    Text,
    /// Strict JSON content.
    Json,
    /// A typed non-filesystem reference.
    Virtual,
}

impl ArtifactKind {
    pub(crate) const fn canonical_tag(self) -> u8 {
        match self {
            Self::File => 1,
            Self::Directory => 2,
            Self::Text => 3,
            Self::Json => 4,
            Self::Virtual => 5,
        }
    }
}

/// A versioned artifact output declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactSpec {
    schema_version: SchemaVersion,
    name: ArtifactName,
    description: Box<str>,
    kind: ArtifactKind,
    path: ProjectPath,
    is_required: bool,
}

impl ArtifactSpec {
    /// Creates a bounded artifact declaration.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::InvalidText`] when the description is empty or
    /// exceeds 4,096 bytes.
    pub fn new(
        schema_version: SchemaVersion,
        name: ArtifactName,
        description: &str,
        kind: ArtifactKind,
        path: ProjectPath,
        is_required: bool,
    ) -> Result<Self, ModelError> {
        validate_text(description, 4_096, "artifact description")?;
        Ok(Self {
            schema_version,
            name,
            description: description.into(),
            kind,
            path,
            is_required,
        })
    }

    /// Returns the artifact schema version.
    #[must_use]
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Returns the output slot name.
    #[must_use]
    pub const fn name(&self) -> &ArtifactName {
        &self.name
    }

    /// Returns the bounded user-facing description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the declared logical kind.
    #[must_use]
    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }

    /// Returns the canonical project-relative output path.
    #[must_use]
    pub const fn path(&self) -> &ProjectPath {
        &self.path
    }

    /// Returns whether the publish gate must observe this output.
    #[must_use]
    pub const fn is_required(&self) -> bool {
        self.is_required
    }
}

/// Provider-neutral effect category declared as task intent.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EffectKind {
    /// Read project content.
    Read,
    /// Create project content.
    Create,
    /// Modify project content.
    Modify,
    /// Move project content.
    Move,
    /// Delete project content.
    Delete,
    /// Run a supervised command.
    Shell,
    /// Access a network resource.
    Network,
}

impl EffectKind {
    pub(crate) const fn canonical_tag(self) -> u8 {
        match self {
            Self::Read => 1,
            Self::Create => 2,
            Self::Modify => 3,
            Self::Move => 4,
            Self::Delete => 5,
            Self::Shell => 6,
            Self::Network => 7,
        }
    }
}

/// One declared effect and optional exact project-relative scope.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EffectRule {
    kind: EffectKind,
    path: Option<ProjectPath>,
}

impl EffectRule {
    /// Creates a provider-neutral effect declaration.
    #[must_use]
    pub const fn new(kind: EffectKind, path: Option<ProjectPath>) -> Self {
        Self { kind, path }
    }

    /// Returns the declared effect category.
    #[must_use]
    pub const fn kind(&self) -> EffectKind {
        self.kind
    }

    /// Returns the optional exact project-relative scope.
    #[must_use]
    pub const fn path(&self) -> Option<&ProjectPath> {
        self.path.as_ref()
    }
}

/// A versioned, deterministic set of declared effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectPolicy {
    schema_version: SchemaVersion,
    allowed: Vec<EffectRule>,
}

impl EffectPolicy {
    /// Creates a bounded effect policy in canonical rule order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::LimitExceeded`] for more than 128 rules or
    /// [`ModelError::DuplicateValue`] for an exact duplicate.
    pub fn new(
        schema_version: SchemaVersion,
        mut allowed: Vec<EffectRule>,
    ) -> Result<Self, ModelError> {
        if allowed.len() > MAX_EFFECT_RULES {
            return Err(ModelError::LimitExceeded {
                resource: "effect rules",
                limit: MAX_EFFECT_RULES,
            });
        }
        allowed.sort();
        if allowed.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ModelError::DuplicateValue {
                field: "effect rules",
            });
        }
        Ok(Self {
            schema_version,
            allowed,
        })
    }

    /// Creates an empty versioned effect policy.
    #[must_use]
    pub const fn empty(schema_version: SchemaVersion) -> Self {
        Self {
            schema_version,
            allowed: Vec::new(),
        }
    }

    /// Returns the policy schema version.
    #[must_use]
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Returns rules in canonical order.
    #[must_use]
    pub fn allowed(&self) -> &[EffectRule] {
        &self.allowed
    }
}

/// A versioned immutable task declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSpec {
    schema_version: SchemaVersion,
    id: TaskId,
    domain_function: DomainFunctionName,
    instruction: Box<str>,
    inputs: BTreeMap<ArtifactName, ArtifactKind>,
    outputs: BTreeMap<ArtifactName, ArtifactSpec>,
    dependencies: Vec<TaskId>,
    required_capabilities: Vec<CapabilityName>,
    preferred_capabilities: Vec<CapabilityName>,
    effects: EffectPolicy,
    effort: Option<Effort>,
}

impl TaskSpec {
    /// Creates a task with no ports, dependencies, or capabilities.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::InvalidText`] when `instruction` is empty or
    /// exceeds 64 KiB.
    pub fn new(
        schema_version: SchemaVersion,
        id: TaskId,
        domain_function: DomainFunctionName,
        instruction: &str,
    ) -> Result<Self, ModelError> {
        validate_text(instruction, MAX_INSTRUCTION_BYTES, "task instruction")?;
        Ok(Self {
            schema_version,
            id,
            domain_function,
            instruction: instruction.into(),
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            dependencies: Vec::new(),
            required_capabilities: Vec::new(),
            preferred_capabilities: Vec::new(),
            effects: EffectPolicy::empty(schema_version),
            effort: None,
        })
    }

    /// Adds a typed input slot.
    ///
    /// # Errors
    ///
    /// Returns a typed model error for duplicate slots or the slot-count limit.
    pub fn with_input(
        mut self,
        name: ArtifactName,
        kind: ArtifactKind,
    ) -> Result<Self, ModelError> {
        if self.inputs.len() >= MAX_ARTIFACTS_PER_TASK {
            return Err(ModelError::LimitExceeded {
                resource: "task inputs",
                limit: MAX_ARTIFACTS_PER_TASK,
            });
        }
        if self.inputs.insert(name, kind).is_some() {
            return Err(ModelError::DuplicateValue {
                field: "task inputs",
            });
        }
        Ok(self)
    }

    /// Adds a typed output declaration whose map key is its own name.
    ///
    /// # Errors
    ///
    /// Returns a typed model error for duplicate outputs or the output limit.
    pub fn with_output(mut self, output: ArtifactSpec) -> Result<Self, ModelError> {
        if self.outputs.len() >= MAX_ARTIFACTS_PER_TASK {
            return Err(ModelError::LimitExceeded {
                resource: "task outputs",
                limit: MAX_ARTIFACTS_PER_TASK,
            });
        }
        if self.outputs.insert(output.name.clone(), output).is_some() {
            return Err(ModelError::DuplicateValue {
                field: "task outputs",
            });
        }
        Ok(self)
    }

    /// Adds an explicit task dependency.
    ///
    /// Duplicate dependencies are rejected; missing and self references are
    /// diagnosed by compilation where the whole graph is available.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateValue`] for a duplicate dependency.
    pub fn with_dependency(mut self, dependency: TaskId) -> Result<Self, ModelError> {
        if self.dependencies.contains(&dependency) {
            return Err(ModelError::DuplicateValue {
                field: "task dependencies",
            });
        }
        self.dependencies.push(dependency);
        self.dependencies.sort();
        Ok(self)
    }

    /// Adds one capability that must be negotiated before the task starts.
    ///
    /// # Errors
    ///
    /// Returns a model error for duplicate/conflicting entries or the hard limit.
    pub fn requiring_capability(mut self, capability: CapabilityName) -> Result<Self, ModelError> {
        self.insert_capability(capability, true)?;
        Ok(self)
    }

    /// Adds one preferred capability whose absence is an explicit degradation.
    ///
    /// # Errors
    ///
    /// Returns a model error for duplicate/conflicting entries or the hard limit.
    pub fn preferring_capability(mut self, capability: CapabilityName) -> Result<Self, ModelError> {
        self.insert_capability(capability, false)?;
        Ok(self)
    }

    fn insert_capability(
        &mut self,
        capability: CapabilityName,
        is_required: bool,
    ) -> Result<(), ModelError> {
        let (target, other) = if is_required {
            (
                &mut self.required_capabilities,
                &self.preferred_capabilities,
            )
        } else {
            (
                &mut self.preferred_capabilities,
                &self.required_capabilities,
            )
        };
        if target.len() >= MAX_CAPABILITY_REQUIREMENTS {
            return Err(ModelError::LimitExceeded {
                resource: "task capability requirements",
                limit: MAX_CAPABILITY_REQUIREMENTS,
            });
        }
        if target.contains(&capability) {
            return Err(ModelError::DuplicateValue {
                field: "task capability requirements",
            });
        }
        if other.contains(&capability) {
            return Err(ModelError::ConflictingCapability);
        }
        target.push(capability);
        target.sort();
        Ok(())
    }

    /// Replaces the task's effect declarations.
    #[must_use]
    pub fn with_effects(mut self, effects: EffectPolicy) -> Self {
        self.effects = effects;
        self
    }

    /// Selects a logical effort route without naming a provider model/variant.
    #[must_use]
    pub fn with_effort(mut self, effort: Effort) -> Self {
        self.effort = Some(effort);
        self
    }

    /// Returns the task schema version.
    #[must_use]
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Returns the stable task identifier.
    #[must_use]
    pub const fn id(&self) -> &TaskId {
        &self.id
    }

    /// Returns the registered provider-neutral domain function.
    #[must_use]
    pub const fn domain_function(&self) -> &DomainFunctionName {
        &self.domain_function
    }

    /// Returns the bounded task instruction.
    #[must_use]
    pub fn instruction(&self) -> &str {
        &self.instruction
    }

    /// Returns typed input slots in canonical name order.
    #[must_use]
    pub const fn inputs(&self) -> &BTreeMap<ArtifactName, ArtifactKind> {
        &self.inputs
    }

    /// Returns output declarations in canonical name order.
    #[must_use]
    pub const fn outputs(&self) -> &BTreeMap<ArtifactName, ArtifactSpec> {
        &self.outputs
    }

    /// Returns explicit dependencies in canonical task-ID order.
    #[must_use]
    pub fn dependencies(&self) -> &[TaskId] {
        &self.dependencies
    }

    /// Returns hard capability requirements in canonical name order.
    #[must_use]
    pub fn required_capabilities(&self) -> &[CapabilityName] {
        &self.required_capabilities
    }

    /// Returns preferred capabilities in canonical name order.
    #[must_use]
    pub fn preferred_capabilities(&self) -> &[CapabilityName] {
        &self.preferred_capabilities
    }

    /// Returns provider-neutral effect declarations.
    #[must_use]
    pub const fn effects(&self) -> &EffectPolicy {
        &self.effects
    }

    /// Returns the optional logical effort route.
    #[must_use]
    pub const fn effort(&self) -> Option<Effort> {
        self.effort
    }
}

/// A typed artifact flow edge between two task ports.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ArtifactBinding {
    source_task_id: TaskId,
    output_name: ArtifactName,
    target_task_id: TaskId,
    input_name: ArtifactName,
}

impl ArtifactBinding {
    /// Creates one explicit artifact edge.
    #[must_use]
    pub const fn new(
        source_task_id: TaskId,
        output_name: ArtifactName,
        target_task_id: TaskId,
        input_name: ArtifactName,
    ) -> Self {
        Self {
            source_task_id,
            output_name,
            target_task_id,
            input_name,
        }
    }

    /// Returns the producer task.
    #[must_use]
    pub const fn source_task_id(&self) -> &TaskId {
        &self.source_task_id
    }

    /// Returns the producer output slot.
    #[must_use]
    pub const fn output_name(&self) -> &ArtifactName {
        &self.output_name
    }

    /// Returns the consumer task.
    #[must_use]
    pub const fn target_task_id(&self) -> &TaskId {
        &self.target_task_id
    }

    /// Returns the consumer input slot.
    #[must_use]
    pub const fn input_name(&self) -> &ArtifactName {
        &self.input_name
    }
}

/// Versioned workflow scheduling and failure policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowPolicy {
    schema_version: SchemaVersion,
    max_concurrency: u16,
    max_fan_out: u16,
    is_fail_fast: bool,
}

impl WorkflowPolicy {
    /// Creates a bounded workflow policy.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::InvalidConcurrency`] when either limit is zero or
    /// maximum concurrency exceeds 1,024.
    pub const fn new(
        schema_version: SchemaVersion,
        max_concurrency: u16,
        max_fan_out: u16,
        is_fail_fast: bool,
    ) -> Result<Self, ModelError> {
        if max_concurrency == 0 || max_concurrency > MAX_TASKS as u16 || max_fan_out == 0 {
            return Err(ModelError::InvalidConcurrency);
        }
        Ok(Self {
            schema_version,
            max_concurrency,
            max_fan_out,
            is_fail_fast,
        })
    }

    /// Returns the policy schema version.
    #[must_use]
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Returns the maximum number of simultaneously running tasks.
    #[must_use]
    pub const fn max_concurrency(&self) -> u16 {
        self.max_concurrency
    }

    /// Returns the maximum number of direct downstream tasks per task.
    #[must_use]
    pub const fn max_fan_out(&self) -> u16 {
        self.max_fan_out
    }

    /// Returns whether the scheduler stops admitting work after a failure.
    #[must_use]
    pub const fn is_fail_fast(&self) -> bool {
        self.is_fail_fast
    }
}

/// Immutable versioned Workflow/Task/Artifact/Policy aggregate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowSpec {
    schema_version: SchemaVersion,
    id: WorkflowId,
    tasks: Vec<TaskSpec>,
    bindings: Vec<ArtifactBinding>,
    policy: WorkflowPolicy,
}

impl WorkflowSpec {
    /// Creates a bounded workflow definition while preserving task declaration order.
    ///
    /// Empty and duplicate task sets are retained for the compiler to diagnose.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::LimitExceeded`] when a graph budget is exceeded.
    pub fn new(
        schema_version: SchemaVersion,
        id: WorkflowId,
        tasks: Vec<TaskSpec>,
        bindings: Vec<ArtifactBinding>,
        policy: WorkflowPolicy,
    ) -> Result<Self, ModelError> {
        if tasks.len() > MAX_TASKS {
            return Err(ModelError::LimitExceeded {
                resource: "workflow tasks",
                limit: MAX_TASKS,
            });
        }
        if bindings.len() > MAX_BINDINGS {
            return Err(ModelError::LimitExceeded {
                resource: "artifact bindings",
                limit: MAX_BINDINGS,
            });
        }
        Ok(Self {
            schema_version,
            id,
            tasks,
            bindings,
            policy,
        })
    }

    /// Returns the workflow schema version.
    #[must_use]
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Returns the stable workflow identifier.
    #[must_use]
    pub const fn id(&self) -> &WorkflowId {
        &self.id
    }

    /// Returns tasks in explicit declaration order.
    #[must_use]
    pub fn tasks(&self) -> &[TaskSpec] {
        &self.tasks
    }

    /// Returns explicit bindings in input order; compilation canonicalizes them.
    #[must_use]
    pub fn bindings(&self) -> &[ArtifactBinding] {
        &self.bindings
    }

    /// Returns the versioned scheduling policy.
    #[must_use]
    pub const fn policy(&self) -> &WorkflowPolicy {
        &self.policy
    }
}

fn validate_text(value: &str, maximum: usize, field: &'static str) -> Result<(), ModelError> {
    if value.trim().is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(ModelError::InvalidText { field, maximum });
    }
    Ok(())
}

/// A bounded Clef value-object construction error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelError {
    /// Schema major zero is reserved for unspecified wire values.
    InvalidSchemaVersion,
    /// A bounded collection exceeds its hard limit.
    LimitExceeded {
        /// Stable resource label.
        resource: &'static str,
        /// Maximum accepted items.
        limit: usize,
    },
    /// A bounded text field is empty, contains NUL, or is oversized.
    InvalidText {
        /// Stable field label.
        field: &'static str,
        /// Maximum accepted bytes.
        maximum: usize,
    },
    /// A set contains an exact duplicate.
    DuplicateValue {
        /// Stable set label.
        field: &'static str,
    },
    /// One capability was declared as both required and preferred.
    ConflictingCapability,
    /// A workflow concurrency or fan-out limit is invalid.
    InvalidConcurrency,
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchemaVersion => formatter.write_str("schema major must be non-zero"),
            Self::LimitExceeded { resource, limit } => {
                write!(formatter, "{resource} exceeds its limit of {limit}")
            }
            Self::InvalidText { field, maximum } => {
                write!(
                    formatter,
                    "{field} must be non-empty, NUL-free, and at most {maximum} bytes"
                )
            }
            Self::DuplicateValue { field } => write!(formatter, "{field} contains a duplicate"),
            Self::ConflictingCapability => {
                formatter.write_str("a capability cannot be both required and preferred")
            }
            Self::InvalidConcurrency => {
                formatter.write_str("workflow concurrency limits are invalid")
            }
        }
    }
}

impl std::error::Error for ModelError {}
