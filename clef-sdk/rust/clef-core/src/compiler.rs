use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use agentro_contracts::{
    CanonicalHasher, CapabilityError, CapabilityName, CapabilitySet, DigestError, Sha256Digest,
};

use crate::{
    ArtifactBinding, ArtifactName, EffectRule, Effort, REASONING_EFFORT_CAPABILITY, SchemaVersion,
    TaskId, TaskSpec, WorkflowId, WorkflowPolicy, WorkflowSpec,
};

const PLAN_DIGEST_FORMAT: u16 = 1;

/// Immutable external inputs that may affect compilation and its digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileContext {
    capabilities: CapabilitySet,
    profile_revision: Sha256Digest,
}

impl CompileContext {
    /// Creates deterministic compiler inputs from negotiated capabilities and
    /// the caller's immutable profile revision.
    #[must_use]
    pub const fn new(capabilities: CapabilitySet, profile_revision: Sha256Digest) -> Self {
        Self {
            capabilities,
            profile_revision,
        }
    }

    /// Returns capabilities negotiated from the selected backend.
    #[must_use]
    pub const fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Returns the immutable caller profile revision included in the plan digest.
    #[must_use]
    pub const fn profile_revision(&self) -> Sha256Digest {
        self.profile_revision
    }
}

/// Stable category for one static workflow validation failure.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CompileIssueCode {
    /// The workflow schema is not supported by this compiler.
    UnsupportedSchema,
    /// A nested value uses a schema different from its workflow.
    SchemaMismatch,
    /// A workflow has no tasks.
    EmptyWorkflow,
    /// Two task declarations use the same ID.
    DuplicateTask,
    /// An explicit dependency references an unknown task.
    MissingDependency,
    /// A binding references an unknown source task.
    MissingSourceTask,
    /// A binding references an unknown target task.
    MissingTargetTask,
    /// A binding references an unknown producer output.
    MissingSourceOutput,
    /// A binding references an unknown consumer input.
    MissingTargetInput,
    /// A task or binding creates a self-edge.
    SelfEdge,
    /// An exact artifact binding is duplicated.
    DuplicateBinding,
    /// Multiple producers target one consumer input.
    DuplicateInputBinding,
    /// Producer and consumer artifact kinds differ.
    ArtifactKindMismatch,
    /// Two outputs use the same portable path ignoring ASCII case.
    DuplicateOutputPath,
    /// A required backend capability was not negotiated.
    CapabilityMissing,
    /// A task's direct downstream count exceeds policy.
    FanOutExceeded,
    /// The graph contains a directed cycle.
    Cycle,
}

/// One bounded, typed validation issue with stable related domain IDs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationIssue {
    code: CompileIssueCode,
    task_id: Option<TaskId>,
    related_tasks: Vec<TaskId>,
    subject: Option<Box<str>>,
}

impl ValidationIssue {
    fn new(
        code: CompileIssueCode,
        task_id: Option<TaskId>,
        related_tasks: Vec<TaskId>,
        subject: Option<String>,
    ) -> Self {
        Self {
            code,
            task_id,
            related_tasks,
            subject: subject.map(Into::into),
        }
    }

    /// Returns the stable issue category.
    #[must_use]
    pub const fn code(&self) -> CompileIssueCode {
        self.code
    }

    /// Returns the primary task, when one is meaningful.
    #[must_use]
    pub const fn task_id(&self) -> Option<&TaskId> {
        self.task_id.as_ref()
    }

    /// Returns a deterministic cycle/path or other related task IDs.
    #[must_use]
    pub fn related_tasks(&self) -> &[TaskId] {
        &self.related_tasks
    }

    /// Returns a bounded capability, artifact, or path subject.
    #[must_use]
    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }
}

/// Aggregate validation failure in deterministic issue order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationReport {
    issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    /// Returns all validation issues in stable compiler order.
    #[must_use]
    pub fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }
}

/// Static compilation or canonical digest failure.
#[derive(Debug)]
pub enum CompileError {
    /// The workflow violates one or more static domain invariants.
    Validation(ValidationReport),
    /// Canonical digest framing rejected an internally bounded payload.
    Digest(DigestError),
    /// A built-in normalized capability name was invalid.
    Capability(CapabilityError),
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(report) => write!(
                formatter,
                "workflow validation failed with {} issue(s)",
                report.issues.len()
            ),
            Self::Digest(error) => write!(formatter, "plan digest failed: {error}"),
            Self::Capability(error) => {
                write!(formatter, "built-in capability is invalid: {error}")
            }
        }
    }
}

impl std::error::Error for CompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Validation(_) => None,
            Self::Digest(error) => Some(error),
            Self::Capability(error) => Some(error),
        }
    }
}

impl From<DigestError> for CompileError {
    fn from(error: DigestError) -> Self {
        Self::Digest(error)
    }
}

impl From<CapabilityError> for CompileError {
    fn from(error: CapabilityError) -> Self {
        Self::Capability(error)
    }
}

/// One normalized task with deterministic scheduling metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledTask {
    spec: TaskSpec,
    declaration_index: u32,
    level: u32,
    missing_preferred_capabilities: Vec<CapabilityName>,
}

impl CompiledTask {
    /// Returns the immutable normalized task definition.
    #[must_use]
    pub const fn spec(&self) -> &TaskSpec {
        &self.spec
    }

    /// Returns the explicit zero-based declaration order.
    #[must_use]
    pub const fn declaration_index(&self) -> u32 {
        self.declaration_index
    }

    /// Returns the maximum predecessor depth.
    #[must_use]
    pub const fn level(&self) -> u32 {
        self.level
    }

    /// Returns preferred capabilities that were not negotiated.
    #[must_use]
    pub fn missing_preferred_capabilities(&self) -> &[CapabilityName] {
        &self.missing_preferred_capabilities
    }
}

/// One deterministic static set of tasks that can be ready at the same level.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadySet {
    level: u32,
    tasks: Vec<TaskId>,
}

impl ReadySet {
    /// Returns the topological depth represented by this set.
    #[must_use]
    pub const fn level(&self) -> u32 {
        self.level
    }

    /// Returns tasks in declaration-order then stable-ID order.
    #[must_use]
    pub fn tasks(&self) -> &[TaskId] {
        &self.tasks
    }
}

/// Deterministic compiled DAG and canonical plan identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionPlan {
    schema_version: SchemaVersion,
    workflow_id: WorkflowId,
    digest: Sha256Digest,
    profile_revision: Sha256Digest,
    policy: WorkflowPolicy,
    tasks: Vec<CompiledTask>,
    positions: BTreeMap<TaskId, usize>,
    predecessors: BTreeMap<TaskId, BTreeSet<TaskId>>,
    successors: BTreeMap<TaskId, BTreeSet<TaskId>>,
    ready_sets: Vec<ReadySet>,
}

impl ExecutionPlan {
    /// Returns the supported plan schema.
    #[must_use]
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Returns the source workflow ID.
    #[must_use]
    pub const fn workflow_id(&self) -> &WorkflowId {
        &self.workflow_id
    }

    /// Returns the versioned canonical SHA-256 identity.
    #[must_use]
    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    /// Returns the profile revision frozen into the digest.
    #[must_use]
    pub const fn profile_revision(&self) -> Sha256Digest {
        self.profile_revision
    }

    /// Returns the bounded scheduling policy.
    #[must_use]
    pub const fn policy(&self) -> &WorkflowPolicy {
        &self.policy
    }

    /// Returns tasks in deterministic topological order.
    #[must_use]
    pub fn tasks(&self) -> &[CompiledTask] {
        &self.tasks
    }

    /// Finds one compiled task by stable domain ID.
    #[must_use]
    pub fn task(&self, task_id: &TaskId) -> Option<&CompiledTask> {
        self.positions
            .get(task_id)
            .and_then(|position| self.tasks.get(*position))
    }

    /// Returns predecessor IDs in stable order.
    #[must_use]
    pub fn predecessors(&self, task_id: &TaskId) -> Option<&BTreeSet<TaskId>> {
        self.predecessors.get(task_id)
    }

    /// Returns successor IDs in stable order.
    #[must_use]
    pub fn successors(&self, task_id: &TaskId) -> Option<&BTreeSet<TaskId>> {
        self.successors.get(task_id)
    }

    /// Returns deterministic static ready sets grouped by topological depth.
    #[must_use]
    pub fn ready_sets(&self) -> &[ReadySet] {
        &self.ready_sets
    }
}

/// Validates and compiles one workflow into a deterministic execution plan.
///
/// Compilation performs schema checks, reference and port resolution,
/// capability checks, cycle detection, stable topological ordering, ready-set
/// construction, and versioned canonical SHA-256 digesting. It reads no clock,
/// random source, filesystem, environment, or provider-specific state.
///
/// # Errors
///
/// Returns [`CompileError::Validation`] with all discovered static issues or a
/// canonical framing error if an internal hard bound is violated.
pub fn compile_workflow(
    definition: &WorkflowSpec,
    context: &CompileContext,
) -> Result<ExecutionPlan, CompileError> {
    let mut issues = Vec::new();
    validate_schema(definition, &mut issues);
    if definition.tasks().is_empty() {
        issues.push(ValidationIssue::new(
            CompileIssueCode::EmptyWorkflow,
            None,
            Vec::new(),
            None,
        ));
    }

    let mut declaration_indices = BTreeMap::new();
    for (index, task) in definition.tasks().iter().enumerate() {
        if declaration_indices
            .insert(task.id().clone(), index)
            .is_some()
        {
            issues.push(ValidationIssue::new(
                CompileIssueCode::DuplicateTask,
                Some(task.id().clone()),
                Vec::new(),
                None,
            ));
        }
    }
    if issues.iter().any(|issue| {
        matches!(
            issue.code,
            CompileIssueCode::UnsupportedSchema
                | CompileIssueCode::SchemaMismatch
                | CompileIssueCode::EmptyWorkflow
                | CompileIssueCode::DuplicateTask
        )
    }) {
        return Err(validation_error(issues));
    }

    let tasks_by_id: BTreeMap<TaskId, &TaskSpec> = definition
        .tasks()
        .iter()
        .map(|task| (task.id().clone(), task))
        .collect();
    validate_capabilities(definition, context, &mut issues)?;
    validate_output_paths(definition, &mut issues);

    let mut predecessors: BTreeMap<TaskId, BTreeSet<TaskId>> = tasks_by_id
        .keys()
        .cloned()
        .map(|task_id| (task_id, BTreeSet::new()))
        .collect();
    let mut successors = predecessors.clone();
    validate_dependencies(
        definition,
        &tasks_by_id,
        &mut predecessors,
        &mut successors,
        &mut issues,
    );
    validate_bindings(
        definition.bindings(),
        &tasks_by_id,
        &mut predecessors,
        &mut successors,
        &mut issues,
    );
    validate_fan_out(definition.policy(), &successors, &mut issues);
    if !issues.is_empty() {
        return Err(validation_error(issues));
    }

    let (order, levels) = topological_order(&declaration_indices, &predecessors, &successors);
    if order.len() != definition.tasks().len() {
        let cycle = find_cycle(&declaration_indices, &successors);
        issues.push(ValidationIssue::new(
            CompileIssueCode::Cycle,
            cycle.first().cloned(),
            cycle,
            None,
        ));
        return Err(validation_error(issues));
    }

    let mut tasks = Vec::with_capacity(order.len());
    for task_id in &order {
        let Some(spec) = tasks_by_id.get(task_id) else {
            continue;
        };
        let missing_preferred_capabilities = spec
            .preferred_capabilities()
            .iter()
            .filter(|capability| !context.capabilities().contains(capability))
            .cloned()
            .collect();
        let declaration_index = declaration_indices
            .get(task_id)
            .copied()
            .unwrap_or_default() as u32;
        let level = levels.get(task_id).copied().unwrap_or_default();
        tasks.push(CompiledTask {
            spec: (**spec).clone(),
            declaration_index,
            level,
            missing_preferred_capabilities,
        });
    }
    let positions = tasks
        .iter()
        .enumerate()
        .map(|(index, task)| (task.spec.id().clone(), index))
        .collect();
    let ready_sets = build_ready_sets(&tasks);
    let digest = plan_digest(
        definition,
        context,
        &tasks,
        &predecessors,
        &successors,
        &ready_sets,
    )?;

    Ok(ExecutionPlan {
        schema_version: definition.schema_version(),
        workflow_id: definition.id().clone(),
        digest,
        profile_revision: context.profile_revision(),
        policy: definition.policy().clone(),
        tasks,
        positions,
        predecessors,
        successors,
        ready_sets,
    })
}

fn validation_error(issues: Vec<ValidationIssue>) -> CompileError {
    CompileError::Validation(ValidationReport { issues })
}

fn validate_schema(definition: &WorkflowSpec, issues: &mut Vec<ValidationIssue>) {
    let schema = definition.schema_version();
    if schema != SchemaVersion::V1 && schema != SchemaVersion::V2 {
        issues.push(ValidationIssue::new(
            CompileIssueCode::UnsupportedSchema,
            None,
            Vec::new(),
            Some(format!("{}.{}", schema.major(), schema.minor())),
        ));
    }
    if definition.policy().schema_version() != schema {
        issues.push(ValidationIssue::new(
            CompileIssueCode::SchemaMismatch,
            None,
            Vec::new(),
            Some("workflow policy".to_owned()),
        ));
    }
    for task in definition.tasks() {
        if task.schema_version() != schema || task.effects().schema_version() != schema {
            issues.push(ValidationIssue::new(
                CompileIssueCode::SchemaMismatch,
                Some(task.id().clone()),
                Vec::new(),
                Some("task or effect policy".to_owned()),
            ));
        }
        for output in task.outputs().values() {
            if output.schema_version() != schema {
                issues.push(ValidationIssue::new(
                    CompileIssueCode::SchemaMismatch,
                    Some(task.id().clone()),
                    Vec::new(),
                    Some(output.name().to_string()),
                ));
            }
        }
    }
}

fn validate_capabilities(
    definition: &WorkflowSpec,
    context: &CompileContext,
    issues: &mut Vec<ValidationIssue>,
) -> Result<(), CompileError> {
    let effort_capability = CapabilityName::parse(REASONING_EFFORT_CAPABILITY)?;
    for task in definition.tasks() {
        for capability in task.required_capabilities() {
            if !context.capabilities().contains(capability) {
                issues.push(ValidationIssue::new(
                    CompileIssueCode::CapabilityMissing,
                    Some(task.id().clone()),
                    Vec::new(),
                    Some(capability.to_string()),
                ));
            }
        }
        if task.effort().is_some()
            && !task.required_capabilities().contains(&effort_capability)
            && !context.capabilities().contains(&effort_capability)
        {
            issues.push(ValidationIssue::new(
                CompileIssueCode::CapabilityMissing,
                Some(task.id().clone()),
                Vec::new(),
                Some(effort_capability.to_string()),
            ));
        }
    }
    Ok(())
}

fn validate_output_paths(definition: &WorkflowSpec, issues: &mut Vec<ValidationIssue>) {
    let mut paths: BTreeMap<String, (TaskId, ArtifactName)> = BTreeMap::new();
    for task in definition.tasks() {
        for output in task.outputs().values() {
            let key = output.path().as_str().to_ascii_lowercase();
            if let Some((previous_task, _)) = paths.get(&key) {
                issues.push(ValidationIssue::new(
                    CompileIssueCode::DuplicateOutputPath,
                    Some(task.id().clone()),
                    vec![previous_task.clone(), task.id().clone()],
                    Some(output.path().to_string()),
                ));
            } else {
                paths.insert(key, (task.id().clone(), output.name().clone()));
            }
        }
    }
}

fn validate_dependencies(
    definition: &WorkflowSpec,
    tasks: &BTreeMap<TaskId, &TaskSpec>,
    predecessors: &mut BTreeMap<TaskId, BTreeSet<TaskId>>,
    successors: &mut BTreeMap<TaskId, BTreeSet<TaskId>>,
    issues: &mut Vec<ValidationIssue>,
) {
    for task in definition.tasks() {
        for dependency in task.dependencies() {
            if dependency == task.id() {
                issues.push(ValidationIssue::new(
                    CompileIssueCode::SelfEdge,
                    Some(task.id().clone()),
                    vec![task.id().clone()],
                    None,
                ));
                continue;
            }
            if !tasks.contains_key(dependency) {
                issues.push(ValidationIssue::new(
                    CompileIssueCode::MissingDependency,
                    Some(task.id().clone()),
                    vec![dependency.clone()],
                    None,
                ));
                continue;
            }
            add_edge(dependency, task.id(), predecessors, successors);
        }
    }
}

fn validate_bindings(
    bindings: &[ArtifactBinding],
    tasks: &BTreeMap<TaskId, &TaskSpec>,
    predecessors: &mut BTreeMap<TaskId, BTreeSet<TaskId>>,
    successors: &mut BTreeMap<TaskId, BTreeSet<TaskId>>,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut canonical = bindings.to_vec();
    canonical.sort();
    let mut previous: Option<&ArtifactBinding> = None;
    let mut target_slots = BTreeSet::new();
    for binding in &canonical {
        if previous == Some(binding) {
            issues.push(ValidationIssue::new(
                CompileIssueCode::DuplicateBinding,
                Some(binding.target_task_id().clone()),
                vec![
                    binding.source_task_id().clone(),
                    binding.target_task_id().clone(),
                ],
                Some(binding.input_name().to_string()),
            ));
        }
        previous = Some(binding);

        let source = tasks.get(binding.source_task_id());
        let target = tasks.get(binding.target_task_id());
        if source.is_none() {
            issues.push(ValidationIssue::new(
                CompileIssueCode::MissingSourceTask,
                Some(binding.target_task_id().clone()),
                vec![binding.source_task_id().clone()],
                None,
            ));
        }
        if target.is_none() {
            issues.push(ValidationIssue::new(
                CompileIssueCode::MissingTargetTask,
                Some(binding.target_task_id().clone()),
                Vec::new(),
                None,
            ));
        }
        let (Some(source), Some(target)) = (source, target) else {
            continue;
        };
        let output = source.outputs().get(binding.output_name());
        let input = target.inputs().get(binding.input_name());
        if output.is_none() {
            issues.push(ValidationIssue::new(
                CompileIssueCode::MissingSourceOutput,
                Some(binding.source_task_id().clone()),
                Vec::new(),
                Some(binding.output_name().to_string()),
            ));
        }
        if input.is_none() {
            issues.push(ValidationIssue::new(
                CompileIssueCode::MissingTargetInput,
                Some(binding.target_task_id().clone()),
                Vec::new(),
                Some(binding.input_name().to_string()),
            ));
        }
        let slot = (
            binding.target_task_id().clone(),
            binding.input_name().clone(),
        );
        if !target_slots.insert(slot) {
            issues.push(ValidationIssue::new(
                CompileIssueCode::DuplicateInputBinding,
                Some(binding.target_task_id().clone()),
                Vec::new(),
                Some(binding.input_name().to_string()),
            ));
        }
        if binding.source_task_id() == binding.target_task_id() {
            issues.push(ValidationIssue::new(
                CompileIssueCode::SelfEdge,
                Some(binding.target_task_id().clone()),
                vec![binding.target_task_id().clone()],
                None,
            ));
        }
        if let (Some(output), Some(input)) = (output, input)
            && output.kind() != *input
        {
            issues.push(ValidationIssue::new(
                CompileIssueCode::ArtifactKindMismatch,
                Some(binding.target_task_id().clone()),
                vec![
                    binding.source_task_id().clone(),
                    binding.target_task_id().clone(),
                ],
                Some(binding.input_name().to_string()),
            ));
        }
        if binding.source_task_id() != binding.target_task_id() {
            add_edge(
                binding.source_task_id(),
                binding.target_task_id(),
                predecessors,
                successors,
            );
        }
    }
}

fn add_edge(
    source: &TaskId,
    target: &TaskId,
    predecessors: &mut BTreeMap<TaskId, BTreeSet<TaskId>>,
    successors: &mut BTreeMap<TaskId, BTreeSet<TaskId>>,
) {
    if let Some(values) = predecessors.get_mut(target) {
        values.insert(source.clone());
    }
    if let Some(values) = successors.get_mut(source) {
        values.insert(target.clone());
    }
}

fn validate_fan_out(
    policy: &WorkflowPolicy,
    successors: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    issues: &mut Vec<ValidationIssue>,
) {
    for (task_id, children) in successors {
        if children.len() > usize::from(policy.max_fan_out()) {
            issues.push(ValidationIssue::new(
                CompileIssueCode::FanOutExceeded,
                Some(task_id.clone()),
                children.iter().cloned().collect(),
                None,
            ));
        }
    }
}

fn topological_order(
    declaration_indices: &BTreeMap<TaskId, usize>,
    predecessors: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    successors: &BTreeMap<TaskId, BTreeSet<TaskId>>,
) -> (Vec<TaskId>, BTreeMap<TaskId, u32>) {
    let mut indegree: BTreeMap<TaskId, usize> = predecessors
        .iter()
        .map(|(task_id, values)| (task_id.clone(), values.len()))
        .collect();
    let mut ready = BTreeSet::new();
    let mut levels = BTreeMap::new();
    for (task_id, degree) in &indegree {
        if *degree == 0 {
            let index = declaration_indices
                .get(task_id)
                .copied()
                .unwrap_or(usize::MAX);
            ready.insert((index, task_id.clone()));
            levels.insert(task_id.clone(), 0_u32);
        }
    }

    let mut order = Vec::with_capacity(indegree.len());
    while let Some((_, current)) = ready.pop_first() {
        order.push(current.clone());
        let current_level = levels.get(&current).copied().unwrap_or_default();
        if let Some(children) = successors.get(&current) {
            let mut children: Vec<TaskId> = children.iter().cloned().collect();
            children.sort_by_key(|task_id| {
                (
                    declaration_indices
                        .get(task_id)
                        .copied()
                        .unwrap_or(usize::MAX),
                    task_id.clone(),
                )
            });
            for child in children {
                let child_level = levels.entry(child.clone()).or_default();
                *child_level = (*child_level).max(current_level.saturating_add(1));
                if let Some(degree) = indegree.get_mut(&child) {
                    *degree = degree.saturating_sub(1);
                    if *degree == 0 {
                        let index = declaration_indices
                            .get(&child)
                            .copied()
                            .unwrap_or(usize::MAX);
                        ready.insert((index, child));
                    }
                }
            }
        }
    }
    (order, levels)
}

fn find_cycle(
    declaration_indices: &BTreeMap<TaskId, usize>,
    successors: &BTreeMap<TaskId, BTreeSet<TaskId>>,
) -> Vec<TaskId> {
    let mut colors: BTreeMap<TaskId, u8> = successors
        .keys()
        .cloned()
        .map(|task_id| (task_id, 0))
        .collect();
    let mut stack = Vec::new();
    let mut roots: Vec<TaskId> = successors.keys().cloned().collect();
    roots.sort_by_key(|task_id| {
        (
            declaration_indices
                .get(task_id)
                .copied()
                .unwrap_or(usize::MAX),
            task_id.clone(),
        )
    });
    for root in roots {
        if colors.get(&root).copied().unwrap_or_default() == 0
            && let Some(cycle) = visit_cycle(
                &root,
                declaration_indices,
                successors,
                &mut colors,
                &mut stack,
            )
        {
            return cycle;
        }
    }
    Vec::new()
}

fn visit_cycle(
    current: &TaskId,
    declaration_indices: &BTreeMap<TaskId, usize>,
    successors: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    colors: &mut BTreeMap<TaskId, u8>,
    stack: &mut Vec<TaskId>,
) -> Option<Vec<TaskId>> {
    colors.insert(current.clone(), 1);
    stack.push(current.clone());
    let mut children: Vec<TaskId> = successors
        .get(current)
        .into_iter()
        .flat_map(|values| values.iter().cloned())
        .collect();
    children.sort_by_key(|task_id| {
        (
            declaration_indices
                .get(task_id)
                .copied()
                .unwrap_or(usize::MAX),
            task_id.clone(),
        )
    });
    for child in children {
        match colors.get(&child).copied().unwrap_or_default() {
            0 => {
                if let Some(cycle) =
                    visit_cycle(&child, declaration_indices, successors, colors, stack)
                {
                    return Some(cycle);
                }
            }
            1 => {
                if let Some(position) = stack.iter().position(|task_id| task_id == &child) {
                    let mut cycle = stack[position..].to_vec();
                    cycle.push(child);
                    return Some(cycle);
                }
            }
            _ => {}
        }
    }
    stack.pop();
    colors.insert(current.clone(), 2);
    None
}

fn build_ready_sets(tasks: &[CompiledTask]) -> Vec<ReadySet> {
    let mut levels: BTreeMap<u32, Vec<(u32, TaskId)>> = BTreeMap::new();
    for task in tasks {
        levels
            .entry(task.level)
            .or_default()
            .push((task.declaration_index, task.spec.id().clone()));
    }
    levels
        .into_iter()
        .map(|(level, mut values)| {
            values.sort();
            ReadySet {
                level,
                tasks: values.into_iter().map(|(_, task_id)| task_id).collect(),
            }
        })
        .collect()
}

fn plan_digest(
    definition: &WorkflowSpec,
    context: &CompileContext,
    tasks: &[CompiledTask],
    predecessors: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    successors: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    ready_sets: &[ReadySet],
) -> Result<Sha256Digest, CompileError> {
    let mut payload = CanonicalEncoder::default();
    payload.u16(definition.schema_version().major());
    payload.u16(definition.schema_version().minor());
    payload.string(definition.id().as_str());
    encode_policy(&mut payload, definition.policy());

    payload.length(tasks.len());
    for task in tasks {
        payload.string(task.spec.id().as_str());
        payload.bytes(digest_task(task)?.as_bytes());
    }

    let mut bindings = definition.bindings().to_vec();
    bindings.sort();
    payload.length(bindings.len());
    for binding in &bindings {
        payload.bytes(digest_binding(binding)?.as_bytes());
    }

    encode_edges(&mut payload, predecessors);
    encode_edges(&mut payload, successors);
    payload.length(ready_sets.len());
    for ready_set in ready_sets {
        payload.u32(ready_set.level);
        payload.length(ready_set.tasks.len());
        for task_id in &ready_set.tasks {
            payload.string(task_id.as_str());
        }
    }

    let mut hasher = CanonicalHasher::new("clef.execution-plan-v1")?;
    hasher.write_field("format", &PLAN_DIGEST_FORMAT.to_be_bytes())?;
    hasher.write_field("payload", &payload.0)?;
    hasher.write_field("profile_revision", context.profile_revision().as_bytes())?;
    Ok(hasher.finish())
}

fn digest_task(task: &CompiledTask) -> Result<Sha256Digest, CompileError> {
    let spec = &task.spec;
    let mut payload = CanonicalEncoder::default();
    payload.u16(spec.schema_version().major());
    payload.u16(spec.schema_version().minor());
    payload.string(spec.id().as_str());
    payload.string(spec.domain_function().as_str());
    payload.string(spec.instruction());
    payload.u32(task.declaration_index);
    payload.u32(task.level);
    payload.optional_u8(spec.effort().map(effort_tag));

    payload.length(spec.inputs().len());
    for (name, kind) in spec.inputs() {
        payload.string(name.as_str());
        payload.u8(kind.canonical_tag());
    }
    payload.length(spec.outputs().len());
    for output in spec.outputs().values() {
        payload.u16(output.schema_version().major());
        payload.u16(output.schema_version().minor());
        payload.string(output.name().as_str());
        payload.string(output.description());
        payload.u8(output.kind().canonical_tag());
        payload.string(output.path().as_str());
        payload.boolean(output.is_required());
    }
    payload.length(spec.dependencies().len());
    for dependency in spec.dependencies() {
        payload.string(dependency.as_str());
    }
    encode_capabilities(&mut payload, spec.required_capabilities());
    encode_capabilities(&mut payload, spec.preferred_capabilities());
    encode_capabilities(&mut payload, &task.missing_preferred_capabilities);
    payload.u16(spec.effects().schema_version().major());
    payload.u16(spec.effects().schema_version().minor());
    payload.length(spec.effects().allowed().len());
    for rule in spec.effects().allowed() {
        encode_effect(&mut payload, rule);
    }
    digest_blob("clef.compiled-task-v1", &payload.0).map_err(Into::into)
}

fn digest_binding(binding: &ArtifactBinding) -> Result<Sha256Digest, CompileError> {
    let mut payload = CanonicalEncoder::default();
    payload.string(binding.source_task_id().as_str());
    payload.string(binding.output_name().as_str());
    payload.string(binding.target_task_id().as_str());
    payload.string(binding.input_name().as_str());
    digest_blob("clef.artifact-binding-v1", &payload.0).map_err(Into::into)
}

fn digest_blob(domain: &str, payload: &[u8]) -> Result<Sha256Digest, DigestError> {
    let mut hasher = CanonicalHasher::new(domain)?;
    hasher.write_field("payload", payload)?;
    Ok(hasher.finish())
}

fn encode_policy(payload: &mut CanonicalEncoder, policy: &WorkflowPolicy) {
    payload.u16(policy.schema_version().major());
    payload.u16(policy.schema_version().minor());
    payload.u16(policy.max_concurrency());
    payload.u16(policy.max_fan_out());
    payload.boolean(policy.is_fail_fast());
}

fn encode_effect(payload: &mut CanonicalEncoder, rule: &EffectRule) {
    payload.u8(rule.kind().canonical_tag());
    match rule.path() {
        Some(path) => {
            payload.boolean(true);
            payload.string(path.as_str());
        }
        None => payload.boolean(false),
    }
}

fn encode_capabilities(payload: &mut CanonicalEncoder, values: &[CapabilityName]) {
    payload.length(values.len());
    for capability in values {
        payload.string(capability.as_str());
    }
}

fn encode_edges(payload: &mut CanonicalEncoder, edges: &BTreeMap<TaskId, BTreeSet<TaskId>>) {
    payload.length(edges.len());
    for (task_id, related) in edges {
        payload.string(task_id.as_str());
        payload.length(related.len());
        for related_id in related {
            payload.string(related_id.as_str());
        }
    }
}

const fn effort_tag(effort: Effort) -> u8 {
    match effort {
        Effort::Xhigh => 1,
        Effort::High => 2,
        Effort::Medium => 3,
        Effort::Low => 4,
    }
}

#[derive(Default)]
struct CanonicalEncoder(Vec<u8>);

impl CanonicalEncoder {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn boolean(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn optional_u8(&mut self, value: Option<u8>) {
        match value {
            Some(value) => {
                self.boolean(true);
                self.u8(value);
            }
            None => self.boolean(false),
        }
    }

    fn length(&mut self, value: usize) {
        self.0.extend_from_slice(&(value as u64).to_be_bytes());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.length(value.len());
        self.0.extend_from_slice(value);
    }

    fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }
}
