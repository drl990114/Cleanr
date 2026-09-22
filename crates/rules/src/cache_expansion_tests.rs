use super::*;
use cleanr_core::{Confidence, ReadOnlyScope};

fn entry(path: &str, kind: EntryKind) -> ScanEntry {
    ScanEntry {
        path: PathBuf::from(path),
        kind,
        size_bytes: 20 * 1024 * 1024,
        modified_at: Some(Utc::now()),
        rule_hits: vec![],
    }
}

#[test]
fn cache_expansion_platform_paths_are_review_only_and_guarded() {
    let registry = RuleRegistry::builtin().unwrap();
    let cases = [
        (
            RulePlatform::Macos,
            "/u/Library/Caches/ms-playwright",
            "macos-playwright-cache",
        ),
        (
            RulePlatform::Linux,
            "/u/.cache/ms-playwright",
            "linux-playwright-cache",
        ),
        (
            RulePlatform::Windows,
            "C:/Users/u/AppData/Local/ms-playwright",
            "windows-playwright-cache",
        ),
        (
            RulePlatform::Macos,
            "/u/Library/Caches/electron",
            "macos-electron-cache",
        ),
        (
            RulePlatform::Linux,
            "/u/.cache/electron",
            "linux-electron-cache",
        ),
        (
            RulePlatform::Windows,
            "C:/Users/u/AppData/Local/electron/Cache",
            "windows-electron-cache",
        ),
        (
            RulePlatform::Macos,
            "/u/Library/Caches/Cypress",
            "macos-cypress-cache",
        ),
        (
            RulePlatform::Linux,
            "/u/.cache/Cypress",
            "linux-cypress-cache",
        ),
        (
            RulePlatform::Windows,
            "C:/Users/u/AppData/Local/Cypress/Cache",
            "windows-cypress-cache",
        ),
        (
            RulePlatform::Linux,
            "/u/.cache/go-build",
            "linux-go-build-cache",
        ),
        (
            RulePlatform::Windows,
            "C:/Users/u/AppData/Local/go-build",
            "windows-go-build-cache",
        ),
        (
            RulePlatform::Windows,
            "C:/Users/u/AppData/Local/pip/Cache",
            "windows-pip-download-cache",
        ),
        (
            RulePlatform::Macos,
            "/u/Library/Caches/Mozilla.sccache",
            "macos-sccache",
        ),
        (RulePlatform::Linux, "/u/.cache/sccache", "linux-sccache"),
        (
            RulePlatform::Windows,
            "C:/Users/u/AppData/Local/Mozilla/sccache",
            "windows-sccache",
        ),
        (
            RulePlatform::Linux,
            "/tmp/torchinductor_alice",
            "linux-torchinductor-cache",
        ),
        (
            RulePlatform::Linux,
            "/tmp/torchinductor_alice/triton",
            "linux-torchinductor-triton-cache",
        ),
    ];
    for (platform, path, id) in cases {
        let candidate = entry(path, EntryKind::Directory);
        let hits = registry.hits_for_at_on_platform(&candidate, Utc::now(), platform);
        let hit = hits
            .iter()
            .find(|hit| hit.rule_id == id)
            .unwrap_or_else(|| panic!("{id}: {path}"));
        assert_eq!(hit.confidence, Confidence::Medium);
        assert!(!hit.default_selected);
        assert!(hit.read_only_scope.is_none());
        assert!(hit.runtime_guard.is_some());
        for other in [
            RulePlatform::Macos,
            RulePlatform::Linux,
            RulePlatform::Windows,
        ] {
            if other != platform {
                assert!(
                    !registry
                        .hits_for_at_on_platform(&candidate, Utc::now(), other)
                        .iter()
                        .any(|hit| hit.rule_id == id)
                );
            }
        }
        for wrong in [
            entry(path, EntryKind::File),
            entry(&format!("{path}/nested"), EntryKind::Directory),
            entry(&path.replace("/", "/fake/"), EntryKind::Directory),
        ] {
            assert!(
                !registry
                    .hits_for_at_on_platform(&wrong, Utc::now(), platform)
                    .iter()
                    .any(|hit| hit.rule_id == id),
                "false positive {id}: {}",
                wrong.path.display()
            );
        }
    }
    for platform in [
        RulePlatform::Macos,
        RulePlatform::Linux,
        RulePlatform::Windows,
    ] {
        for (path, id) in [
            ("/u/.cache/puppeteer", "puppeteer-browser-cache"),
            (
                "/u/.cache/huggingface/xet/chunk_cache",
                "huggingface-xet-legacy-chunks",
            ),
            (
                "/u/.cache/huggingface/xet/environment/chunk_cache",
                "huggingface-xet-chunks",
            ),
        ] {
            let hits = registry.hits_for_at_on_platform(
                &entry(path, EntryKind::Directory),
                Utc::now(),
                platform,
            );
            assert!(
                hits.iter().any(|hit| hit.rule_id == id
                    && !hit.default_selected
                    && hit.runtime_guard.is_some()),
                "{path}"
            );
            assert!(
                hits.iter().all(|hit| hit.read_only_scope.is_none()),
                "must allow {path}"
            );
        }
    }
}

#[test]
fn cache_expansion_jetbrains_retains_history_unknown_products_and_future_state() {
    let registry = RuleRegistry::builtin().unwrap();
    for (platform, prefix) in [
        (RulePlatform::Macos, "/u/Library/Caches"),
        (RulePlatform::Linux, "/u/.cache"),
        (RulePlatform::Windows, "C:/Users/u/AppData/Local"),
    ] {
        for product in [
            "IntelliJIdea2026.2",
            "IdeaIC2025.3",
            "PyCharm2026.1",
            "PyCharmCE2024.2",
            "WebStorm2026.2",
        ] {
            let base = format!("{prefix}/JetBrains/{product}");
            let hits = registry.hits_for_at_on_platform(
                &entry(&format!("{base}/index"), EntryKind::Directory),
                Utc::now(),
                platform,
            );
            assert!(
                hits.iter()
                    .any(|hit| hit.rule_id.ends_with("jetbrains-index"))
            );
            assert!(
                hits.iter().all(|hit| hit.read_only_scope.is_none()),
                "index should be reviewable: {base}"
            );
            let logs = if platform == RulePlatform::Macos {
                format!("/u/Library/Logs/JetBrains/{product}")
            } else {
                format!("{base}/log")
            };
            let log_hits = registry.hits_for_at_on_platform(
                &entry(&logs, EntryKind::Directory),
                Utc::now(),
                platform,
            );
            assert!(
                log_hits
                    .iter()
                    .any(|hit| hit.rule_id.ends_with("jetbrains-logs"))
            );
            assert!(
                log_hits.iter().all(|hit| hit.read_only_scope.is_none()),
                "review logs {logs}"
            );
            for retained in [
                "LocalHistory",
                "LocalHistory/node_modules",
                "plugins",
                "config",
                "scratches",
                "future-state",
                "future-state/node_modules",
            ] {
                let path = format!("{base}/{retained}");
                let hits = registry.hits_for_at_on_platform(
                    &entry(&path, EntryKind::Directory),
                    Utc::now(),
                    platform,
                );
                assert!(
                    hits.iter()
                        .any(|hit| hit.read_only_scope == Some(ReadOnlyScope::Subtree)),
                    "retain {path}"
                );
            }
        }
        for product in ["AndroidStudio2026.1", "Unreviewed2026.1", "PyCharmBackup"] {
            let path = format!("{prefix}/JetBrains/{product}/node_modules");
            assert!(
                registry
                    .hits_for_at_on_platform(
                        &entry(&path, EntryKind::Directory),
                        Utc::now(),
                        platform
                    )
                    .iter()
                    .any(|hit| hit.read_only_scope == Some(ReadOnlyScope::Subtree))
            );
        }
    }
}

#[test]
fn cache_expansion_uv_and_model_protection_is_inherited_by_explicit_child_scans() {
    let registry = RuleRegistry::builtin().unwrap();
    for platform in [
        RulePlatform::Macos,
        RulePlatform::Linux,
        RulePlatform::Windows,
    ] {
        for path in [
            "/u/.cache/uv",
            "/u/.cache/uv/archive-v0/node_modules",
            "/u/.cache/huggingface/hub/node_modules",
            "/u/.cache/huggingface/xet/environment/staging/node_modules",
            "/u/.cache/huggingface/xet/staging/chunk_cache",
            "/u/miniforge3/envs/env/node_modules",
            "/u/miniforge3/pkgs/fake.conda/node_modules",
        ] {
            let hits = registry.hits_for_at_on_platform(
                &entry(path, EntryKind::Directory),
                Utc::now(),
                platform,
            );
            assert!(
                hits.iter()
                    .any(|hit| hit.read_only_scope == Some(ReadOnlyScope::Subtree)),
                "retain {path}"
            );
        }
    }
}

#[test]
fn cache_expansion_conda_archive_requires_regular_marker_in_same_snapshot() {
    let registry = RuleRegistry::builtin().unwrap();
    for suffix in ["package-1.0-0.conda", "package-1.0-0.tar.bz2"] {
        let archive = entry(&format!("/u/miniforge3/pkgs/{suffix}"), EntryKind::File);
        for marker_kind in [
            None,
            Some(EntryKind::Symlink),
            Some(EntryKind::Directory),
            Some(EntryKind::File),
        ] {
            let mut entries = vec![archive.clone()];
            if let Some(kind) = marker_kind {
                entries.push(entry("/u/miniforge3/pkgs/urls.txt", kind));
            }
            registry.annotate_entries(&mut entries);
            let matched = entries[0]
                .rule_hits
                .iter()
                .any(|hit| hit.rule_id == "conda-package-archives");
            assert_eq!(matched, marker_kind == Some(EntryKind::File));
            if matched {
                assert!(
                    entries[0]
                        .rule_hits
                        .iter()
                        .all(|hit| hit.read_only_scope.is_none())
                );
            }
        }
    }
    // A directory with an archive suffix, arbitrary Downloads, and extracted package contents
    // never become compressed-download candidates, even when they carry a urls.txt marker.
    for (path, kind) in [
        ("/u/miniforge3/pkgs/fake.conda", EntryKind::Directory),
        ("/u/Downloads/fake.conda", EntryKind::File),
        ("/u/miniforge3/pkgs/extracted/inner.conda", EntryKind::File),
    ] {
        let mut entries = vec![
            entry(path, kind),
            entry(
                &format!("{}/urls.txt", Path::new(path).parent().unwrap().display()),
                EntryKind::File,
            ),
        ];
        registry.annotate_entries(&mut entries);
        assert!(
            !entries[0]
                .rule_hits
                .iter()
                .any(|hit| hit.rule_id == "conda-package-archives")
        );
    }
}

#[test]
fn cache_expansion_inspection_schema_fails_closed_and_preserves_old_packs() {
    let pack = RuleRegistry::builtin().unwrap().packs[0].definition.clone();
    let mut pack = pack;
    let inspector = pack
        .rules
        .iter()
        .find(|rule| rule.action == crate::RuleAction::Inspect)
        .unwrap()
        .clone();
    pack.rules = vec![inspector.clone()];
    pack.rules[0].default_selected = true;
    assert!(pack.validate().is_err());
    pack.rules[0] = inspector.clone();
    pack.rules[0].matcher.min_size = Some(1);
    assert!(
        pack.validate().is_err(),
        "retention must not depend on scan size"
    );
    pack.rules[0] = inspector;
    pack.rules[0].action = crate::RuleAction::Trash;
    pack.rules[0].inspection_scope = Some(ReadOnlyScope::Entry);
    assert!(pack.validate().is_err());
    let old = r#"id="old"
name="Old pack"
version="1.0.0"
description="Old cache"
categories=["cache"]
[[rules]]
id="cache"
label="Cache"
category="cache"
match={dir_name="cache"}
confidence="medium"
default_selected=false
action="trash"
reason="Generated"
risk_note="Review"
"#;
    let old = RulePack::from_toml(old).unwrap();
    assert!(old.rules[0].read_only_scope().is_none());
    assert!(old.rules[0].matcher.exclude_path_globs.is_empty());
}

#[test]
fn cache_expansion_torchinductor_nested_cache_is_counted_once() {
    use cleanr_core::{
        RecommendationPolicy, RuntimeGuardState, SafetyPolicy, UserSelection,
        build_analysis_report, build_cleanup_plan_from_analysis,
    };
    let registry = RuleRegistry::builtin().unwrap();
    let now = Utc::now();
    let mut entries = vec![
        entry("/tmp/torchinductor_fixture", EntryKind::Directory),
        entry("/tmp/torchinductor_fixture/triton", EntryKind::Directory),
    ];
    entries[0].size_bytes = 40 * 1024 * 1024;
    for entry in &mut entries {
        entry.rule_hits = registry.hits_for_at_on_platform(entry, now, RulePlatform::Linux);
        for hit in &mut entry.rule_hits {
            if let Some(guard) = &mut hit.runtime_guard {
                guard.state = RuntimeGuardState::Idle;
            }
        }
    }
    let analysis = build_analysis_report(
        now,
        now,
        vec![PathBuf::from("/tmp")],
        &entries,
        &[],
        RecommendationPolicy::new(0).unwrap(),
    )
    .unwrap();
    let parent = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.local_path == entries[0].path)
        .unwrap();
    let selection = UserSelection {
        candidate_ids: [parent.id.clone()].into_iter().collect(),
    };
    let plan = build_cleanup_plan_from_analysis(
        vec![PathBuf::from("/tmp")],
        registry.versions(),
        &entries,
        &analysis,
        &selection,
        &SafetyPolicy::default(),
    )
    .unwrap();
    assert_eq!(plan.summary.selected_count, 1);
    assert_eq!(plan.summary.selected_size_bytes, 40 * 1024 * 1024);
    assert!(
        plan.items
            .iter()
            .filter(|item| item.selected)
            .all(|item| item.path == entries[0].path)
    );
}

#[test]
fn cache_expansion_disabling_cleanup_pack_cannot_remove_builtin_retention() {
    let mut config = cleanr_config::Config::default();
    config.plugins.dirs.clear();
    config.cleanup.enabled_rule_packs = vec!["builtin-system".into()];
    let mut registry = RuleRegistry::load(&config).unwrap();
    let retained = registry
        .packs
        .iter()
        .find(|pack| pack.definition.id == "builtin-dev")
        .unwrap();
    assert!(
        retained
            .definition
            .rules
            .iter()
            .all(|rule| rule.read_only_scope().is_some())
    );
    let mut plugin = retained.definition.clone();
    plugin.id = "broad-plugin".into();
    plugin.rules.truncate(1);
    let rule = &mut plugin.rules[0];
    rule.id = "dependencies".into();
    rule.action = crate::RuleAction::Trash;
    rule.inspection_scope = None;
    rule.matcher = crate::RuleMatcher {
        kind: Some(EntryKind::Directory),
        path_glob: Some("**/node_modules".into()),
        ..Default::default()
    };
    rule.confidence = Confidence::High;
    rule.default_selected = true;
    rule.platforms.clear();
    registry
        .add_pack(
            plugin,
            PluginSource::LegacyFile(PathBuf::from("/fixture/plugin.toml")),
            TrustLevel::Trusted,
            None,
        )
        .unwrap();
    for path in [
        "/u/Library/Caches/uv",
        "/u/Library/Caches/JetBrains/PyCharm2026.1/LocalHistory",
        "/u/.cache/huggingface/hub/node_modules",
    ] {
        let candidate = entry(path, EntryKind::Directory);
        let hits = registry.hits_for_at_on_platform(&candidate, Utc::now(), RulePlatform::Macos);
        assert!(
            hits.iter().any(|hit| hit.read_only_scope.is_some()),
            "must retain {path}"
        );
    }
}
