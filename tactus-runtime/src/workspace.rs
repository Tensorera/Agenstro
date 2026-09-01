//! Typed `.tactus` workspace configuration and deterministic script discovery.

use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::{executable::ExecutableResolver, limits::RuntimeLimits};

/// Runtime configuration version shared with Clef.
pub const RUNTIME_API: &str = "clef.runtime/v1";
/// Project-local control directory.
pub const CONTROL_DIRECTORY: &str = ".tactus";
/// Runtime TOML filename.
pub const CONFIG_NAME: &str = "tactus.toml";
/// Haskell generation instructions filename.
pub const PROMPT_NAME: &str = "PROMPT.md";
/// Directory containing workflow entry points and helper modules.
pub const SCRIPTS_DIRECTORY: &str = "scripts";
/// Directory containing immutable per-run records.
pub const RUNS_DIRECTORY: &str = "runs";
/// Directory containing durable human-in-the-loop session documents.
pub const SESSIONS_DIRECTORY: &str = "sessions";
/// Directory containing runtime-owned agent guidance.
pub const SKILLS_DIRECTORY: &str = "skills";
/// Canonical Tactus skill directory name.
pub const TACTUS_SKILL_DIRECTORY: &str = "tactus";

const TACTUS_SKILL: &str = include_str!("../../skills/tactus/SKILL.md");
const TACTUS_SKILL_COMMANDS: &str = include_str!("../../skills/tactus/references/commands.md");
const TACTUS_SKILL_OUTCOMES: &str = include_str!("../../skills/tactus/references/outcomes.md");
const MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;

const DEFAULT_CONFIG: &str = r#"api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[limits]
max_concurrent_provider_calls = 4
check_timeout_seconds = 1800
script_timeout_seconds = 15300
plugin_timeout_seconds = 3600
provider_timeout_seconds = 13500
provider_outer_timeout_seconds = 14400
max_request_bytes = 1048576
max_frame_bytes = 33554432
max_stdout_bytes = 67108864
max_event_frames = 10000
max_stderr_bytes = 1048576
event_queue_bound = 4
native_max_line_bytes = 8388608
native_max_stdout_bytes = 1073741824
native_max_result_bytes = 4194304
native_max_stderr_bytes = 1048576
native_output_queue_bound = 8

[providers.codex]
command = ["tactus", "provider-host", "codex"]

[providers."claude-code"]
command = ["tactus", "provider-host", "claude-code"]

[providers.opencode]
command = ["tactus", "provider-host", "opencode"]

[effects."workspace.paths"]
command = ["tactus", "effect-host", "workspace-paths"]
observe_invocations = true

[plugins]
"#;

const DEFAULT_PROMPT: &str = r#"# Tactus workflow scripts

- Write Haskell workflow entry points below `.tactus/scripts/`.
- Name runnable entries `NNN_slug.hs` or `NNN_slug.lhs`, with increasing
  three-digit prefixes such as `010_atoms.hs`, `020_compose.hs`.
- Helpers may use any valid Haskell filename and nested directory.
- Each entry is an ordinary command-line Haskell program using `clef-sdk`.
- Route external work through configured providers, effects, or generic plugins.
- Keep each provider call atomic: one entry, one deliverable, or one checkpoint.
  Split independent work across bounded calls instead of one monolithic request.
- Write multi-MiB business artifacts into the workspace and return a relative
  path, digest, and short summary instead of returning the whole artifact in a
  terminal plugin value.
- Before inspecting or changing existing workflows, follow
  `.tactus/skills/tactus/SKILL.md`.
- During generation, only create or update DSL sources. Do not invoke Cabal, GHC,
  tests, or the generated workflows; Tactus owns those later phases.
"#;

/// One provider registry entry.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDefinition {
    /// Executable followed by arguments.
    pub command: Vec<String>,
    /// Optional model for provider-shaped plugins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional provider-specific effort level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Extension surface serialized into runtime JSON without interpretation.
    #[serde(default)]
    pub options: BTreeMap<String, JsonValue>,
}

/// One effect registry entry.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDefinition {
    /// Executable followed by arguments.
    pub command: Vec<String>,
    /// Extension surface serialized into runtime JSON without interpretation.
    #[serde(default)]
    pub options: BTreeMap<String, JsonValue>,
    /// Whether this effect observes other invocations.
    #[serde(default)]
    pub observe_invocations: bool,
}

/// One open generic plugin registry entry.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDefinition {
    /// Executable followed by arguments.
    pub command: Vec<String>,
    /// Extension surface serialized into runtime JSON without interpretation.
    #[serde(default)]
    pub options: BTreeMap<String, JsonValue>,
}

/// Validated project-local configuration.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Runtime document version.
    pub api: String,
    /// Provider selected when a command omits one.
    pub default_provider: String,
    /// UTF-8 prompt path, relative to the workspace unless absolute.
    pub instructions: PathBuf,
    /// Validated process, transport, and provider concurrency policy.
    #[serde(default)]
    pub limits: RuntimeLimits,
    /// Provider convenience registry.
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderDefinition>,
    /// Effect convenience registry.
    #[serde(default)]
    pub effects: BTreeMap<String, EffectDefinition>,
    /// Open registry for any plugin that does not fit a convenience category.
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginDefinition>,
}

/// Registry category used when resolving a generic plugin call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginNamespace {
    /// Resolve only when exactly one registry contains the name.
    Auto,
    /// Resolve from `[plugins]`.
    Plugin,
    /// Resolve from `[providers]`.
    Provider,
    /// Resolve from `[effects]`.
    Effect,
}

/// A category-preserving reference to one resolved registry entry.
#[derive(Clone, Copy, Debug)]
pub enum ResolvedPlugin<'a> {
    /// Generic plugin entry.
    Plugin(&'a PluginDefinition),
    /// Provider entry.
    Provider(&'a ProviderDefinition),
    /// Effect entry.
    Effect(&'a EffectDefinition),
}

impl<'a> ResolvedPlugin<'a> {
    /// Executable and arguments shared by all plugin categories.
    #[must_use]
    pub fn command(self) -> &'a [String] {
        match self {
            Self::Plugin(value) => &value.command,
            Self::Provider(value) => &value.command,
            Self::Effect(value) => &value.command,
        }
    }
}

/// An initialized workspace and all stable paths derived from it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    /// Project root containing `.tactus`.
    pub root: PathBuf,
    /// `.tactus` directory.
    pub control: PathBuf,
    /// TOML configuration.
    pub config_path: PathBuf,
    /// Generation instructions.
    pub prompt_path: PathBuf,
    /// Haskell scripts directory.
    pub scripts_path: PathBuf,
    /// Run journal directory.
    pub runs_path: PathBuf,
    /// Durable elicitation session directory.
    pub sessions_path: PathBuf,
    /// Cabal project file that points at Clef.
    pub cabal_project_path: PathBuf,
}

impl Workspace {
    /// Derive paths without touching the filesystem.
    #[must_use]
    pub fn at(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let control = root.join(CONTROL_DIRECTORY);
        Self {
            root,
            config_path: control.join(CONFIG_NAME),
            prompt_path: control.join(PROMPT_NAME),
            scripts_path: control.join(SCRIPTS_DIRECTORY),
            runs_path: control.join(RUNS_DIRECTORY),
            sessions_path: control.join(SESSIONS_DIRECTORY),
            cabal_project_path: control.join("cabal.project"),
            control,
        }
    }

    /// Open an initialized workspace at an exact root.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let root = dunce::canonicalize(root.as_ref()).map_err(WorkspaceError::Io)?;
        let workspace = Self::at(root);
        workspace.require_layout()?;
        Ok(workspace)
    }

    /// Search `start` and its ancestors for an initialized workspace.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let mut current = dunce::canonicalize(start.as_ref()).map_err(WorkspaceError::Io)?;
        if current.is_file() {
            current.pop();
        }
        loop {
            let candidate = Self::at(&current);
            if candidate.config_path.is_file() {
                candidate.require_layout()?;
                return Ok(candidate);
            }
            if !current.pop() {
                return Err(WorkspaceError::NotInitialized(start.as_ref().to_path_buf()));
            }
        }
    }

    /// Load and validate the typed TOML configuration.
    pub fn load_config(&self) -> Result<RuntimeConfig, WorkspaceError> {
        let content = fs::read_to_string(&self.config_path).map_err(WorkspaceError::Io)?;
        let raw: toml::Value = toml::from_str(&content).map_err(WorkspaceError::Toml)?;
        validate_json_domain(&raw, "config")?;
        let config: RuntimeConfig = raw.try_into().map_err(WorkspaceError::Toml)?;
        config.validate()?;
        Ok(config)
    }

    /// Resolve and read the UTF-8 generation prompt.
    pub fn read_prompt(&self, config: &RuntimeConfig) -> Result<String, WorkspaceError> {
        let path = if config.instructions.is_absolute() {
            config.instructions.clone()
        } else {
            self.root.join(&config.instructions)
        };
        fs::read_to_string(path).map_err(WorkspaceError::Io)
    }

    /// Read the project-local Tactus skill, falling back to the embedded
    /// version for workspaces initialized by an older runtime.
    pub fn read_tactus_skill(&self) -> Result<String, WorkspaceError> {
        let skill_root = self
            .control
            .join(SKILLS_DIRECTORY)
            .join(TACTUS_SKILL_DIRECTORY);
        let references = skill_root.join("references");
        let skill =
            read_contained_skill_file(&self.root, &self.control, &skill_root.join("SKILL.md"))?
                .unwrap_or_else(|| TACTUS_SKILL.to_owned());
        let commands =
            read_contained_skill_file(&self.root, &self.control, &references.join("commands.md"))?
                .unwrap_or_else(|| TACTUS_SKILL_COMMANDS.to_owned());
        let outcomes =
            read_contained_skill_file(&self.root, &self.control, &references.join("outcomes.md"))?
                .unwrap_or_else(|| TACTUS_SKILL_OUTCOMES.to_owned());
        Ok(format!(
            "{skill}\n\n# Bundled command reference\n\n{commands}\n\n# Bundled outcome reference\n\n{outcomes}"
        ))
    }

    /// Produce the language-neutral runtime JSON consumed by Clef.
    pub fn runtime_json(&self) -> Result<JsonValue, WorkspaceError> {
        self.runtime_json_with_dispatcher(Path::new("tactus"))
    }

    /// Produce runtime JSON whose commands target one exact Tactus executable.
    pub fn runtime_json_with_dispatcher(
        &self,
        dispatcher: &Path,
    ) -> Result<JsonValue, WorkspaceError> {
        let config = self.load_config()?;
        let instructions = self.read_prompt(&config)?;
        let providers = self.dispatch_registry(dispatcher, "provider", &config.providers)?;
        let effects = self.dispatch_registry(dispatcher, "effect", &config.effects)?;
        let plugins = self.dispatch_registry(dispatcher, "plugin", &config.plugins)?;
        Ok(serde_json::json!({
            "api": config.api,
            "workspace": self.root,
            "default_provider": config.default_provider,
            "instructions": instructions,
            "limits": config.limits,
            "providers": providers,
            "effects": effects,
            "plugins": plugins,
        }))
    }

    fn dispatch_registry<T>(
        &self,
        dispatcher: &Path,
        namespace: &str,
        definitions: &BTreeMap<String, T>,
    ) -> Result<BTreeMap<String, JsonValue>, WorkspaceError>
    where
        T: Serialize,
    {
        definitions
            .iter()
            .map(|(name, definition)| {
                let mut value = serde_json::to_value(definition)
                    .map_err(WorkspaceError::Json)?
                    .as_object()
                    .cloned()
                    .expect("PluginDefinition serializes as an object");
                value.insert(
                    "command".to_owned(),
                    serde_json::json!([
                        dispatcher.to_string_lossy(),
                        "dispatch",
                        "--namespace",
                        namespace,
                        "--name",
                        name,
                        "--root",
                        self.root.to_string_lossy(),
                    ]),
                );
                Ok((name.clone(), JsonValue::Object(value)))
            })
            .collect()
    }

    /// Find a plugin in one explicit namespace or unambiguously across all three.
    pub fn resolve_plugin<'a>(
        &self,
        config: &'a RuntimeConfig,
        name: &str,
        namespace: PluginNamespace,
    ) -> Result<ResolvedPlugin<'a>, WorkspaceError> {
        let selected = match namespace {
            PluginNamespace::Plugin => config.plugins.get(name).map(ResolvedPlugin::Plugin),
            PluginNamespace::Provider => config.providers.get(name).map(ResolvedPlugin::Provider),
            PluginNamespace::Effect => config.effects.get(name).map(ResolvedPlugin::Effect),
            PluginNamespace::Auto => {
                let matches = config.plugins.contains_key(name) as usize
                    + config.providers.contains_key(name) as usize
                    + config.effects.contains_key(name) as usize;
                if matches > 1 {
                    return Err(WorkspaceError::AmbiguousPlugin(name.to_owned()));
                }
                config
                    .plugins
                    .get(name)
                    .map(ResolvedPlugin::Plugin)
                    .or_else(|| config.providers.get(name).map(ResolvedPlugin::Provider))
                    .or_else(|| config.effects.get(name).map(ResolvedPlugin::Effect))
            }
        };
        selected.ok_or_else(|| WorkspaceError::UnknownPlugin(name.to_owned()))
    }

    fn require_layout(&self) -> Result<(), WorkspaceError> {
        let resolved_root = dunce::canonicalize(&self.root).map_err(WorkspaceError::Io)?;
        let control_metadata = fs::symlink_metadata(&self.control).map_err(WorkspaceError::Io)?;
        if !control_metadata.is_dir()
            || control_metadata.file_type().is_symlink()
            || directory_is_reparse_point(&control_metadata)
        {
            return Err(WorkspaceError::InvalidConfig(
                ".tactus must be a plain directory, not a symlink or reparse point".to_owned(),
            ));
        }
        let resolved_control = dunce::canonicalize(&self.control).map_err(WorkspaceError::Io)?;
        if resolved_control.parent() != Some(resolved_root.as_path()) {
            return Err(WorkspaceError::InvalidConfig(
                ".tactus must resolve directly below the workspace root".to_owned(),
            ));
        }
        for path in [&self.config_path, &self.prompt_path] {
            let metadata = fs::symlink_metadata(path).map_err(WorkspaceError::Io)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || directory_is_reparse_point(&metadata)
            {
                return Err(WorkspaceError::MissingPath(path.clone()));
            }
            let resolved = dunce::canonicalize(path).map_err(WorkspaceError::Io)?;
            if resolved.parent() != Some(resolved_control.as_path()) {
                return Err(WorkspaceError::InvalidConfig(
                    "workspace control file resolved outside .tactus".to_owned(),
                ));
            }
        }
        for path in [&self.scripts_path, &self.runs_path] {
            let metadata = fs::symlink_metadata(path).map_err(WorkspaceError::Io)?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || directory_is_reparse_point(&metadata)
            {
                return Err(WorkspaceError::MissingPath(path.clone()));
            }
            let resolved = dunce::canonicalize(path).map_err(WorkspaceError::Io)?;
            if resolved.parent() != Some(resolved_control.as_path()) {
                return Err(WorkspaceError::InvalidConfig(
                    "workspace runtime directory resolved outside .tactus".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_json_domain(value: &toml::Value, path: &str) -> Result<(), WorkspaceError> {
    match value {
        toml::Value::Float(number) if !number.is_finite() => Err(WorkspaceError::InvalidConfig(
            format!("{path} contains a non-finite number"),
        )),
        toml::Value::Datetime(_) => Err(WorkspaceError::InvalidConfig(format!(
            "{path} contains a TOML datetime, which is outside the plugin JSON domain"
        ))),
        toml::Value::Array(values) => {
            for (index, item) in values.iter().enumerate() {
                validate_json_domain(item, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        toml::Value::Table(values) => {
            for (key, item) in values {
                validate_json_domain(item, &format!("{path}.{key}"))?;
            }
            Ok(())
        }
        toml::Value::String(_)
        | toml::Value::Integer(_)
        | toml::Value::Float(_)
        | toml::Value::Boolean(_) => Ok(()),
    }
}

impl RuntimeConfig {
    /// Validate cross-field constraints after deserialization.
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.api != RUNTIME_API {
            return Err(WorkspaceError::InvalidConfig(format!(
                "api must be {RUNTIME_API:?}, received {:?}",
                self.api
            )));
        }
        if !self.providers.contains_key(&self.default_provider) {
            return Err(WorkspaceError::InvalidConfig(format!(
                "default_provider {:?} is not registered",
                self.default_provider
            )));
        }
        self.limits
            .validate()
            .map_err(WorkspaceError::InvalidConfig)?;
        for (name, plugin) in &self.providers {
            validate_command("providers", name, &plugin.command)?;
            let options = plugin
                .options
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            self.limits
                .validate_provider_options(&options, self.limits.provider_timeout_seconds)
                .map_err(|error| {
                    WorkspaceError::InvalidConfig(format!("providers.{name}: {error}"))
                })?;
        }
        for (name, plugin) in &self.effects {
            validate_command("effects", name, &plugin.command)?;
        }
        for (name, plugin) in &self.plugins {
            validate_command("plugins", name, &plugin.command)?;
        }
        Ok(())
    }
}

fn validate_command(namespace: &str, name: &str, command: &[String]) -> Result<(), WorkspaceError> {
    if name.is_empty() || command.first().is_none_or(String::is_empty) {
        return Err(WorkspaceError::InvalidConfig(format!(
            "{namespace}.{name} must have a non-empty command"
        )));
    }
    Ok(())
}

/// Filesystem changes made by idempotent initialization.
#[derive(Clone, Debug, Serialize)]
pub struct InitReport {
    /// Initialized workspace.
    #[serde(skip)]
    pub workspace: Workspace,
    /// Clef SDK selected for Cabal, if discoverable.
    pub clef_sdk: PathBuf,
    /// Relative files created by this call.
    pub created: Vec<String>,
    /// Relative files preserved without modification.
    pub preserved: Vec<String>,
}

/// Initialize `.tactus` without overwriting project-owned files.
pub fn initialize_workspace(
    root: impl AsRef<Path>,
    supplied_sdk: Option<&Path>,
) -> Result<InitReport, WorkspaceError> {
    fs::create_dir_all(root.as_ref()).map_err(WorkspaceError::Io)?;
    let root = dunce::canonicalize(root.as_ref()).map_err(WorkspaceError::Io)?;
    let sdk = resolve_sdk(&root, supplied_sdk)?;
    let workspace = Workspace::at(&root);
    let skill_root = workspace
        .control
        .join(SKILLS_DIRECTORY)
        .join(TACTUS_SKILL_DIRECTORY);
    let skill_references = skill_root.join("references");
    reject_linked_skill_directories(&root, &skill_references)?;
    fs::create_dir_all(&skill_references).map_err(WorkspaceError::Io)?;
    require_contained_directory(&root, &workspace.control, &skill_references)?;
    fs::create_dir_all(&workspace.scripts_path).map_err(WorkspaceError::Io)?;
    fs::create_dir_all(&workspace.runs_path).map_err(WorkspaceError::Io)?;
    reject_linked_skill_directories(&root, &workspace.sessions_path)?;
    fs::create_dir_all(&workspace.sessions_path).map_err(WorkspaceError::Io)?;
    require_contained_directory(&root, &workspace.control, &workspace.sessions_path)?;

    let mut files = vec![
        (workspace.config_path.clone(), DEFAULT_CONFIG.to_owned()),
        (workspace.prompt_path.clone(), DEFAULT_PROMPT.to_owned()),
        (skill_root.join("SKILL.md"), TACTUS_SKILL.to_owned()),
        (
            skill_references.join("commands.md"),
            TACTUS_SKILL_COMMANDS.to_owned(),
        ),
        (
            skill_references.join("outcomes.md"),
            TACTUS_SKILL_OUTCOMES.to_owned(),
        ),
    ];
    let portable = sdk.to_string_lossy().replace('\\', "/");
    let quoted = serde_json::to_string(&portable).expect("a path string is valid JSON");
    files.push((
        workspace.cabal_project_path.clone(),
        format!("packages:\n  {quoted}\n"),
    ));
    let mut created = Vec::new();
    let mut preserved = Vec::new();
    for (path, content) in files {
        let relative = relative_display(&root, &path);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write as _;
                file.write_all(content.as_bytes())
                    .map_err(WorkspaceError::Io)?;
                created.push(relative);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                preserved.push(relative);
            }
            Err(error) => return Err(WorkspaceError::Io(error)),
        }
    }
    workspace.require_layout()?;
    Ok(InitReport {
        workspace,
        clef_sdk: sdk,
        created,
        preserved,
    })
}

fn reject_linked_skill_directories(root: &Path, leaf: &Path) -> Result<(), WorkspaceError> {
    let resolved_root = dunce::canonicalize(root).map_err(WorkspaceError::Io)?;
    let relative = leaf.strip_prefix(root).map_err(|_| {
        WorkspaceError::InvalidConfig("Tactus skill path escaped the workspace".to_owned())
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink() || directory_is_reparse_point(&metadata) =>
            {
                return Err(WorkspaceError::InvalidConfig(format!(
                    "Tactus skill directory must not be a link: {}",
                    current.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(WorkspaceError::InvalidConfig(format!(
                    "Tactus skill directory is not a directory: {}",
                    current.display()
                )));
            }
            Ok(_) => {
                let resolved_current = dunce::canonicalize(&current).map_err(WorkspaceError::Io)?;
                if !resolved_current.starts_with(&resolved_root) {
                    return Err(WorkspaceError::InvalidConfig(format!(
                        "Tactus skill directory resolved outside the workspace: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(WorkspaceError::Io(error)),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn directory_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn directory_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn require_contained_directory(
    root: &Path,
    control: &Path,
    directory: &Path,
) -> Result<(), WorkspaceError> {
    let resolved_root = dunce::canonicalize(root).map_err(WorkspaceError::Io)?;
    let resolved_control = dunce::canonicalize(control).map_err(WorkspaceError::Io)?;
    let resolved_directory = dunce::canonicalize(directory).map_err(WorkspaceError::Io)?;
    if !resolved_control.starts_with(&resolved_root)
        || !resolved_directory.starts_with(&resolved_control)
    {
        return Err(WorkspaceError::InvalidConfig(
            "Tactus skill directory resolved outside the workspace".to_owned(),
        ));
    }
    Ok(())
}

fn read_contained_skill_file(
    root: &Path,
    control: &Path,
    path: &Path,
) -> Result<Option<String>, WorkspaceError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkspaceError::Io(error)),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let resolved_root = dunce::canonicalize(root).map_err(WorkspaceError::Io)?;
    let resolved_control = dunce::canonicalize(control).map_err(WorkspaceError::Io)?;
    let resolved_path = dunce::canonicalize(path).map_err(WorkspaceError::Io)?;
    if !resolved_control.starts_with(&resolved_root)
        || !resolved_path.starts_with(&resolved_control)
    {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(resolved_path)
        .map_err(WorkspaceError::Io)?
        .take(MAX_SKILL_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(WorkspaceError::Io)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SKILL_FILE_BYTES {
        return Err(WorkspaceError::InvalidConfig(format!(
            "Tactus skill file exceeds {MAX_SKILL_FILE_BYTES} bytes: {}",
            path.display()
        )));
    }
    String::from_utf8(bytes).map(Some).map_err(|_| {
        WorkspaceError::InvalidConfig(format!("Tactus skill is not UTF-8: {}", path.display()))
    })
}

fn resolve_sdk(root: &Path, supplied: Option<&Path>) -> Result<PathBuf, WorkspaceError> {
    if let Some(value) = supplied {
        let candidate = if value.is_absolute() {
            value.to_path_buf()
        } else {
            root.join(value)
        };
        let resolved = dunce::canonicalize(candidate).map_err(WorkspaceError::Io)?;
        if !resolved.join("clef-sdk.cabal").is_file() {
            return Err(WorkspaceError::InvalidSdk(resolved));
        }
        return Ok(resolved);
    }
    let mut candidates = Vec::new();
    if let Some(value) = env::var_os("TACTUS_CLEF_SDK") {
        candidates.push(PathBuf::from(value));
    }
    candidates.push(root.join("clef-sdk"));
    if let Some(parent) = root.parent() {
        candidates.push(parent.join("clef-sdk"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../clef-sdk"));
    for candidate in candidates {
        if candidate.join("clef-sdk.cabal").is_file() {
            return dunce::canonicalize(candidate).map_err(WorkspaceError::Io);
        }
    }
    Err(WorkspaceError::SdkNotFound)
}

/// One deterministically discovered Haskell source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScriptInfo {
    /// Absolute source path.
    pub path: PathBuf,
    /// Forward-slash path relative to the workspace.
    pub relative_path: String,
    /// Three-digit execution order for entries; helpers have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u16>,
    /// Whether this source is an ordered entry point.
    pub runnable: bool,
}

/// Recursively discover `.hs` and `.lhs` files without following symlink dirs.
pub fn discover_scripts(workspace: &Workspace) -> Result<Vec<ScriptInfo>, WorkspaceError> {
    let mut paths = Vec::new();
    collect_haskell(&workspace.scripts_path, &mut paths)?;
    let mut scripts = paths
        .into_iter()
        .map(|path| {
            let order = path
                .file_name()
                .and_then(|value| value.to_str())
                .and_then(entry_order);
            ScriptInfo {
                relative_path: relative_display(&workspace.root, &path),
                path,
                order,
                runnable: order.is_some(),
            }
        })
        .collect::<Vec<_>>();
    scripts.sort_by(|left, right| match (left.order, right.order) {
        (Some(left_order), Some(right_order)) => left_order
            .cmp(&right_order)
            .then_with(|| stable_path_cmp(&left.relative_path, &right.relative_path)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => stable_path_cmp(&left.relative_path, &right.relative_path),
    });
    Ok(scripts)
}

fn collect_haskell(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), WorkspaceError> {
    for entry in fs::read_dir(directory).map_err(WorkspaceError::Io)? {
        let entry = entry.map_err(WorkspaceError::Io)?;
        let file_type = entry.file_type().map_err(WorkspaceError::Io)?;
        if file_type.is_dir() {
            collect_haskell(&entry.path(), output)?;
        } else if file_type.is_file()
            && entry
                .path()
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|suffix| {
                    suffix.eq_ignore_ascii_case("hs") || suffix.eq_ignore_ascii_case("lhs")
                })
        {
            output.push(dunce::canonicalize(entry.path()).map_err(WorkspaceError::Io)?);
        }
    }
    Ok(())
}

fn entry_order(name: &str) -> Option<u16> {
    let (stem, suffix) = name.rsplit_once('.')?;
    if !suffix.eq_ignore_ascii_case("hs") && !suffix.eq_ignore_ascii_case("lhs") {
        return None;
    }
    let (prefix, slug) = stem.split_once('_')?;
    if prefix.len() != 3
        || !prefix.bytes().all(|byte| byte.is_ascii_digit())
        || slug.is_empty()
        || slug.starts_with('_')
        || slug.ends_with('_')
        || slug.contains("__")
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    prefix.parse().ok()
}

fn stable_path_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// One factual diagnostic check.
#[derive(Clone, Debug, Serialize)]
pub struct DoctorCheck {
    /// Stable check name.
    pub name: String,
    /// Whether the requirement is satisfied.
    pub ok: bool,
    /// Human-readable evidence.
    pub detail: String,
}

/// Diagnose workspace structure, config, SDK linkage, and required tools.
pub fn doctor(workspace: &Workspace) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let resolver = ExecutableResolver::environment(&workspace.root);
    match workspace.load_config() {
        Ok(config) => {
            checks.push(DoctorCheck {
                name: "config".to_owned(),
                ok: true,
                detail: workspace.config_path.display().to_string(),
            });
            for (name, definition) in config.providers {
                push_plugin_check(
                    &mut checks,
                    "provider",
                    &name,
                    &definition.command,
                    &resolver,
                );
                push_native_provider_check(&mut checks, &name, &definition, &resolver);
            }
            for (name, definition) in config.effects {
                push_plugin_check(&mut checks, "effect", &name, &definition.command, &resolver);
            }
            for (name, definition) in config.plugins {
                push_plugin_check(&mut checks, "plugin", &name, &definition.command, &resolver);
            }
        }
        Err(error) => checks.push(DoctorCheck {
            name: "config".to_owned(),
            ok: false,
            detail: error.to_string(),
        }),
    }
    checks.push(clef_sdk_link_check(workspace));
    for executable in ["ghc", "cabal"] {
        let found = resolver.resolve(executable);
        checks.push(DoctorCheck {
            name: executable.to_owned(),
            ok: found.is_ok(),
            detail: found.map_or_else(|error| error.to_string(), |path| path.display().to_string()),
        });
    }
    checks
}

fn clef_sdk_link_check(workspace: &Workspace) -> DoctorCheck {
    let failure = |detail: String| DoctorCheck {
        name: "clef-sdk-link".to_owned(),
        ok: false,
        detail,
    };
    let project = match fs::read_to_string(&workspace.cabal_project_path) {
        Ok(project) => project,
        Err(error) => {
            return failure(format!(
                "cannot read {}: {error}",
                workspace.cabal_project_path.display()
            ));
        }
    };
    let mut lines = project
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    if lines.next() != Some("packages:") {
        return failure("expected init-generated `packages:` Cabal linkage".to_owned());
    }
    let Some(encoded_path) = lines.next() else {
        return failure("Cabal linkage contains no Clef SDK path".to_owned());
    };
    let package: String = match serde_json::from_str(encoded_path) {
        Ok(package) => package,
        Err(error) => return failure(format!("invalid Clef SDK path in cabal.project: {error}")),
    };
    let candidate = PathBuf::from(package);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        workspace.control.join(candidate)
    };
    let resolved = match dunce::canonicalize(&candidate) {
        Ok(resolved) => resolved,
        Err(error) => {
            return failure(format!(
                "linked Clef SDK path {} is unavailable: {error}",
                candidate.display()
            ));
        }
    };
    let manifest = resolved.join("clef-sdk.cabal");
    DoctorCheck {
        name: "clef-sdk-link".to_owned(),
        ok: manifest.is_file(),
        detail: if manifest.is_file() {
            manifest.display().to_string()
        } else {
            format!("linked SDK has no clef-sdk.cabal: {}", resolved.display())
        },
    }
}

fn push_plugin_check(
    checks: &mut Vec<DoctorCheck>,
    namespace: &str,
    name: &str,
    command: &[String],
    resolver: &ExecutableResolver,
) {
    let executable = &command[0];
    let found = if executable == "tactus"
        && command.get(1).is_some_and(|value| {
            matches!(value.as_str(), "provider-host" | "effect-host" | "dispatch")
        }) {
        env::current_exe().map_err(|error| format!("cannot resolve the running tactus: {error}"))
    } else {
        resolver
            .resolve(executable)
            .map_err(|error| error.to_string())
    };
    checks.push(DoctorCheck {
        name: format!("{namespace}:{name}"),
        ok: found.is_ok(),
        detail: found.map_or_else(|error| error, |path| path.display().to_string()),
    });
}

fn push_native_provider_check(
    checks: &mut Vec<DoctorCheck>,
    name: &str,
    definition: &ProviderDefinition,
    resolver: &ExecutableResolver,
) {
    let Some(default_executable) = builtin_native_executable(&definition.command) else {
        return;
    };
    let configured = match definition.options.get("executable") {
        None => default_executable,
        Some(JsonValue::String(value)) if !value.is_empty() => value,
        Some(_) => {
            checks.push(DoctorCheck {
                name: format!("provider-native:{name}"),
                ok: false,
                detail: "options.executable must be a non-empty string".to_owned(),
            });
            return;
        }
    };
    if let Some(prefix) = provider_command_prefix(&definition.options) {
        let result = resolver.resolve(prefix);
        checks.push(DoctorCheck {
            name: format!("provider-native:{name}"),
            ok: result.is_ok(),
            detail: result.map_or_else(
                |error| {
                    format!("native command {configured:?} uses an unavailable wrapper: {error}")
                },
                |path| {
                    format!(
                        "native command {configured:?} is delegated through {}",
                        path.display()
                    )
                },
            ),
        });
        return;
    }
    let result = resolver.resolve(configured);
    checks.push(DoctorCheck {
        name: format!("provider-native:{name}"),
        ok: result.is_ok(),
        detail: result.map_or_else(|error| error.to_string(), |path| path.display().to_string()),
    });
}

fn builtin_native_executable(command: &[String]) -> Option<&'static str> {
    let dispatcher = command.first()?;
    let file_name = Path::new(dispatcher)
        .file_name()
        .and_then(|value| value.to_str())?;
    if !(file_name.eq_ignore_ascii_case("tactus") || file_name.eq_ignore_ascii_case("tactus.exe"))
        || command.get(1).map(String::as_str) != Some("provider-host")
    {
        return None;
    }
    match command.get(2).map(String::as_str) {
        Some("codex") => Some("codex"),
        Some("claude" | "claude-code") => Some("claude"),
        Some("opencode") => Some("opencode"),
        _ => None,
    }
}

fn provider_command_prefix(options: &BTreeMap<String, JsonValue>) -> Option<&str> {
    options
        .get("command_prefix")?
        .as_array()?
        .first()?
        .as_str()
        .filter(|value| !value.is_empty())
}

/// Workspace/configuration failure.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// Filesystem operation failed.
    #[error("workspace I/O failed: {0}")]
    Io(#[source] io::Error),
    /// No `.tactus` directory was found.
    #[error("no initialized .tactus workspace found from {0}")]
    NotInitialized(PathBuf),
    /// Required initialized path is absent.
    #[error("workspace is missing {0}")]
    MissingPath(PathBuf),
    /// TOML syntax or shape was invalid.
    #[error("invalid Tactus TOML: {0}")]
    Toml(#[source] toml::de::Error),
    /// A validated config value could not be normalized into runtime JSON.
    #[error("cannot encode runtime JSON: {0}")]
    Json(#[source] serde_json::Error),
    /// Cross-field config validation failed.
    #[error("invalid Tactus config: {0}")]
    InvalidConfig(String),
    /// Explicit SDK path was not a Clef package.
    #[error("Clef SDK path does not contain clef-sdk.cabal: {0}")]
    InvalidSdk(PathBuf),
    /// No SDK was discoverable in an installed or relocated environment.
    #[error("cannot locate clef-sdk; pass --sdk PATH or set TACTUS_CLEF_SDK")]
    SdkNotFound,
    /// Plugin name did not exist in the selected registry.
    #[error("unknown plugin {0:?}")]
    UnknownPlugin(String),
    /// Auto resolution would silently choose between multiple registries.
    #[error("plugin {0:?} exists in multiple registries; select a namespace")]
    AmbiguousPlugin(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_open_plugin_registry() {
        let config: RuntimeConfig = toml::from_str(DEFAULT_CONFIG).expect("default config");
        config.validate().expect("valid config");
        assert!(config.plugins.is_empty());
        assert_eq!(config.providers.len(), 3);
    }

    #[test]
    fn doctor_resolves_the_configured_native_provider_from_the_workspace() {
        let temporary = tempfile::tempdir().expect("temporary workspace");
        let tools = temporary.path().join("中文 provider tools");
        fs::create_dir_all(&tools).expect("tools directory");
        #[cfg(windows)]
        let executable = tools.join("claude.cmd");
        #[cfg(not(windows))]
        let executable = tools.join("claude");
        fs::write(&executable, b"fixture").expect("native provider fixture");
        let relative = executable
            .strip_prefix(temporary.path())
            .expect("relative executable")
            .to_string_lossy()
            .into_owned();
        let definition = ProviderDefinition {
            command: vec![
                "tactus".to_owned(),
                "provider-host".to_owned(),
                "claude-code".to_owned(),
            ],
            model: None,
            effort: None,
            options: BTreeMap::from([("executable".to_owned(), JsonValue::String(relative))]),
        };
        let resolver = ExecutableResolver::new(std::ffi::OsString::new(), None, temporary.path());
        let mut checks = Vec::new();

        push_native_provider_check(&mut checks, "reviewer", &definition, &resolver);

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "provider-native:reviewer");
        assert!(checks[0].ok, "{}", checks[0].detail);
        assert_eq!(
            PathBuf::from(&checks[0].detail),
            dunce::canonicalize(executable).expect("canonical executable")
        );
    }

    #[test]
    fn doctor_reports_ambiguous_native_provider_candidates() {
        let temporary = tempfile::tempdir().expect("temporary workspace");
        let first = temporary.path().join("first");
        let second = temporary.path().join("second");
        fs::create_dir_all(&first).expect("first tools directory");
        fs::create_dir_all(&second).expect("second tools directory");
        #[cfg(windows)]
        let executable = "claude.exe";
        #[cfg(not(windows))]
        let executable = "claude";
        fs::write(first.join(executable), b"first").expect("first provider");
        fs::write(second.join(executable), b"second").expect("second provider");
        let definition = ProviderDefinition {
            command: vec![
                "tactus".to_owned(),
                "provider-host".to_owned(),
                "claude-code".to_owned(),
            ],
            model: None,
            effort: None,
            options: BTreeMap::new(),
        };
        let resolver = ExecutableResolver::new(
            env::join_paths([&first, &second]).expect("test PATH"),
            Some(std::ffi::OsString::from(".EXE")),
            temporary.path(),
        );
        let mut checks = Vec::new();

        push_native_provider_check(&mut checks, "reviewer", &definition, &resolver);

        assert_eq!(checks.len(), 1);
        assert!(!checks[0].ok);
        assert!(
            checks[0].detail.contains("ambiguous"),
            "{}",
            checks[0].detail
        );
        let first = dunce::canonicalize(first).expect("canonical first directory");
        let second = dunce::canonicalize(second).expect("canonical second directory");
        assert!(
            checks[0].detail.contains(&first.display().to_string()),
            "{}",
            checks[0].detail
        );
        assert!(
            checks[0].detail.contains(&second.display().to_string()),
            "{}",
            checks[0].detail
        );
    }

    #[test]
    fn category_specific_fields_are_rejected() {
        let invalid = DEFAULT_CONFIG.replace(
            "observe_invocations = true",
            "observe_invocations = true\nmodel = \"not-an-effect-field\"",
        );
        assert!(toml::from_str::<RuntimeConfig>(&invalid).is_err());
    }

    #[test]
    fn non_finite_plugin_options_are_rejected() {
        let invalid = DEFAULT_CONFIG.replace(
            "command = [\"tactus\", \"provider-host\", \"codex\"]",
            "command = [\"tactus\", \"provider-host\", \"codex\"]\noptions = { temperature = nan }",
        );
        let raw: toml::Value = toml::from_str(&invalid).expect("TOML syntax");
        assert!(validate_json_domain(&raw, "config").is_err());
    }

    #[test]
    fn entry_names_are_intentionally_narrow() {
        assert_eq!(entry_order("010_atoms.hs"), Some(10));
        assert_eq!(entry_order("999_a1_b2.lhs"), Some(999));
        assert_eq!(entry_order("10_atoms.hs"), None);
        assert_eq!(entry_order("010_Atoms.hs"), None);
        assert_eq!(entry_order("010_a__b.hs"), None);
    }

    #[test]
    fn doctor_validates_the_linked_clef_manifest() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let sdk = temporary.path().join("sdk");
        fs::create_dir(&sdk).expect("sdk directory");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("sdk manifest");
        let report =
            initialize_workspace(temporary.path().join("project"), Some(&sdk)).expect("initialize");
        let linkage = doctor(&report.workspace)
            .into_iter()
            .find(|check| check.name == "clef-sdk-link")
            .expect("link check");
        assert!(linkage.ok, "{}", linkage.detail);

        fs::write(
            &report.workspace.cabal_project_path,
            "packages:\n  \"missing-sdk\"\n",
        )
        .expect("broken linkage");
        let linkage = doctor(&report.workspace)
            .into_iter()
            .find(|check| check.name == "clef-sdk-link")
            .expect("link check");
        assert!(!linkage.ok);
    }

    #[test]
    fn older_workspace_receives_complete_embedded_skill_guidance() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("project");
        fs::create_dir_all(root.join(CONTROL_DIRECTORY)).expect("control directory");
        let workspace = Workspace::at(&root);

        let guidance = workspace.read_tactus_skill().expect("embedded skill");
        assert!(guidance.contains("Never blindly retry `OutcomeUnknown`"));
        assert!(guidance.contains("# Bundled command reference"));
        assert!(guidance.contains("# Bundled outcome reference"));
    }

    fn write_minimal_control_layout(control: &Path) {
        fs::create_dir_all(control.join(SCRIPTS_DIRECTORY)).expect("scripts directory");
        fs::create_dir_all(control.join(RUNS_DIRECTORY)).expect("runs directory");
        fs::write(control.join(CONFIG_NAME), DEFAULT_CONFIG).expect("config");
        fs::write(control.join(PROMPT_NAME), DEFAULT_PROMPT).expect("prompt");
    }

    #[cfg(unix)]
    #[test]
    fn discovery_rejects_an_entire_linked_control_directory() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("project");
        let outside_control = temporary.path().join("outside-control");
        fs::create_dir(&root).expect("project directory");
        write_minimal_control_layout(&outside_control);
        symlink(&outside_control, root.join(CONTROL_DIRECTORY)).expect("control symlink");

        let error = Workspace::discover(&root).expect_err("linked control rejected");
        assert!(error.to_string().contains("plain directory"));
    }

    #[cfg(windows)]
    #[test]
    fn discovery_rejects_an_entire_control_junction() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("project");
        let outside_control = temporary.path().join("outside-control");
        fs::create_dir(&root).expect("project directory");
        write_minimal_control_layout(&outside_control);
        let control = root.join(CONTROL_DIRECTORY);
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:TACTUS_TEST_JUNCTION -Target $env:TACTUS_TEST_TARGET | Out-Null",
            ])
            .env("TACTUS_TEST_JUNCTION", &control)
            .env("TACTUS_TEST_TARGET", &outside_control)
            .output()
            .expect("create control junction");
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let error = Workspace::discover(&root).expect_err("control junction rejected");
        assert!(error.to_string().contains("plain directory"));
    }

    #[cfg(unix)]
    #[test]
    fn init_rejects_linked_skill_directories_before_writing_outside() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("project");
        let sdk = temporary.path().join("sdk");
        let outside = temporary.path().join("outside");
        fs::create_dir_all(root.join(CONTROL_DIRECTORY)).expect("control directory");
        fs::create_dir_all(&sdk).expect("sdk directory");
        fs::create_dir_all(&outside).expect("outside directory");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("sdk manifest");
        symlink(
            &outside,
            root.join(CONTROL_DIRECTORY).join(SKILLS_DIRECTORY),
        )
        .expect("skill symlink");

        let error = initialize_workspace(&root, Some(&sdk)).expect_err("linked skills rejected");
        assert!(error.to_string().contains("must not be a link"));
        assert!(!outside.join(TACTUS_SKILL_DIRECTORY).exists());
    }

    #[cfg(windows)]
    #[test]
    fn init_rejects_a_sessions_junction_to_a_sibling_directory() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join("project");
        let control = root.join(CONTROL_DIRECTORY);
        let runs = control.join(RUNS_DIRECTORY);
        let sdk = temporary.path().join("sdk");
        fs::create_dir_all(&runs).expect("runs directory");
        fs::create_dir_all(&sdk).expect("sdk directory");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("sdk manifest");

        let sessions = control.join(SESSIONS_DIRECTORY);
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:TACTUS_TEST_JUNCTION -Target $env:TACTUS_TEST_TARGET | Out-Null",
            ])
            .env("TACTUS_TEST_JUNCTION", &sessions)
            .env("TACTUS_TEST_TARGET", &runs)
            .output()
            .expect("create junction");
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let error =
            initialize_workspace(&root, Some(&sdk)).expect_err("sessions junction rejected");
        assert!(error.to_string().contains("must not be a link"));
        assert!(!control.join(CONFIG_NAME).exists());
    }
}
