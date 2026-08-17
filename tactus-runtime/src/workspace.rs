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
        for path in [&self.config_path, &self.prompt_path] {
            if !path.is_file() {
                return Err(WorkspaceError::MissingPath(path.clone()));
            }
        }
        for path in [&self.scripts_path, &self.runs_path] {
            if !path.is_dir() {
                return Err(WorkspaceError::MissingPath(path.clone()));
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
        for (name, plugin) in &self.providers {
            validate_command("providers", name, &plugin.command)?;
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
            Ok(metadata) if metadata.file_type().is_symlink() => {
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
                // `symlink_metadata` does not classify Windows junctions as
                // symbolic links.  Canonical containment catches those (and
                // any other reparse-point directory) before `create_dir_all`
                // can materialize skill files outside this workspace.
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
    match workspace.load_config() {
        Ok(config) => {
            checks.push(DoctorCheck {
                name: "config".to_owned(),
                ok: true,
                detail: workspace.config_path.display().to_string(),
            });
            for (name, definition) in config.providers {
                push_plugin_check(&mut checks, "provider", &name, &definition.command);
            }
            for (name, definition) in config.effects {
                push_plugin_check(&mut checks, "effect", &name, &definition.command);
            }
            for (name, definition) in config.plugins {
                push_plugin_check(&mut checks, "plugin", &name, &definition.command);
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
        let found = find_executable(executable);
        checks.push(DoctorCheck {
            name: executable.to_owned(),
            ok: found.is_some(),
            detail: found.map_or_else(
                || format!("{executable} not found on PATH"),
                |path| path.display().to_string(),
            ),
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
) {
    let executable = &command[0];
    let found = if executable == "tactus"
        && command.get(1).is_some_and(|value| {
            matches!(value.as_str(), "provider-host" | "effect-host" | "dispatch")
        }) {
        env::current_exe().ok()
    } else {
        find_executable(executable)
    };
    checks.push(DoctorCheck {
        name: format!("{namespace}:{name}"),
        ok: found.is_some(),
        detail: found.map_or_else(
            || format!("{executable} not found"),
            |path| path.display().to_string(),
        ),
    });
}

fn find_executable(command: &str) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_path_buf());
    }
    let extensions: Vec<String> = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    let value = effective_path();
    (!value.is_empty())
        .then(|| {
            env::split_paths(&value).find_map(|directory| {
                extensions.iter().find_map(|extension| {
                    let candidate =
                        if cfg!(windows) && !command.to_ascii_uppercase().ends_with(extension) {
                            directory.join(format!("{command}{extension}"))
                        } else {
                            directory.join(command)
                        };
                    candidate.is_file().then_some(candidate)
                })
            })
        })
        .flatten()
}

/// Return the inherited PATH plus newly persisted Windows user/machine PATH
/// entries. Long-lived shells and coding agents otherwise cannot see tools
/// installed after they started (notably GHCup).
pub(crate) fn effective_path() -> std::ffi::OsString {
    #[cfg(not(windows))]
    {
        env::var_os("PATH").unwrap_or_default()
    }

    #[cfg(windows)]
    {
        use winreg::{
            RegKey,
            enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
        };

        let mut values = Vec::new();
        if let Some(current) = env::var_os("PATH") {
            values.push(current);
        }
        let locations = [
            (HKEY_CURRENT_USER, "Environment"),
            (
                HKEY_LOCAL_MACHINE,
                r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            ),
        ];
        for (root, key) in locations {
            let Ok(key) = RegKey::predef(root).open_subkey(key) else {
                continue;
            };
            if let Ok(path) = key.get_value::<String, _>("Path") {
                values.push(path.into());
            }
        }
        let entries = values.iter().flat_map(env::split_paths);
        env::join_paths(entries).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
    }
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
}
