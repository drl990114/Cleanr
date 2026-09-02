use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use cleanr_config::Config;
use cleanr_plugin_api::{
    INSTALLED_PLUGIN_METADATA_FILE, InstalledPlugin, PLUGIN_INDEX_SCHEMA_VERSION, PluginCapability,
    PluginIndex, PluginIndexEntry, PluginIndexFile, PluginIndexSource, PluginManifest,
    discover_bundles, sorted_dir_entries, validate_plugin_id,
};
use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};

const MAX_PLUGIN_INDEX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PLUGIN_FILE_BYTES: u64 = 4 * 1024 * 1024;

pub struct InitOptions {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub force: bool,
}

pub struct InstallOptions {
    pub id: String,
    pub index_url: Option<String>,
    pub github_repo: String,
    pub github_ref: String,
    pub plugin_dir: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub trust: bool,
    pub enable: bool,
    pub force: bool,
}

pub struct IndexOptions {
    pub plugin_dir: PathBuf,
    pub output: Option<PathBuf>,
    pub base_url: String,
    pub check: bool,
}

pub struct LinkOptions {
    pub path: PathBuf,
    pub config_path: Option<PathBuf>,
    pub trust: bool,
    pub enable: bool,
}

pub struct UnlinkOptions {
    pub id: String,
    pub config_path: Option<PathBuf>,
}

pub struct RemoveOptions {
    pub id: String,
    pub config_path: Option<PathBuf>,
}

pub struct UpdateOptions {
    pub id: Option<String>,
    pub config_path: Option<PathBuf>,
    pub force: bool,
}

pub struct SearchOptions {
    pub query: Option<String>,
    pub index_url: Option<String>,
    pub github_repo: String,
    pub github_ref: String,
}

pub struct InfoOptions {
    pub id: String,
    pub index_url: Option<String>,
    pub github_repo: String,
    pub github_ref: String,
    pub config_path: Option<PathBuf>,
    pub local: bool,
}

pub struct TrustOptions {
    pub id: String,
    pub config_path: Option<PathBuf>,
}

pub fn init(options: InitOptions) -> Result<()> {
    validate_plugin_id(&options.id)?;
    if options.name.trim().is_empty() {
        bail!("plugin name cannot be empty");
    }
    if options.path.exists() && !options.force && !is_empty_dir(&options.path)? {
        bail!(
            "{} already exists and is not empty; pass --force to write template files",
            options.path.display()
        );
    }
    fs::create_dir_all(options.path.join("rules"))
        .with_context(|| format!("failed to create {}", options.path.join("rules").display()))?;
    write_template_file(
        &options.path.join("plugin.toml"),
        &plugin_template(&options.id, &options.name),
        options.force,
    )?;
    write_template_file(
        &options.path.join("rules").join("default.toml"),
        &rule_pack_template(&options.id, &options.name),
        options.force,
    )?;
    validate_bundle(&options.path)?;
    println!(
        "Created plugin {} at {}",
        options.id,
        options.path.display()
    );
    Ok(())
}

pub fn validate(paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        bail!("provide at least one plugin bundle, rule TOML, or language YAML path");
    }
    for path in paths {
        validate_path(path)?;
        println!("valid: {}", path.display());
    }
    Ok(())
}

pub fn print_schema(kind: &str) -> Result<()> {
    let schema = match kind {
        "manifest" => cleanr_plugin_api::plugin_manifest_schema(),
        "index" => cleanr_plugin_api::plugin_index_schema(),
        "rules" => cleanr_rules::rule_pack_schema(),
        "locations" => cleanr_rules::scan_location_pack_schema(),
        "language" | "translations" => cleanr_i18n::language_pack_schema(),
        "config" => cleanr_config::config_schema(),
        _ => bail!("unknown schema kind: {kind}"),
    };
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}

pub fn generate_index(options: IndexOptions) -> Result<()> {
    if !options.base_url.starts_with("http://") && !options.base_url.starts_with("https://") {
        bail!("--base-url must be an HTTP(S) URL");
    }
    let plugin_dir = options.plugin_dir;
    let output = options
        .output
        .unwrap_or_else(|| plugin_dir.join("index.json"));
    let base_url = options.base_url.trim_end_matches('/').to_string();
    let mut plugins = Vec::new();

    for bundle_dir in sorted_dir_entries(&plugin_dir)? {
        if !bundle_dir.is_dir() || !bundle_dir.join("plugin.toml").is_file() {
            continue;
        }
        validate_bundle(&bundle_dir)?;
        let manifest = read_manifest(&bundle_dir)?;
        let bundle_name = bundle_dir
            .file_name()
            .and_then(|name| name.to_str())
            .context("plugin directory name is not valid UTF-8")?;
        let files = publishable_files(&bundle_dir)?
            .into_iter()
            .map(|path| plugin_index_file(&base_url, bundle_name, &bundle_dir, &path))
            .collect::<Result<Vec<_>>>()?;
        let entry = PluginIndexEntry {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            description: manifest.description,
            cleanr_version: manifest.cleanr_version,
            capabilities: manifest.capabilities,
            categories: manifest.categories,
            keywords: manifest.keywords,
            author: None,
            homepage: manifest.homepage,
            repository: manifest.repository.clone(),
            license: manifest.license,
            source: manifest.repository.map(|repository| PluginIndexSource {
                kind: "github".to_string(),
                repo: Some(repository),
                path: Some(format!("plugins/{bundle_name}")),
                url: None,
            }),
            files,
        };
        validate_index_entry(&entry)?;
        plugins.push(entry);
    }

    let index = PluginIndex {
        schema_version: PLUGIN_INDEX_SCHEMA_VERSION,
        plugins,
    };
    let serialized = format!("{}\n", serde_json::to_string_pretty(&index)?);
    if options.check {
        let current = fs::read_to_string(&output).unwrap_or_default();
        if current != serialized {
            bail!("{} is out of date", output.display());
        }
        println!("{} is up to date", output.display());
    } else {
        fs::write(&output, serialized)
            .with_context(|| format!("failed to write {}", output.display()))?;
        println!("Generated {}", output.display());
    }
    Ok(())
}

pub fn install(options: InstallOptions) -> Result<()> {
    validate_plugin_id(&options.id)?;
    let index_url = plugin_index_url(options.index_url, &options.github_repo, &options.github_ref)?;
    let index = fetch_index(&index_url)?;
    let entry = index
        .plugins
        .iter()
        .find(|entry| entry.id == options.id)
        .with_context(|| format!("plugin {} was not found in {index_url}", options.id))?;
    install_entry(EntryInstallOptions {
        entry,
        index_url: &index_url,
        plugin_dir: options.plugin_dir,
        config_path: options.config_path,
        trust: options.trust,
        enable: options.enable,
        force: options.force,
    })
}

pub fn update(options: UpdateOptions) -> Result<()> {
    let (config_path, config) = load_config(options.config_path.clone())?;
    let discovery = discover_bundles(
        &config.plugins.dirs,
        &config.plugins.trusted,
        env!("CARGO_PKG_VERSION"),
    );
    let mut updated = 0usize;
    let mut matched = 0usize;

    for bundle in discovery.bundles {
        if options
            .id
            .as_ref()
            .is_some_and(|id| id != &bundle.manifest.id)
        {
            continue;
        }
        matched += 1;
        let Some(metadata) = read_install_metadata(&bundle.root)? else {
            println!("Skipping linked plugin {}", bundle.manifest.id);
            continue;
        };
        let index = fetch_index(&metadata.index_url)?;
        let entry = index
            .plugins
            .iter()
            .find(|entry| entry.id == metadata.id)
            .with_context(|| {
                format!(
                    "plugin {} was not found in {}",
                    metadata.id, metadata.index_url
                )
            })?;
        let current = Version::parse(&metadata.version)
            .with_context(|| format!("installed plugin {} has invalid version", metadata.id))?;
        let available = Version::parse(&entry.version)
            .with_context(|| format!("index plugin {} has invalid version", entry.id))?;
        if available <= current && !options.force {
            println!("{} is up to date ({})", metadata.id, metadata.version);
            continue;
        }
        let parent = bundle
            .root
            .parent()
            .map(Path::to_path_buf)
            .context("installed plugin root has no parent directory")?;
        install_entry(EntryInstallOptions {
            entry,
            index_url: &metadata.index_url,
            plugin_dir: Some(parent),
            config_path: Some(config_path.clone()),
            trust: config.plugins.trusted.iter().any(|id| id == &metadata.id),
            enable: true,
            force: true,
        })?;
        updated += 1;
    }

    if matched == 0 {
        if let Some(id) = options.id {
            bail!("plugin {id} is not installed or linked");
        }
        println!("No installed plugins found");
    } else if updated == 0 {
        println!("No plugin updates applied");
    }
    Ok(())
}

pub fn remove(options: RemoveOptions) -> Result<()> {
    let (config_path, mut config) = load_config(options.config_path)?;
    let discovery = discover_bundles(
        &config.plugins.dirs,
        &config.plugins.trusted,
        env!("CARGO_PKG_VERSION"),
    );
    let bundle = discovery
        .bundles
        .iter()
        .find(|bundle| bundle.manifest.id == options.id)
        .with_context(|| format!("plugin {} is not installed", options.id))?;
    let Some(_) = read_install_metadata(&bundle.root)? else {
        bail!(
            "plugin {} is linked from {}; use plugin unlink instead",
            options.id,
            bundle.root.display()
        );
    };
    let rule_ids = rule_pack_ids(&bundle.root).unwrap_or_default();
    fs::remove_dir_all(&bundle.root)
        .with_context(|| format!("failed to remove {}", bundle.root.display()))?;
    remove_config_references(&mut config, &options.id, &rule_ids);
    config.save_to(&config_path)?;
    println!("Removed plugin {}", options.id);
    Ok(())
}

pub fn link(options: LinkOptions) -> Result<()> {
    validate_bundle(&options.path)?;
    let root = fs::canonicalize(&options.path)
        .with_context(|| format!("failed to canonicalize {}", options.path.display()))?;
    let manifest = read_manifest(&root)?;
    let rule_ids = rule_pack_ids(&root)?;
    let (config_path, mut config) = load_config(options.config_path)?;
    if !config.plugins.dirs.iter().any(|dir| dir == &root) {
        config.plugins.dirs.push(root.clone());
    }
    if options.trust && !config.plugins.trusted.iter().any(|id| id == &manifest.id) {
        config.plugins.trusted.push(manifest.id.clone());
    }
    if options.enable {
        enable_rule_packs(&mut config, &rule_ids);
    }
    config.save_to(&config_path)?;
    println!("Linked plugin {} at {}", manifest.id, root.display());
    Ok(())
}

pub fn unlink(options: UnlinkOptions) -> Result<()> {
    let (config_path, mut config) = load_config(options.config_path)?;
    let discovery = discover_bundles(
        &config.plugins.dirs,
        &config.plugins.trusted,
        env!("CARGO_PKG_VERSION"),
    );
    let bundle = discovery
        .bundles
        .iter()
        .find(|bundle| bundle.manifest.id == options.id)
        .with_context(|| format!("plugin {} is not linked", options.id))?;
    let rule_ids = rule_pack_ids(&bundle.root).unwrap_or_default();
    let before = config.plugins.dirs.len();
    config.plugins.dirs.retain(|dir| dir != &bundle.root);
    if config.plugins.dirs.len() == before {
        bail!(
            "plugin {} is discovered through a parent directory; remove that directory from [plugins].dirs manually",
            options.id
        );
    }
    remove_config_references(&mut config, &options.id, &rule_ids);
    config.save_to(&config_path)?;
    println!("Unlinked plugin {}", options.id);
    Ok(())
}

pub fn list(config_path: Option<PathBuf>) -> Result<()> {
    let (_, config) = load_config(config_path)?;
    let discovery = discover_bundles(
        &config.plugins.dirs,
        &config.plugins.trusted,
        env!("CARGO_PKG_VERSION"),
    );
    if discovery.bundles.is_empty() {
        println!("No plugins found");
    }
    for bundle in discovery.bundles {
        let install_kind = if read_install_metadata(&bundle.root)?.is_some() {
            "installed"
        } else if config.plugins.dirs.iter().any(|dir| dir == &bundle.root) {
            "linked"
        } else {
            "local"
        };
        println!(
            "{} {} [{} / {:?}] {}",
            bundle.manifest.id,
            bundle.manifest.version,
            install_kind,
            bundle.trust,
            bundle.root.display()
        );
    }
    for diagnostic in discovery.diagnostics {
        println!("! {} {}", diagnostic.code, diagnostic.message);
    }
    Ok(())
}

pub fn search(options: SearchOptions) -> Result<()> {
    let index_url = plugin_index_url(options.index_url, &options.github_repo, &options.github_ref)?;
    let index = fetch_index(&index_url)?;
    let query = options.query.unwrap_or_default().to_ascii_lowercase();
    for entry in index
        .plugins
        .iter()
        .filter(|entry| matches_query(entry, &query))
    {
        println!(
            "{} {} [{}] {}",
            entry.id,
            entry.version,
            format_capabilities(&entry.capabilities),
            entry.name
        );
        if !entry.description.trim().is_empty() {
            println!("  {}", entry.description);
        }
    }
    Ok(())
}

pub fn info(options: InfoOptions) -> Result<()> {
    if options.local {
        return print_local_info(options.config_path, &options.id);
    }
    if print_local_info(options.config_path.clone(), &options.id).is_ok() {
        return Ok(());
    }
    let index_url = plugin_index_url(options.index_url, &options.github_repo, &options.github_ref)?;
    let index = fetch_index(&index_url)?;
    let entry = index
        .plugins
        .iter()
        .find(|entry| entry.id == options.id)
        .with_context(|| format!("plugin {} was not found in {index_url}", options.id))?;
    print_index_entry(entry);
    Ok(())
}

pub fn trust(options: TrustOptions) -> Result<()> {
    let (config_path, mut config) = load_config(options.config_path)?;
    validate_plugin_id(&options.id)?;
    if !config.plugins.trusted.iter().any(|id| id == &options.id) {
        config.plugins.trusted.push(options.id.clone());
    }
    config.save_to(&config_path)?;
    println!("Trusted plugin {}", options.id);
    Ok(())
}

pub fn untrust(options: TrustOptions) -> Result<()> {
    let (config_path, mut config) = load_config(options.config_path)?;
    validate_plugin_id(&options.id)?;
    config.plugins.trusted.retain(|id| id != &options.id);
    config.save_to(&config_path)?;
    println!("Untrusted plugin {}", options.id);
    Ok(())
}

pub fn doctor(config_path: Option<PathBuf>) -> Result<()> {
    let (_, config) = load_config(config_path)?;
    println!("Plugin dirs: {}", join_paths(&config.plugins.dirs));
    let discovery = discover_bundles(
        &config.plugins.dirs,
        &config.plugins.trusted,
        env!("CARGO_PKG_VERSION"),
    );
    println!("Discovered plugins: {}", discovery.bundles.len());
    for bundle in discovery.bundles {
        match validate_bundle(&bundle.root) {
            Ok(()) => println!("ok {} {}", bundle.manifest.id, bundle.root.display()),
            Err(error) => println!("! {} {}", bundle.manifest.id, error),
        }
    }
    for diagnostic in discovery.diagnostics {
        println!("! {} {}", diagnostic.code, diagnostic.message);
    }
    Ok(())
}

pub fn github_raw_plugin_index_url(repo: &str, reference: &str) -> Result<String> {
    validate_github_repo(repo)?;
    validate_github_ref(reference)?;
    Ok(format!(
        "https://raw.githubusercontent.com/{repo}/{reference}/plugins/index.json"
    ))
}

struct EntryInstallOptions<'a> {
    entry: &'a PluginIndexEntry,
    index_url: &'a str,
    plugin_dir: Option<PathBuf>,
    config_path: Option<PathBuf>,
    trust: bool,
    enable: bool,
    force: bool,
}

fn install_entry(options: EntryInstallOptions<'_>) -> Result<()> {
    validate_index_entry(options.entry)?;
    let plugin_dir = options
        .plugin_dir
        .or_else(cleanr_config::default_plugin_dir)
        .context("platform plugin directory is unavailable; pass --plugin-dir")?;
    fs::create_dir_all(&plugin_dir)
        .with_context(|| format!("failed to create {}", plugin_dir.display()))?;
    let target_dir = plugin_dir.join(&options.entry.id);
    if target_dir.exists() && !options.force {
        bail!(
            "plugin {} is already installed at {}; pass --force to replace it",
            options.entry.id,
            target_dir.display()
        );
    }

    let staging_parent = tempfile::tempdir_in(&plugin_dir).with_context(|| {
        format!(
            "failed to create staging directory in {}",
            plugin_dir.display()
        )
    })?;
    let staging_dir = staging_parent.path().join(&options.entry.id);
    fs::create_dir(&staging_dir)
        .with_context(|| format!("failed to create {}", staging_dir.display()))?;

    let client = download_client()?;
    for file in &options.entry.files {
        let relative = safe_plugin_file_path(&file.path)?;
        let body = fetch_bytes(&client, &file.url, MAX_PLUGIN_FILE_BYTES)
            .with_context(|| format!("failed to download plugin file {}", file.url))?;
        verify_index_file(file, &body)?;
        let output = staging_dir.join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&output, body)
            .with_context(|| format!("failed to write {}", output.display()))?;
    }

    validate_bundle(&staging_dir)?;
    let manifest = read_manifest(&staging_dir)?;
    if manifest.id != options.entry.id {
        bail!(
            "plugin index id {} does not match manifest id {}",
            options.entry.id,
            manifest.id
        );
    }
    write_install_metadata(
        &staging_dir,
        &InstalledPlugin {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            index_url: options.index_url.to_string(),
            files: options.entry.files.clone(),
        },
    )?;

    let backup_dir = if target_dir.exists() {
        let backup_dir = replacement_backup_dir(&plugin_dir, &options.entry.id)?;
        fs::rename(&target_dir, &backup_dir).with_context(|| {
            format!(
                "failed to move existing plugin {} aside before replacement",
                target_dir.display()
            )
        })?;
        Some(backup_dir)
    } else {
        None
    };
    if let Err(error) = fs::rename(&staging_dir, &target_dir) {
        if let Some(backup_dir) = &backup_dir {
            let _ = fs::rename(backup_dir, &target_dir);
        }
        return Err(error).with_context(|| {
            format!(
                "failed to install plugin {} into {}",
                manifest.id,
                target_dir.display()
            )
        });
    }
    if let Some(backup_dir) = backup_dir
        && let Err(error) = fs::remove_dir_all(&backup_dir)
    {
        eprintln!(
            "warning: installed plugin {}, but failed to remove backup {}: {}",
            manifest.id,
            backup_dir.display(),
            error
        );
    }

    let rule_pack_ids = rule_pack_ids(&target_dir)?;
    let (config_path, mut config) = load_config(options.config_path)?;
    if !config.plugins.dirs.iter().any(|dir| dir == &plugin_dir) {
        config.plugins.dirs.push(plugin_dir.clone());
    }
    if options.trust && !config.plugins.trusted.iter().any(|id| id == &manifest.id) {
        config.plugins.trusted.push(manifest.id.clone());
    }
    if options.enable {
        enable_rule_packs(&mut config, &rule_pack_ids);
    }
    config.save_to(&config_path)?;

    println!(
        "Installed plugin {} {} at {}",
        manifest.id,
        manifest.version,
        target_dir.display()
    );
    if options.enable && !rule_pack_ids.is_empty() {
        println!("Enabled rule packs: {}", rule_pack_ids.join(", "));
    }
    if options.trust {
        println!("Trusted plugin: {}", manifest.id);
    }
    println!("Config written to {}", config_path.display());
    Ok(())
}

fn validate_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        return validate_bundle(path);
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("toml") => {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            if path.file_name().and_then(|name| name.to_str()) == Some("plugin.toml") {
                PluginManifest::from_toml(&raw, env!("CARGO_PKG_VERSION"))?;
            } else if path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some("locations")
            {
                cleanr_rules::scan_location_pack_from_toml(&raw)?;
            } else {
                cleanr_rules::RulePack::from_toml(&raw)?;
            }
            Ok(())
        }
        Some("yml" | "yaml") => cleanr_i18n::validate_language_file(path),
        Some("json") => {
            let raw =
                fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
            let index: PluginIndex =
                serde_json::from_slice(&raw).context("failed to parse plugin index JSON")?;
            validate_index(&index)
        }
        _ => bail!("unsupported plugin file: {}", path.display()),
    }
}

fn validate_bundle(root: &Path) -> Result<()> {
    let manifest_path = root.join("plugin.toml");
    let raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest = PluginManifest::from_toml(&raw, env!("CARGO_PKG_VERSION"))?;

    if manifest.capabilities.contains(&PluginCapability::Rules) {
        validate_rule_directory(&root.join("rules"))?;
    }
    if manifest
        .capabilities
        .contains(&PluginCapability::ScanLocations)
    {
        validate_scan_location_directory(&root.join("locations"))?;
    }
    if manifest
        .capabilities
        .contains(&PluginCapability::Translations)
    {
        let paths = sorted_dir_entries(root.join("locales"))?
            .into_iter()
            .filter(|path| {
                matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("yml" | "yaml")
                )
            })
            .collect::<Vec<_>>();
        if paths.is_empty() {
            bail!("translations capability requires at least one locale file");
        }
        let mut locales = BTreeSet::new();
        for path in paths {
            let locale = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .context("language file name is not valid UTF-8")?;
            if !locales.insert(locale.to_ascii_lowercase()) {
                bail!("duplicate locale {locale} in {}", root.display());
            }
            cleanr_i18n::validate_language_file(path)?;
        }
    }
    Ok(())
}

fn validate_rule_directory(directory: &Path) -> Result<()> {
    let paths = sorted_dir_entries(directory)?
        .into_iter()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        bail!("rules capability requires at least one rule pack");
    }
    let mut ids = BTreeSet::new();
    for path in paths {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let pack = cleanr_rules::RulePack::from_toml(&raw)
            .with_context(|| format!("failed to validate {}", path.display()))?;
        if !ids.insert(pack.id.clone()) {
            bail!(
                "duplicate rule pack id {} in {}",
                pack.id,
                directory.display()
            );
        }
    }
    Ok(())
}

fn validate_scan_location_directory(directory: &Path) -> Result<()> {
    let paths = sorted_dir_entries(directory)?
        .into_iter()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        bail!("scan-locations capability requires at least one location pack");
    }
    let mut ids = BTreeSet::new();
    for path in paths {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let pack = cleanr_rules::scan_location_pack_from_toml(&raw)
            .with_context(|| format!("failed to validate {}", path.display()))?;
        for location in pack.locations {
            if !ids.insert(location.id.clone()) {
                bail!("duplicate scan location id {}", location.id);
            }
        }
    }
    Ok(())
}

fn validate_index(index: &PluginIndex) -> Result<()> {
    if index.schema_version != PLUGIN_INDEX_SCHEMA_VERSION {
        bail!(
            "plugin index schema version {} is not supported",
            index.schema_version
        );
    }
    let mut ids = BTreeSet::new();
    for entry in &index.plugins {
        validate_index_entry(entry)?;
        if !ids.insert(entry.id.as_str()) {
            bail!("plugin index contains duplicate plugin id {}", entry.id);
        }
    }
    Ok(())
}

fn validate_index_entry(entry: &PluginIndexEntry) -> Result<()> {
    validate_plugin_id(&entry.id)?;
    if entry.name.trim().is_empty() {
        bail!("plugin {} has an empty name", entry.id);
    }
    Version::parse(&entry.version)
        .with_context(|| format!("plugin {} has an invalid semantic version", entry.id))?;
    if entry.capabilities.is_empty() {
        bail!("plugin {} declares no capabilities", entry.id);
    }
    if let Some(requirement) = &entry.cleanr_version {
        let requirement = VersionReq::parse(requirement).with_context(|| {
            format!(
                "plugin {} has an invalid cleanr_version requirement",
                entry.id
            )
        })?;
        let host =
            Version::parse(env!("CARGO_PKG_VERSION")).context("host cleanr version is invalid")?;
        if !requirement.matches(&host) {
            bail!(
                "plugin {} requires cleanr {}; current version is {}",
                entry.id,
                requirement,
                host
            );
        }
    }
    if entry.files.is_empty() {
        bail!("plugin {} has no files", entry.id);
    }
    if !entry.files.iter().any(|file| file.path == "plugin.toml") {
        bail!("plugin {} must include plugin.toml", entry.id);
    }
    let mut paths = BTreeSet::new();
    for file in &entry.files {
        safe_plugin_file_path(&file.path)?;
        if !paths.insert(file.path.as_str()) {
            bail!("plugin {} contains duplicate file {}", entry.id, file.path);
        }
        if file.url.trim().is_empty() {
            bail!("plugin {} file {} has an empty URL", entry.id, file.path);
        }
        if file.size > MAX_PLUGIN_FILE_BYTES {
            bail!(
                "plugin {} file {} exceeds the {} byte limit",
                entry.id,
                file.path,
                MAX_PLUGIN_FILE_BYTES
            );
        }
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!(
                "plugin {} file {} has an invalid SHA-256",
                entry.id,
                file.path
            );
        }
    }
    Ok(())
}

fn fetch_index(index_url: &str) -> Result<PluginIndex> {
    let client = download_client()?;
    let index_body = fetch_bytes(&client, index_url, MAX_PLUGIN_INDEX_BYTES)
        .with_context(|| format!("failed to download plugin index {index_url}"))?;
    let index: PluginIndex =
        serde_json::from_slice(&index_body).context("failed to parse plugin index JSON")?;
    validate_index(&index)?;
    Ok(index)
}

fn download_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to create plugin download client")
}

fn fetch_bytes(
    client: &reqwest::blocking::Client,
    location: &str,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    if location.starts_with("http://") || location.starts_with("https://") {
        let response = client
            .get(location)
            .send()
            .with_context(|| format!("failed to GET {location}"))?;
        if !response.status().is_success() {
            bail!("failed to download {location}: HTTP {}", response.status());
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes)
        {
            bail!(
                "download from {location} exceeds the {} byte limit",
                max_bytes
            );
        }
        let mut body = Vec::new();
        response
            .take(max_bytes + 1)
            .read_to_end(&mut body)
            .with_context(|| format!("failed to read response body from {location}"))?;
        if body.len() as u64 > max_bytes {
            bail!(
                "download from {location} exceeds the {} byte limit",
                max_bytes
            );
        }
        return Ok(body);
    }

    let path = location.strip_prefix("file://").unwrap_or(location);
    let metadata = fs::metadata(path).with_context(|| format!("failed to stat {path}"))?;
    if metadata.len() > max_bytes {
        bail!("file {path} exceeds the {} byte limit", max_bytes);
    }
    fs::read(path).with_context(|| format!("failed to read {path}"))
}

fn verify_index_file(file: &PluginIndexFile, body: &[u8]) -> Result<()> {
    if body.len() as u64 != file.size {
        bail!(
            "plugin file {} size mismatch: expected {}, got {}",
            file.path,
            file.size,
            body.len()
        );
    }
    let actual = format!("{:x}", Sha256::digest(body));
    if !actual.eq_ignore_ascii_case(&file.sha256) {
        bail!(
            "plugin file {} SHA-256 mismatch: expected {}, got {}",
            file.path,
            file.sha256,
            actual
        );
    }
    Ok(())
}

fn read_manifest(root: &Path) -> Result<PluginManifest> {
    let manifest_path = root.join("plugin.toml");
    let raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    PluginManifest::from_toml(&raw, env!("CARGO_PKG_VERSION"))
}

fn rule_pack_ids(root: &Path) -> Result<Vec<String>> {
    let rules_dir = root.join("rules");
    let Ok(paths) = sorted_dir_entries(&rules_dir) else {
        return Ok(Vec::new());
    };
    let mut ids = Vec::new();
    for path in paths
        .into_iter()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
    {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let pack = cleanr_rules::RulePack::from_toml(&raw)
            .with_context(|| format!("failed to validate {}", path.display()))?;
        ids.push(pack.id);
    }
    Ok(ids)
}

fn safe_plugin_file_path(path: &str) -> Result<PathBuf> {
    let raw = Path::new(path);
    if raw.is_absolute() {
        bail!("plugin file path must be relative: {path}");
    }
    let mut normalized = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            _ => bail!("plugin file path contains unsupported component: {path}"),
        }
    }
    if normalized.as_os_str().is_empty() {
        bail!("plugin file path cannot be empty");
    }
    Ok(normalized)
}

fn replacement_backup_dir(plugin_dir: &Path, plugin_id: &str) -> Result<PathBuf> {
    for attempt in 0..100 {
        let candidate = plugin_dir.join(format!(
            ".{plugin_id}.replace-{}-{attempt}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("failed to allocate a replacement backup path for plugin {plugin_id}")
}

fn plugin_index_url(index_url: Option<String>, repo: &str, reference: &str) -> Result<String> {
    match index_url {
        Some(url) => Ok(url),
        None => github_raw_plugin_index_url(repo, reference),
    }
}

fn validate_github_repo(repo: &str) -> Result<()> {
    if repo.split('/').count() != 2
        || !repo
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_'))
    {
        bail!("GitHub repo must be in owner/name form");
    }
    Ok(())
}

fn validate_github_ref(reference: &str) -> Result<()> {
    if reference.is_empty()
        || !reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/'))
        || reference.contains("..")
    {
        bail!("GitHub reference contains unsupported characters");
    }
    Ok(())
}

fn load_config(path: Option<PathBuf>) -> Result<(PathBuf, Config)> {
    let config_path = path
        .or_else(cleanr_config::default_config_path)
        .context("platform config directory is unavailable; pass --config")?;
    let config = if config_path.exists() {
        Config::load_from(&config_path)?
    } else {
        Config::default()
    };
    Ok((config_path, config))
}

fn enable_rule_packs(config: &mut Config, rule_pack_ids: &[String]) {
    for id in rule_pack_ids {
        if !config
            .cleanup
            .enabled_rule_packs
            .iter()
            .any(|enabled| enabled == id)
        {
            config.cleanup.enabled_rule_packs.push(id.clone());
        }
    }
}

fn remove_config_references(config: &mut Config, plugin_id: &str, rule_pack_ids: &[String]) {
    config.plugins.trusted.retain(|id| id != plugin_id);
    config
        .cleanup
        .enabled_rule_packs
        .retain(|enabled| !rule_pack_ids.iter().any(|id| id == enabled));
}

fn read_install_metadata(root: &Path) -> Result<Option<InstalledPlugin>> {
    let path = root.join(INSTALLED_PLUGIN_METADATA_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))
        .map(Some)
}

fn write_install_metadata(root: &Path, metadata: &InstalledPlugin) -> Result<()> {
    let path = root.join(INSTALLED_PLUGIN_METADATA_FILE);
    let raw = format!("{}\n", serde_json::to_string_pretty(metadata)?);
    fs::write(&path, raw).with_context(|| format!("failed to write {}", path.display()))
}

fn print_local_info(config_path: Option<PathBuf>, id: &str) -> Result<()> {
    let (_, config) = load_config(config_path)?;
    let discovery = discover_bundles(
        &config.plugins.dirs,
        &config.plugins.trusted,
        env!("CARGO_PKG_VERSION"),
    );
    let bundle = discovery
        .bundles
        .iter()
        .find(|bundle| bundle.manifest.id == id)
        .with_context(|| format!("plugin {id} was not found locally"))?;
    let metadata = read_install_metadata(&bundle.root)?;
    println!("{} {}", bundle.manifest.id, bundle.manifest.version);
    println!("{}", bundle.manifest.name);
    if !bundle.manifest.description.trim().is_empty() {
        println!("{}", bundle.manifest.description);
    }
    println!(
        "Capabilities: {}",
        format_capabilities(&bundle.manifest.capabilities)
    );
    println!("Trust: {:?}", bundle.trust);
    println!("Path: {}", bundle.root.display());
    if let Some(metadata) = metadata {
        println!("Index: {}", metadata.index_url);
    }
    Ok(())
}

fn print_index_entry(entry: &PluginIndexEntry) {
    println!("{} {}", entry.id, entry.version);
    println!("{}", entry.name);
    if !entry.description.trim().is_empty() {
        println!("{}", entry.description);
    }
    println!("Capabilities: {}", format_capabilities(&entry.capabilities));
    if !entry.categories.is_empty() {
        println!("Categories: {}", entry.categories.join(", "));
    }
    if !entry.keywords.is_empty() {
        println!("Keywords: {}", entry.keywords.join(", "));
    }
    if let Some(repository) = &entry.repository {
        println!("Repository: {repository}");
    }
    if let Some(homepage) = &entry.homepage {
        println!("Homepage: {homepage}");
    }
    if let Some(license) = &entry.license {
        println!("License: {license}");
    }
}

fn matches_query(entry: &PluginIndexEntry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut haystack = vec![
        entry.id.as_str(),
        entry.name.as_str(),
        entry.description.as_str(),
    ];
    haystack.extend(entry.categories.iter().map(String::as_str));
    haystack.extend(entry.keywords.iter().map(String::as_str));
    haystack
        .iter()
        .any(|value| value.to_ascii_lowercase().contains(query))
}

fn format_capabilities(capabilities: &BTreeSet<PluginCapability>) -> String {
    capabilities
        .iter()
        .map(|capability| match capability {
            PluginCapability::Rules => "rules",
            PluginCapability::ScanLocations => "scan-locations",
            PluginCapability::Translations => "translations",
            PluginCapability::DynamicCandidates => "dynamic-candidates",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn publishable_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut output = Vec::new();
    collect_publishable_files(root, root, &mut output)?;
    output.sort();
    Ok(output)
}

fn collect_publishable_files(root: &Path, dir: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for path in sorted_dir_entries(dir)? {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.starts_with('.') || name == "index.json" {
            continue;
        }
        if path.is_dir() {
            collect_publishable_files(root, &path, output)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("failed to relativize {}", path.display()))?;
            if relative.components().any(|component| {
                matches!(component, Component::Normal(part) if part.to_string_lossy().starts_with('.'))
            }) {
                continue;
            }
            output.push(path);
        }
    }
    Ok(())
}

fn plugin_index_file(
    base_url: &str,
    bundle_name: &str,
    bundle_dir: &Path,
    path: &Path,
) -> Result<PluginIndexFile> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let rel = slash_path(
        path.strip_prefix(bundle_dir)
            .with_context(|| format!("failed to relativize {}", path.display()))?,
    )?;
    Ok(PluginIndexFile {
        path: rel.clone(),
        url: format!("{base_url}/{}", url_path(&format!("{bundle_name}/{rel}"))?),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        size: bytes.len() as u64,
    })
}

fn slash_path(path: &Path) -> Result<String> {
    path.components()
        .map(|component| match component {
            Component::Normal(part) => part
                .to_str()
                .map(str::to_string)
                .context("plugin path is not valid UTF-8"),
            _ => bail!("plugin path contains unsupported component"),
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

fn url_path(path: &str) -> Result<String> {
    path.split('/')
        .map(percent_encode)
        .collect::<Vec<_>>()
        .join("/")
        .pipe(Ok)
}

fn percent_encode(segment: &str) -> String {
    let mut output = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn join_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "(none)".to_string();
    }
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_empty_dir(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    if !path.is_dir() {
        bail!("{} exists and is not a directory", path.display());
    }
    Ok(fs::read_dir(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .next()
        .is_none())
}

fn write_template_file(path: &Path, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite",
            path.display()
        );
    }
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

fn plugin_template(id: &str, name: &str) -> String {
    format!(
        r#"api_version = "1"
id = "{id}"
name = "{name}"
version = "0.1.0"
description = "Cleanup rules for {name}."
cleanr_version = ">=0.1.0"
capabilities = ["rules"]
categories = ["developer"]
keywords = ["cache"]
"#
    )
}

fn rule_pack_template(id: &str, name: &str) -> String {
    let pack_id = id.replace(['.', '_'], "-");
    format!(
        r#"id = "{pack_id}"
name = "{name}"
version = "0.1.0"
description = "Cleanup rules for {name}."
categories = ["cache"]

[[rules]]
id = "example-cache"
label = "{name} cache"
category = "cache"
match = {{ dir_name = ".example-cache" }}
confidence = "high"
default_selected = true
action = "trash"
reason = "{name} can recreate this cache."
risk_note = "The next {name} run may be slower."
"#
    )
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(capabilities: &str) -> String {
        format!(
            r#"
api_version = "1"
id = "test.plugin"
name = "Test"
version = "1.0.0"
capabilities = [{capabilities}]
"#
        )
    }

    fn rule_pack(id: &str) -> String {
        format!(
            r#"
id = "{id}"
name = "Test"
version = "1.0.0"
description = "Test"
categories = ["cache"]

[[rules]]
id = "cache"
label = "Cache"
category = "cache"
match = {{ dir_name = "cache" }}
confidence = "high"
default_selected = true
action = "trash"
reason = "generated"
risk_note = "rebuild"
"#
        )
    }

    #[test]
    fn validates_complete_bundle_with_rules_and_translations() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("rules")).expect("rules dir");
        fs::create_dir(temp.path().join("locales")).expect("locales dir");
        fs::write(
            temp.path().join("plugin.toml"),
            manifest(r#""rules", "translations""#),
        )
        .expect("manifest");
        fs::write(
            temp.path().join("rules").join("rules.toml"),
            rule_pack("test"),
        )
        .expect("rules");
        fs::write(
            temp.path().join("locales").join("en-XA.yml"),
            "_version: 1\nname: Test\nversion: 1.0.0\nlabel_status: Test\n",
        )
        .expect("locale");

        validate_path(temp.path()).expect("valid bundle");
    }

    #[test]
    fn bundle_capabilities_require_matching_content() {
        let rules = tempfile::tempdir().expect("rules tempdir");
        fs::write(rules.path().join("plugin.toml"), manifest(r#""rules""#)).expect("manifest");
        assert!(
            validate_path(rules.path())
                .expect_err("missing rules")
                .to_string()
                .contains("rules")
        );

        let translations = tempfile::tempdir().expect("translations tempdir");
        fs::write(
            translations.path().join("plugin.toml"),
            manifest(r#""translations""#),
        )
        .expect("manifest");
        fs::create_dir(translations.path().join("locales")).expect("locales");
        assert!(
            validate_path(translations.path())
                .expect_err("missing translations")
                .to_string()
                .contains("at least one locale")
        );

        let locations = tempfile::tempdir().expect("locations tempdir");
        fs::write(
            locations.path().join("plugin.toml"),
            manifest(r#""scan-locations""#),
        )
        .expect("manifest");
        fs::create_dir(locations.path().join("locations")).expect("locations");
        assert!(
            validate_path(locations.path())
                .expect_err("missing scan locations")
                .to_string()
                .contains("at least one location pack")
        );
    }

    #[test]
    fn duplicate_rule_pack_ids_are_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("a.toml"), rule_pack("duplicate")).expect("a");
        fs::write(temp.path().join("b.toml"), rule_pack("duplicate")).expect("b");

        let error = validate_rule_directory(temp.path()).expect_err("duplicate packs");

        assert!(error.to_string().contains("duplicate rule pack id"));
    }

    #[test]
    fn rejects_unsupported_files_empty_inputs_and_unknown_schemas() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("plugin.txt");
        fs::write(&path, "text").expect("file");

        assert!(validate(&[]).is_err());
        assert!(validate_path(&path).is_err());
        assert!(print_schema("unknown").is_err());
    }

    #[test]
    fn github_plugin_index_url_is_constrained_to_safe_repo_refs() {
        assert_eq!(
            github_raw_plugin_index_url("owner/repo", "main").expect("url"),
            "https://raw.githubusercontent.com/owner/repo/main/plugins/index.json"
        );
        assert!(github_raw_plugin_index_url("owner", "main").is_err());
        assert!(github_raw_plugin_index_url("owner/repo", "../main").is_err());
    }

    #[test]
    fn plugin_file_paths_cannot_escape_install_root() {
        assert_eq!(
            safe_plugin_file_path("rules/caches.toml").expect("path"),
            PathBuf::from("rules").join("caches.toml")
        );
        assert!(safe_plugin_file_path("../plugin.toml").is_err());
        assert!(safe_plugin_file_path("/tmp/plugin.toml").is_err());
        assert!(safe_plugin_file_path("").is_err());
    }

    #[test]
    fn plugin_index_entries_require_manifest_and_hashes() {
        let mut entry = PluginIndexEntry {
            id: "example.caches".to_string(),
            name: "Example".to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            cleanr_version: Some(">=0.1.0".to_string()),
            capabilities: BTreeSet::from([PluginCapability::Rules]),
            categories: Vec::new(),
            keywords: Vec::new(),
            author: None,
            homepage: None,
            repository: None,
            license: None,
            source: None,
            files: vec![PluginIndexFile {
                path: "plugin.toml".to_string(),
                url: "https://example.invalid/plugin.toml".to_string(),
                sha256: "0".repeat(64),
                size: 12,
            }],
        };
        validate_index_entry(&entry).expect("valid entry");

        entry.files[0].sha256 = "not-a-hash".to_string();
        assert!(validate_index_entry(&entry).is_err());
    }

    #[test]
    fn init_template_validates() {
        let temp = tempfile::tempdir().expect("tempdir");
        init(InitOptions {
            path: temp.path().join("example"),
            id: "example.cache".to_string(),
            name: "Example".to_string(),
            force: false,
        })
        .expect("init plugin");
        validate_path(&temp.path().join("example")).expect("valid template");
    }

    #[test]
    fn generated_index_uses_plugins_collection() {
        let temp = tempfile::tempdir().expect("tempdir");
        init(InitOptions {
            path: temp.path().join("plugins").join("example"),
            id: "example.cache".to_string(),
            name: "Example".to_string(),
            force: false,
        })
        .expect("init plugin");
        let output = temp.path().join("plugins").join("index.json");
        generate_index(IndexOptions {
            plugin_dir: temp.path().join("plugins"),
            output: Some(output.clone()),
            base_url: "https://example.invalid/plugins".to_string(),
            check: false,
        })
        .expect("generate index");
        let raw = fs::read_to_string(output).expect("index");
        assert!(raw.contains(r#""plugins""#));
        assert!(!raw.contains(r#""extensions""#));
    }

    #[test]
    fn install_from_file_index_writes_metadata_and_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source").join("example");
        init(InitOptions {
            path: source.clone(),
            id: "example.cache".to_string(),
            name: "Example".to_string(),
            force: false,
        })
        .expect("init plugin");
        let manifest = read_manifest(&source).expect("manifest");
        let files = publishable_files(&source)
            .expect("files")
            .into_iter()
            .map(|path| {
                let bytes = fs::read(&path).expect("read plugin file");
                PluginIndexFile {
                    path: slash_path(path.strip_prefix(&source).expect("relative")).expect("rel"),
                    url: path.display().to_string(),
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    size: bytes.len() as u64,
                }
            })
            .collect::<Vec<_>>();
        let index = PluginIndex {
            schema_version: PLUGIN_INDEX_SCHEMA_VERSION,
            plugins: vec![PluginIndexEntry {
                id: manifest.id.clone(),
                name: manifest.name,
                version: manifest.version,
                description: manifest.description,
                cleanr_version: manifest.cleanr_version,
                capabilities: manifest.capabilities,
                categories: Vec::new(),
                keywords: Vec::new(),
                author: None,
                homepage: None,
                repository: None,
                license: None,
                source: None,
                files,
            }],
        };
        let index_path = temp.path().join("index.json");
        fs::write(
            &index_path,
            serde_json::to_string_pretty(&index).expect("index json"),
        )
        .expect("write index");
        let install_dir = temp.path().join("installed");
        let config_path = temp.path().join("config.toml");

        install(InstallOptions {
            id: "example.cache".to_string(),
            index_url: Some(index_path.display().to_string()),
            github_repo: "owner/repo".to_string(),
            github_ref: "main".to_string(),
            plugin_dir: Some(install_dir.clone()),
            config_path: Some(config_path.clone()),
            trust: true,
            enable: true,
            force: false,
        })
        .expect("install plugin");

        let installed_root = install_dir.join("example.cache");
        assert!(installed_root.join("plugin.toml").is_file());
        assert!(
            installed_root
                .join(INSTALLED_PLUGIN_METADATA_FILE)
                .is_file()
        );
        let config = Config::load_from(config_path).expect("config");
        assert!(config.plugins.dirs.iter().any(|dir| dir == &install_dir));
        assert!(
            config
                .plugins
                .trusted
                .iter()
                .any(|id| id == "example.cache")
        );
        assert!(
            config
                .cleanup
                .enabled_rule_packs
                .iter()
                .any(|id| id == "example-cache")
        );
    }
}
