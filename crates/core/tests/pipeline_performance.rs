use std::{path::PathBuf, time::Instant};

use chrono::{Duration, Utc};
use cleanr_core::{
    Confidence, EntryKind, RecommendationPolicy, RuleHit, RuleMatchRole, RuleTrust, RulesetVersion,
    SafetyPolicy, ScanEntry, UserSelection, build_analysis_report_with_safety_policy,
    build_cleanup_plan_from_analysis,
};

const ENTRIES_ENV: &str = "CLEANR_BENCH_ENTRIES";
const ROUNDS_ENV: &str = "CLEANR_BENCH_ROUNDS";
const FILES_PER_CANDIDATE_ENV: &str = "CLEANR_BENCH_FILES_PER_CANDIDATE";

#[test]
#[ignore = "manual synthetic pipeline performance evidence"]
fn analysis_and_plan_pipeline() {
    let requested_entries = env_usize(ENTRIES_ENV, 100_000).max(101);
    let rounds = env_usize(ROUNDS_ENV, 3).max(1);
    let files_per_candidate = env_usize(FILES_PER_CANDIDATE_ENV, 99);
    let root = PathBuf::from("cleanr-synthetic-benchmark");
    let entries = synthetic_entries(&root, requested_entries, files_per_candidate);
    let candidate_count = entries
        .iter()
        .filter(|entry| !entry.rule_hits.is_empty())
        .count();
    let safety = SafetyPolicy::new(Vec::new(), true);
    let policy = RecommendationPolicy::default();
    let versions = vec![RulesetVersion {
        id: "synthetic".to_string(),
        version: "1".to_string(),
        sources: Vec::new(),
    }];

    let mut analysis_samples = Vec::with_capacity(rounds);
    let mut plan_samples = Vec::with_capacity(rounds);
    let mut serialization_samples = Vec::with_capacity(rounds);

    for round in 1..=rounds {
        let as_of = Utc::now();
        let analysis_started = Instant::now();
        let analysis = build_analysis_report_with_safety_policy(
            as_of,
            as_of,
            vec![root.clone()],
            &entries,
            &[],
            policy.clone(),
            &safety,
        )
        .expect("synthetic analysis");
        let analysis_elapsed = analysis_started.elapsed();
        assert_eq!(analysis.candidates.len(), candidate_count);

        let selection = UserSelection::from_recommendations(&analysis);
        let plan_started = Instant::now();
        let plan = build_cleanup_plan_from_analysis(
            vec![root.clone()],
            versions.clone(),
            &entries,
            &analysis,
            &selection,
            &safety,
        )
        .expect("supported complete analysis");
        let plan_elapsed = plan_started.elapsed();
        assert_eq!(plan.items.len(), candidate_count);

        let serialization_started = Instant::now();
        let encoded = serde_json::to_vec(&analysis).expect("serialize synthetic analysis");
        let serialization_elapsed = serialization_started.elapsed();
        assert!(!encoded.is_empty());

        eprintln!(
            "cleanr-pipeline-benchmark round={round} entries={} candidates={} files_per_candidate={} analysis_ms={} plan_ms={} serialize_ms={} encoded_bytes={}",
            entries.len(),
            candidate_count,
            files_per_candidate,
            analysis_elapsed.as_millis(),
            plan_elapsed.as_millis(),
            serialization_elapsed.as_millis(),
            encoded.len(),
        );
        analysis_samples.push(analysis_elapsed.as_millis());
        plan_samples.push(plan_elapsed.as_millis());
        serialization_samples.push(serialization_elapsed.as_millis());
    }

    analysis_samples.sort_unstable();
    plan_samples.sort_unstable();
    serialization_samples.sort_unstable();
    eprintln!(
        "cleanr-pipeline-benchmark summary rounds={} analysis_median_ms={} plan_median_ms={} serialize_median_ms={}",
        rounds,
        analysis_samples[rounds / 2],
        plan_samples[rounds / 2],
        serialization_samples[rounds / 2],
    );
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn synthetic_entries(
    root: &std::path::Path,
    requested_entries: usize,
    files_per_candidate: usize,
) -> Vec<ScanEntry> {
    let candidates = (requested_entries - 1) / (files_per_candidate + 1);
    let modified_at = Utc::now() - Duration::days(120);
    let mut entries = Vec::with_capacity(1 + candidates * (files_per_candidate + 1));
    entries.push(ScanEntry {
        path: root.to_path_buf(),
        kind: EntryKind::Directory,
        size_bytes: 0,
        modified_at: Some(modified_at),
        rule_hits: Vec::new(),
    });

    for candidate_idx in 0..candidates {
        let candidate = root.join(format!("candidate-{candidate_idx:06}"));
        entries.push(ScanEntry {
            path: candidate.clone(),
            kind: EntryKind::Directory,
            size_bytes: files_per_candidate as u64,
            modified_at: Some(modified_at),
            rule_hits: vec![RuleHit {
                rule_pack_id: "synthetic".to_string(),
                rule_id: "candidate".to_string(),
                label: "Synthetic candidate".to_string(),
                category: "benchmark".to_string(),
                confidence: Confidence::High,
                reason: "Synthetic local performance fixture".to_string(),
                risk_note: "Benchmark only".to_string(),
                default_selected: true,
                trust: RuleTrust::Builtin,
                match_role: RuleMatchRole::Primary,
                sources: Vec::new(),
                runtime_guard: None,
            }],
        });
        for file_idx in 0..files_per_candidate {
            entries.push(ScanEntry {
                path: candidate.join(format!("file-{file_idx:03}")),
                kind: EntryKind::File,
                size_bytes: 1,
                modified_at: Some(modified_at),
                rule_hits: Vec::new(),
            });
        }
    }

    // The production reducer must not depend on traversal order.
    entries.reverse();
    entries
}
