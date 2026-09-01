use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    time::{Duration, Instant},
};

use cleanr_core::ScanBudgetExceeded;
use cleanr_fs::{ScanOptions, ScanReport, scan_paths};

const ROOT_ENV: &str = "CLEANR_BENCH_ROOT";
const ROUNDS_ENV: &str = "CLEANR_BENCH_ROUNDS";
const WORKERS_ENV: &str = "CLEANR_BENCH_WORKERS";

#[test]
#[ignore = "manual local performance evidence; set CLEANR_BENCH_ROOT"]
fn scan_local_fixture() {
    let root = std::env::var_os(ROOT_ENV)
        .map(PathBuf::from)
        .expect("CLEANR_BENCH_ROOT must name a local benchmark root");
    let rounds = std::env::var(ROUNDS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5)
        .max(5);
    let workers = std::env::var(WORKERS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    let options = ScanOptions {
        workers,
        ..ScanOptions::default()
    };
    let mut samples = Vec::with_capacity(rounds);
    let mut rss_after_samples = Vec::with_capacity(rounds);
    let mut expected_report = None;

    for round in 1..=rounds {
        let started = Instant::now();
        let report = scan_paths(std::slice::from_ref(&root), &options).expect("benchmark scan");
        let elapsed = started.elapsed();
        let rss_after_kib = resident_set_kib();
        // Keep determinism evidence without retaining a second tree-sized report. Holding a full
        // clone here would contaminate the benchmark's resident-memory comparison.
        let fingerprint = report_fingerprint(&report);
        let deterministic_report = (report.summary.clone(), fingerprint);
        if let Some(expected) = &expected_report {
            assert_eq!(
                &deterministic_report, expected,
                "benchmark rounds must preserve deterministic reports"
            );
        } else {
            expected_report = Some(deterministic_report);
        }
        eprintln!(
            "cleanr-scan-benchmark round={round} workers={} elapsed_ms={} entries={} errors={} bytes={} fingerprint={fingerprint:016x} rss_after_kib={}",
            options.effective_workers(),
            elapsed.as_millis(),
            report.summary.entries_seen,
            report.summary.errors,
            report.summary.total_size_bytes,
            rss_after_kib.map_or_else(|| "unavailable".to_string(), |rss| rss.to_string()),
        );
        samples.push(elapsed);
        if let Some(rss_after_kib) = rss_after_kib {
            rss_after_samples.push(rss_after_kib);
        }
    }

    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let p95_rank = (samples.len() * 95).div_ceil(100);
    let p95 = samples[p95_rank.saturating_sub(1)];
    let total = samples.iter().copied().sum::<Duration>();
    let mean = total / u32::try_from(samples.len()).expect("benchmark round count fits u32");
    let peak_observed_rss_after_kib = rss_after_samples.iter().max().copied();
    let fingerprint = expected_report
        .as_ref()
        .map_or(0, |(_, fingerprint)| *fingerprint);
    eprintln!(
        "cleanr-scan-benchmark summary rounds={} workers={} median_ms={} p95_ms={} mean_ms={} fingerprint={fingerprint:016x} peak_observed_rss_after_kib={}",
        samples.len(),
        options.effective_workers(),
        median.as_millis(),
        p95.as_millis(),
        mean.as_millis(),
        peak_observed_rss_after_kib
            .map_or_else(|| "unavailable".to_string(), |rss| rss.to_string()),
    );
}

fn report_fingerprint(report: &ScanReport) -> u64 {
    let mut hasher = DefaultHasher::new();
    report.summary.roots.hash(&mut hasher);
    report.summary.entries_seen.hash(&mut hasher);
    report.summary.errors.hash(&mut hasher);
    report.summary.total_size_bytes.hash(&mut hasher);
    for entry in &report.entries {
        entry.path.hash(&mut hasher);
        std::mem::discriminant(&entry.kind).hash(&mut hasher);
        entry.size_bytes.hash(&mut hasher);
        entry.modified_at.hash(&mut hasher);
        entry.rule_hits.len().hash(&mut hasher);
    }
    for issue in &report.issues {
        std::mem::discriminant(&issue.code).hash(&mut hasher);
        issue.path.hash(&mut hasher);
    }
    for error in &report.errors {
        error.path.hash(&mut hasher);
        error.message.hash(&mut hasher);
    }
    report.completed_roots.hash(&mut hasher);
    for exceeded in &report.budget_exceeded {
        hash_budget_exceeded(exceeded, &mut hasher);
    }
    hasher.finish()
}

fn hash_budget_exceeded(exceeded: &ScanBudgetExceeded, hasher: &mut impl Hasher) {
    std::mem::discriminant(exceeded).hash(hasher);
    match exceeded {
        ScanBudgetExceeded::EntryCount { limit, observed }
        | ScanBudgetExceeded::IssueDetails { limit, observed } => {
            limit.hash(hasher);
            observed.hash(hasher);
        }
        ScanBudgetExceeded::ElapsedTime {
            limit_millis,
            observed_millis,
        } => {
            limit_millis.hash(hasher);
            observed_millis.hash(hasher);
        }
        ScanBudgetExceeded::EstimatedMemory {
            limit_bytes,
            observed_bytes,
        } => {
            limit_bytes.hash(hasher);
            observed_bytes.hash(hasher);
        }
    }
}

#[cfg(target_os = "linux")]
fn resident_set_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn resident_set_kib() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

#[cfg(not(unix))]
fn resident_set_kib() -> Option<u64> {
    None
}
