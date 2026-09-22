use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use cleanr_core::{
    AnalysisScanContext, GlobalScanKind, ReadOnlyScope, RecommendationPolicy, RecommendationState,
    RulePlatform, SafetyPolicy, ScanRequest, UserSelection,
    build_analysis_report_with_scan_context, build_cleanup_plan_from_analysis,
};
use cleanr_fs::{
    GlobalScanEnvironment, ScanOptions, global_scan_evidence,
    resolve_scan_roots_with_env_and_locations, scan_paths, scan_resolved_paths,
};
use cleanr_rules::RuleRegistry;

use crate::runtime::{ProcessSnapshot, resolve_runtime_guards};

fn payload(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = fs::File::create(&path).unwrap();
    file.set_len(2 * 1024 * 1024).unwrap();
    path
}

#[test]
fn cache_expansion_discovery_covers_every_declared_platform_without_user_data() {
    let fixture = tempfile::tempdir().unwrap();
    let registry = RuleRegistry::builtin().unwrap();
    for platform in [
        RulePlatform::Macos,
        RulePlatform::Linux,
        RulePlatform::Windows,
    ] {
        let root = fixture.path().join(format!("{platform:?}"));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("home");
        let cache = match platform {
            RulePlatform::Macos => home.join("Library/Caches"),
            RulePlatform::Linux => home.join(".cache"),
            RulePlatform::Windows => home.join("AppData/Local"),
        };
        let local = home.join("AppData/Local");
        let temp = root.join("tmp");
        let env = GlobalScanEnvironment {
            home_dir: Some(home.clone()),
            cache_dir: Some(cache.clone()),
            data_local_dir: Some(local.clone()),
            temp_dir: Some(temp.clone()),
            ..Default::default()
        };
        fs::create_dir_all(&temp).unwrap();
        for dir in [
            home.join(".cache/puppeteer"),
            home.join(".cache/uv"),
            home.join(".cache/huggingface/xet/environment/chunk_cache"),
            home.join("miniforge3/pkgs"),
            cache.join("ms-playwright"),
            cache.join("JetBrains/PyCharm2026.1/index"),
            cache.join("JetBrains/PyCharm2026.1/log"),
        ] {
            fs::create_dir_all(dir).unwrap();
        }
        let extras = match platform {
            RulePlatform::Macos => vec![
                cache.join("electron"),
                cache.join("Cypress"),
                cache.join("Mozilla.sccache"),
                home.join("Library/Logs/JetBrains/PyCharm2026.1"),
            ],
            RulePlatform::Linux => vec![
                cache.join("electron"),
                cache.join("Cypress"),
                cache.join("sccache"),
                cache.join("go-build"),
                temp.join("torchinductor_fixture"),
            ],
            RulePlatform::Windows => vec![
                local.join("electron/Cache"),
                local.join("Cypress/Cache"),
                local.join("Mozilla/sccache"),
                local.join("go-build"),
                local.join("pip/Cache"),
                local.join("uv/cache"),
            ],
        };
        for dir in &extras {
            fs::create_dir_all(dir).unwrap();
        }
        let request =
            ScanRequest::global(vec![GlobalScanKind::DeveloperCaches, GlobalScanKind::Logs]);
        let resolved = resolve_scan_roots_with_env_and_locations(
            &request,
            &[],
            &env,
            registry.scan_locations(),
            Some(platform),
        )
        .unwrap();
        assert!(resolved.issues.is_empty());
        let expected = [
            home.join(".cache/puppeteer"),
            home.join(".cache/huggingface/xet"),
            home.join("miniforge3/pkgs"),
            cache.join("ms-playwright"),
            cache.join("JetBrains"),
        ];
        for path in expected.into_iter().chain(extras) {
            if !cfg!(unix) && path.starts_with(&temp) {
                continue;
            }
            let canonical = path.canonicalize().unwrap();
            assert!(
                resolved
                    .global_locations
                    .iter()
                    .any(|location| canonical == location.path
                        || canonical.starts_with(&location.path)),
                "undiscovered {platform:?}: {}",
                path.display()
            );
            assert!(
                resolved
                    .roots
                    .iter()
                    .any(|root| canonical.starts_with(root)),
                "not traversed {}",
                path.display()
            );
        }
        assert!(
            resolved
                .global_locations
                .iter()
                .all(|location| location.path.starts_with(&root))
        );
        let report = scan_resolved_paths(&resolved, &ScanOptions::default()).unwrap();
        assert!(report.issues.is_empty());
        let evidence = global_scan_evidence(&request, &[], &resolved, &report.summary.roots);
        assert!(
            evidence
                .locations
                .iter()
                .any(|location| location.kind == GlobalScanKind::Logs)
        );
    }
}

#[test]
fn cache_expansion_real_scan_keeps_retained_data_out_of_all_plan_entry_points() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    for path in [
        ".cache/puppeteer/browser.bin",
        ".cache/uv/archive-v0/node_modules/dependency.bin",
        ".cache/huggingface/xet/environment/chunk_cache/chunks.bin",
        ".cache/huggingface/xet/environment/staging/node_modules/upload.bin",
        ".cache/huggingface/hub/node_modules/model.bin",
        "miniforge3/pkgs/package-1.0.conda",
        "miniforge3/pkgs/package-2.0.tar.bz2",
        "miniforge3/pkgs/fake.conda/node_modules/retained.bin",
        "miniforge3/envs/env/node_modules/retained.bin",
    ] {
        payload(&root, path);
    }
    fs::write(
        root.join("miniforge3/pkgs/urls.txt"),
        "https://example.invalid/package.conda\n",
    )
    .unwrap();
    let cache_prefix = if cfg!(target_os = "macos") {
        "Library/Caches"
    } else if cfg!(windows) {
        "AppData/Local"
    } else {
        ".cache"
    };
    for relative in [
        "ms-playwright/browser.bin",
        "JetBrains/PyCharm2026.1/index/data.bin",
        "JetBrains/PyCharm2026.1/LocalHistory/node_modules/history.bin",
        "JetBrains/PyCharm2026.1/new-state/node_modules/data.bin",
    ] {
        payload(&root, &format!("{cache_prefix}/{relative}"));
    }
    let registry = RuleRegistry::builtin().unwrap();
    let mut scan = scan_paths(std::slice::from_ref(&root), &ScanOptions::default()).unwrap();
    assert!(scan.issues.is_empty());
    registry.annotate_entries_at(&mut scan.entries, scan.as_of);
    resolve_runtime_guards(&mut scan.entries, &ProcessSnapshot::from_names(&[]));
    let policy = SafetyPolicy::default();
    let mut analysis = build_analysis_report_with_scan_context(
        scan.as_of,
        Utc::now(),
        scan.summary.roots.clone(),
        &scan.entries,
        &scan.issues,
        RecommendationPolicy::new(0).unwrap(),
        AnalysisScanContext {
            safety_policy: Some(&policy),
            explicit_roots: std::slice::from_ref(&root),
            ..Default::default()
        },
    )
    .unwrap();
    let expected = [
        root.join(".cache/puppeteer"),
        root.join(".cache/huggingface/xet/environment/chunk_cache"),
        root.join("miniforge3/pkgs/package-1.0.conda"),
        root.join("miniforge3/pkgs/package-2.0.tar.bz2"),
        root.join(format!("{cache_prefix}/ms-playwright")),
        root.join(format!("{cache_prefix}/JetBrains/PyCharm2026.1/index")),
    ];
    for path in &expected {
        let candidate = analysis
            .candidates
            .iter()
            .find(|candidate| &candidate.local_path == path)
            .unwrap_or_else(|| panic!("missing {}", path.display()));
        assert_eq!(
            candidate.recommendation.state,
            RecommendationState::Review,
            "{}",
            path.display()
        );
        assert!(!candidate.recommendation.initial_selected);
    }
    for candidate in &analysis.candidates {
        if candidate
            .rules
            .matched
            .iter()
            .any(|rule| rule.read_only_scope.is_some())
        {
            assert_eq!(
                candidate.recommendation.state,
                RecommendationState::Excluded
            );
        }
    }
    // Deliberately tamper with recommendation state and selection. Independent protection indexes
    // still reject the retained entries, their ancestors and candidates inside retained subtrees.
    for candidate in &mut analysis.candidates {
        candidate.recommendation.state = RecommendationState::Review;
    }
    let selection = UserSelection {
        candidate_ids: analysis
            .candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect(),
    };
    let plan = build_cleanup_plan_from_analysis(
        scan.summary.roots.clone(),
        registry.versions(),
        &scan.entries,
        &analysis,
        &selection,
        &policy,
    )
    .unwrap();
    let mut selected = plan
        .items
        .iter()
        .filter(|item| item.selected)
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();
    selected.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(selected, expected);
    crate::validate_recoverable_plan(&plan).unwrap();
    #[allow(deprecated)]
    let legacy = cleanr_core::build_cleanup_plan_with_policy(
        scan.summary.roots.clone(),
        registry.versions(),
        &scan.entries,
        &policy,
    );
    assert!(
        legacy
            .items
            .iter()
            .all(|item| expected.contains(&item.path)),
        "legacy builder must preserve the same exclusions"
    );
    // If the caller only supplies analysis, retained rule evidence still protects the plan.
    let mut stripped_entries = scan.entries.clone();
    for entry in &mut stripped_entries {
        entry.rule_hits.clear();
    }
    let from_analysis = build_cleanup_plan_from_analysis(
        scan.summary.roots,
        registry.versions(),
        &stripped_entries,
        &analysis,
        &selection,
        &policy,
    )
    .unwrap();
    assert!(
        from_analysis
            .items
            .iter()
            .all(|item| expected.contains(&item.path))
    );
    assert!(scan.entries.iter().any(|entry| {
        entry
            .rule_hits
            .iter()
            .any(|hit| hit.read_only_scope == Some(ReadOnlyScope::Subtree))
    }));
}

#[test]
fn cache_expansion_active_or_unknown_process_blocks_cache_and_ancestors() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    payload(&root, "node_modules/.cache/puppeteer/browser.bin");
    let registry = RuleRegistry::builtin().unwrap();
    let mut scan = scan_paths(std::slice::from_ref(&root), &ScanOptions::default()).unwrap();
    registry.annotate_entries(&mut scan.entries);
    for snapshot in [
        ProcessSnapshot::from_names(&["Python3.13t.EXE"]),
        ProcessSnapshot::unavailable(),
    ] {
        resolve_runtime_guards(&mut scan.entries, &snapshot);
        let analysis = build_analysis_report_with_scan_context(
            scan.as_of,
            Utc::now(),
            scan.summary.roots.clone(),
            &scan.entries,
            &scan.issues,
            RecommendationPolicy::new(0).unwrap(),
            AnalysisScanContext::default(),
        )
        .unwrap();
        let candidate = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.local_path.ends_with("puppeteer"))
            .unwrap();
        assert_eq!(
            candidate.recommendation.state,
            RecommendationState::Excluded
        );
        let parent = analysis
            .candidates
            .iter()
            .find(|candidate| candidate.local_path.ends_with("node_modules"))
            .unwrap();
        assert_eq!(parent.recommendation.state, RecommendationState::Excluded);
        let selection = UserSelection {
            candidate_ids: analysis
                .candidates
                .iter()
                .map(|candidate| candidate.id.clone())
                .collect(),
        };
        let plan = build_cleanup_plan_from_analysis(
            scan.summary.roots.clone(),
            registry.versions(),
            &scan.entries,
            &analysis,
            &selection,
            &SafetyPolicy::default(),
        )
        .unwrap();
        assert_eq!(plan.summary.selected_count, 0);
    }
}

#[test]
fn cache_expansion_executor_rejects_selected_inspections_before_journal_or_trash() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let path = root.join("cache");
    fs::create_dir(&path).unwrap();
    let mut entry = crate::tests::cleanup_entry(path.clone(), cleanr_core::EntryKind::Directory, 0);
    entry.modified_at = Some(Utc::now());
    let analysis = build_analysis_report_with_scan_context(
        Utc::now(),
        Utc::now(),
        vec![root.clone()],
        std::slice::from_ref(&entry),
        &[],
        RecommendationPolicy::new(0).unwrap(),
        AnalysisScanContext::default(),
    )
    .unwrap();
    let selection = UserSelection::from_recommendations(&analysis);
    let mut plan = build_cleanup_plan_from_analysis(
        vec![root.clone()],
        vec![],
        &[entry],
        &analysis,
        &selection,
        &SafetyPolicy::default(),
    )
    .unwrap();
    assert_eq!(plan.summary.selected_count, 1);
    // Selected inspection evidence is invalid even when it is marked shadowed.
    for shadowed in [false, true] {
        let evidence = plan.items[0].evidence.as_mut().unwrap();
        evidence.matched_rules[0].read_only_scope = Some(ReadOnlyScope::Subtree);
        if shadowed {
            evidence
                .shadowed_rules
                .push(evidence.matched_rules[0].key.clone());
        }
        let executor = crate::FakeTrashExecutor::default();
        let state = root.join("state");
        let error = crate::execute_cleanup_plan(
            &plan,
            &executor,
            &state,
            Some(&crate::CleanupAuthorization::explicit_user_confirmation()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("read-only inspection"));
        assert!(executor.trashed_paths().is_empty());
        assert!(!state.exists(), "no journal before validation");
        assert!(path.exists());
    }
}

#[cfg(not(windows))]
#[test]
fn cache_expansion_global_category_keeps_developer_and_log_candidates_separate() {
    let fixture = tempfile::tempdir().unwrap();
    let home = fixture.path().canonicalize().unwrap();
    let cache = home.join(if cfg!(target_os = "macos") {
        "Library/Caches"
    } else {
        ".cache"
    });
    payload(&cache, "ms-playwright/browser.bin");
    payload(&cache, "JetBrains/PyCharm2026.1/index/data.bin");
    let log = if cfg!(target_os = "macos") {
        home.join("Library/Logs/JetBrains/PyCharm2026.1")
    } else {
        cache.join("JetBrains/PyCharm2026.1/log")
    };
    payload(&log, "idea.log");
    let registry = RuleRegistry::builtin().unwrap();
    let env = GlobalScanEnvironment {
        home_dir: Some(home),
        cache_dir: Some(cache),
        ..Default::default()
    };
    for kind in [
        GlobalScanKind::AppCaches,
        GlobalScanKind::DeveloperCaches,
        GlobalScanKind::Logs,
    ] {
        let request = ScanRequest::global(vec![kind]);
        let resolved = resolve_scan_roots_with_env_and_locations(
            &request,
            &[],
            &env,
            registry.scan_locations(),
            RulePlatform::current(),
        )
        .unwrap();
        let mut scan = scan_resolved_paths(&resolved, &ScanOptions::default()).unwrap();
        assert!(scan.issues.is_empty());
        let global = global_scan_evidence(&request, &[], &resolved, &scan.summary.roots);
        registry.annotate_entries_at(&mut scan.entries, scan.as_of);
        resolve_runtime_guards(&mut scan.entries, &ProcessSnapshot::from_names(&[]));
        let analysis = build_analysis_report_with_scan_context(
            scan.as_of,
            Utc::now(),
            scan.summary.roots.clone(),
            &scan.entries,
            &scan.issues,
            RecommendationPolicy::new(0).unwrap(),
            AnalysisScanContext {
                global: Some(&global),
                ..Default::default()
            },
        )
        .unwrap();
        let selection = UserSelection {
            candidate_ids: analysis
                .candidates
                .iter()
                .map(|candidate| candidate.id.clone())
                .collect(),
        };
        let plan = crate::build_workflow_plan(
            scan.summary.roots,
            registry.versions(),
            &scan.entries,
            &analysis,
            &selection,
            &SafetyPolicy::default(),
            &[],
            &global,
        )
        .unwrap();
        let chosen = plan
            .items
            .iter()
            .filter(|item| item.selected)
            .collect::<Vec<_>>();
        match kind {
            GlobalScanKind::AppCaches => assert!(chosen.is_empty()),
            GlobalScanKind::DeveloperCaches => {
                assert_eq!(chosen.len(), 2);
                assert!(chosen.iter().all(
                    |item| item.path.ends_with("index") || item.path.ends_with("ms-playwright")
                ));
            }
            GlobalScanKind::Logs => {
                assert_eq!(chosen.len(), 1);
                assert_eq!(chosen[0].path, log);
                crate::validate_recoverable_plan(&plan).unwrap();
            }
            _ => unreachable!(),
        }
    }
}
