// These fixtures exercise the executor boundary without a filesystem scan.
#![allow(deprecated)]

use std::{
    cell::Cell,
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
};

use anyhow::Result;
use cleanr_core::{
    EntryKind, ExecutionManifest, ExecutionStatus, RestoreStatus, RollbackReceipt,
    build_cleanup_plan,
};

use crate::{
    CleanupAuthorization, CleanupExecutor, FakeRestoreExecutor, FakeTrashExecutor,
    ManifestRepository, RestoreExecutor, execute_cleanup_plan, restore_execution_manifest,
    restored_run_ids,
    tests::{cleanup_entry, restorable_manifest},
};

struct RestoreWith<F>(F);

impl<F: Fn(&Path) -> Result<()>> RestoreExecutor for RestoreWith<F> {
    fn restore(&self, path: &Path, _: &RollbackReceipt, _: i64) -> Result<()> {
        (self.0)(path)
    }
}

struct CleanupWith<F>(F);

impl<F: Fn(&Path) -> Result<RollbackReceipt>> CleanupExecutor for CleanupWith<F> {
    fn trash(&self, path: &Path) -> Result<RollbackReceipt> {
        (self.0)(path)
    }
}

fn sample_manifest(root: &Path, count: usize) -> ExecutionManifest {
    let mut manifest = restorable_manifest("recovery-test", root.join("item-0"));
    for index in 1..count {
        let mut item = manifest.items[0].clone();
        item.path = root.join(format!("item-{index}"));
        manifest.items.push(item);
    }
    manifest.summary.attempted = count;
    manifest.summary.succeeded = count;
    manifest
}

// Keep the last durable JSON intact, but block its atomic replacement with a
// directory. This simulates a write failure without permissions or real Trash.
fn block_journal_write(directory: &Path) -> Result<(PathBuf, PathBuf)> {
    let journal = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .expect("one journal exists before the operation");
    let saved = journal.with_extension("saved");
    fs::rename(&journal, &saved)?;
    fs::create_dir(&journal)?;
    Ok((journal, saved))
}

fn unblock_journal(directory: &Path) -> Result<()> {
    let saved = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "saved")
        })
        .expect("the last durable journal was preserved");
    let journal = saved.with_extension("json");
    fs::remove_dir(&journal)?;
    fs::rename(saved, journal)?;
    Ok(())
}

#[test]
fn restore_journals_each_item_and_continues_after_an_executor_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = sample_manifest(temp.path(), 3);
    let repository = ManifestRepository::new(temp.path());
    let calls = Cell::new(0);
    let executor = RestoreWith(|_: &Path| {
        let index = calls.get();
        calls.set(index + 1);
        let history = repository.list_restores()?;
        let journal = &history[0];
        assert_eq!(journal.items[index].status, RestoreStatus::Pending);
        assert_eq!(journal.summary.pending, 1);
        assert_eq!(journal.summary.not_attempted, 2 - index);
        if index > 0 {
            assert_eq!(journal.items[0].status, RestoreStatus::Restored);
        }
        if index == 1 {
            anyhow::bail!("simulated restore failure");
        }
        if index == 2 {
            assert_eq!(journal.items[1].status, RestoreStatus::Failed);
        }
        Ok(())
    });
    let result = restore_execution_manifest(&manifest, &executor, temp.path()).expect("restore");
    assert_eq!(calls.get(), 3);
    assert_eq!(result.summary.succeeded, 2);
    assert_eq!(result.summary.failed, 1);
    assert_eq!(result.summary.pending, 0);
    assert_eq!(repository.list_restores().expect("history"), vec![result]);
}

#[test]
fn interrupted_restore_blocks_unknown_item_but_resumes_unattempted_items() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = sample_manifest(temp.path(), 2);
    let executor = RestoreWith(|path: &Path| {
        fs::write(path, b"restored before interruption")?;
        panic!("simulate interruption after filesystem operation");
    });
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            restore_execution_manifest(&manifest, &executor, temp.path())
        }))
        .is_err()
    );
    let history = ManifestRepository::new(temp.path())
        .list_restores()
        .expect("history");
    assert_eq!(history[0].items[0].status, RestoreStatus::Pending);
    assert_eq!(history[0].items[1].status, RestoreStatus::NotAttempted);
    assert!(restored_run_ids(&history).is_empty());

    let restarted = FakeRestoreExecutor::default();
    let result = restore_execution_manifest(&manifest, &restarted, temp.path()).expect("restart");
    assert_eq!(result.items[0].status, RestoreStatus::Failed);
    assert!(
        result.items[0]
            .error
            .as_deref()
            .expect("error")
            .contains("automatic retry is blocked")
    );
    assert_eq!(
        restarted.restored_paths(),
        vec![manifest.items[1].path.clone()]
    );
    assert_eq!(
        fs::read(&manifest.items[0].path).expect("original preserved"),
        b"restored before interruption"
    );
}

#[test]
fn successful_restore_with_failed_journal_write_stops_and_blocks_retry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = sample_manifest(temp.path(), 2);
    let calls = Cell::new(0);
    let executor = RestoreWith(|path: &Path| {
        calls.set(calls.get() + 1);
        fs::write(path, b"restored before write failure")?;
        block_journal_write(&temp.path().join("restores"))?;
        Ok(())
    });
    let error =
        restore_execution_manifest(&manifest, &executor, temp.path()).expect_err("write fails");
    assert!(error.to_string().contains("executor reported Restored"));
    assert!(
        error
            .to_string()
            .contains("no further items were attempted")
    );
    assert_eq!(calls.get(), 1);
    unblock_journal(&temp.path().join("restores")).expect("restore journal storage");

    let restarted = FakeRestoreExecutor::default();
    let result = restore_execution_manifest(&manifest, &restarted, temp.path()).expect("restart");
    assert_eq!(result.summary.failed, 1);
    assert_eq!(
        restarted.restored_paths(),
        vec![manifest.items[1].path.clone()]
    );
    assert_eq!(
        fs::read(&manifest.items[0].path).expect("original preserved"),
        b"restored before write failure"
    );
}

#[test]
fn successful_cleanup_with_failed_journal_write_preserves_pending_and_locator_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = temp.path().join("first.bin");
    let second = temp.path().join("second.bin");
    fs::write(&first, b"one").expect("first");
    fs::write(&second, b"two").expect("second");
    let state = temp.path().join("state");
    let plan = build_cleanup_plan(
        vec![temp.path().to_path_buf()],
        vec![],
        &[
            cleanup_entry(first.clone(), EntryKind::File, 3),
            cleanup_entry(second.clone(), EntryKind::File, 3),
        ],
    );
    let isolated_destination = temp.path().join("isolated-recovery-item");
    let calls = Cell::new(0);
    let executor = CleanupWith(|path: &Path| {
        calls.set(calls.get() + 1);
        fs::rename(path, &isolated_destination)?;
        block_journal_write(&state.join("runs"))?;
        Ok(RollbackReceipt {
            method: "fake-trash".to_string(),
            note: "isolated test destination".to_string(),
            locator: Some("test-recovery-locator".to_string()),
        })
    });
    let error = execute_cleanup_plan(
        &plan,
        &executor,
        &state,
        Some(&CleanupAuthorization::explicit_user_confirmation()),
    )
    .expect_err("write fails");
    assert!(error.to_string().contains("test-recovery-locator"));
    assert_eq!(calls.get(), 1);
    assert_eq!(
        fs::read(&isolated_destination).expect("recoverable isolated item"),
        b"one"
    );
    assert_eq!(fs::read(second).expect("second was not attempted"), b"two");
    unblock_journal(&state.join("runs")).expect("restore journal storage");
    let history = ManifestRepository::new(&state)
        .list_executions()
        .expect("history");
    assert_eq!(history[0].items[0].status, ExecutionStatus::Pending);
    assert_eq!(history[0].items[1].status, ExecutionStatus::Skipped);
    let fake = FakeRestoreExecutor::default();
    let result = restore_execution_manifest(&history[0], &fake, &state).expect("restore review");
    assert_eq!(result.summary.failed, 1);
    assert!(
        result.items[0]
            .error
            .as_deref()
            .expect("error")
            .contains("cleanup outcome was not recorded")
    );
    assert!(fake.restored_paths().is_empty());
}

#[test]
fn operation_lock_blocks_reentrant_restore_and_cleanup_until_released() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ManifestRepository::new(temp.path());
    let held = repository.lock_operations().expect("operation lock");
    let manifest = sample_manifest(temp.path(), 1);
    let restore = FakeRestoreExecutor::default();
    assert!(
        restore_execution_manifest(&manifest, &restore, temp.path())
            .expect_err("lock held")
            .to_string()
            .contains("could not lock")
    );
    assert!(restore.restored_paths().is_empty());
    let target = temp.path().join("target");
    fs::write(&target, b"one").expect("target");
    let plan = build_cleanup_plan(
        vec![temp.path().to_path_buf()],
        vec![],
        &[cleanup_entry(target, EntryKind::File, 3)],
    );
    let cleanup = FakeTrashExecutor::default();
    assert!(
        execute_cleanup_plan(
            &plan,
            &cleanup,
            temp.path(),
            Some(&CleanupAuthorization::explicit_user_confirmation())
        )
        .is_err()
    );
    assert!(cleanup.trashed_paths().is_empty());
    assert!(!temp.path().join("runs").exists());
    assert!(!temp.path().join("restores").exists());
    drop(held);
    assert_eq!(
        restore_execution_manifest(&manifest, &restore, temp.path())
            .expect("lock released")
            .summary
            .succeeded,
        1
    );
}

#[test]
fn operation_lock_is_shared_across_processes_and_released_on_exit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repository = ManifestRepository::new(temp.path());
    let held = repository.lock_operations().expect("parent lock");
    let child = |expect_blocked: bool| {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "recovery_tests::operation_lock_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("CLEANR_OPERATION_LOCK_TEST_STATE", temp.path())
            .env(
                "CLEANR_OPERATION_LOCK_TEST_BLOCKED",
                if expect_blocked { "1" } else { "0" },
            )
            .output()
            .expect("run lock helper process");
        assert!(
            output.status.success(),
            "lock helper failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    };
    child(true);
    drop(held);
    child(false);
    repository
        .lock_operations()
        .expect("child exit released lock");
}

#[test]
#[ignore = "subprocess helper invoked by the isolated operation-lock test"]
fn operation_lock_child_fixture() {
    let Some(state) = std::env::var_os("CLEANR_OPERATION_LOCK_TEST_STATE") else {
        return;
    };
    let result = ManifestRepository::new(state).lock_operations();
    if std::env::var("CLEANR_OPERATION_LOCK_TEST_BLOCKED").expect("helper mode") == "1" {
        assert!(result.is_err(), "parent process must exclude this process");
    } else {
        // Leak the handle so the parent verifies OS cleanup on process exit,
        // rather than merely testing File's normal destructor.
        std::mem::forget(result.expect("parent released its lock"));
    }
}

#[test]
fn unreadable_history_and_unwritable_state_fail_before_restore_executor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = sample_manifest(temp.path(), 1);
    let executor = FakeRestoreExecutor::default();
    assert!(restore_execution_manifest(&manifest, &executor, Path::new("")).is_err());
    let state_file = temp.path().join("state-file");
    fs::write(&state_file, b"not a directory").expect("state file");
    assert!(restore_execution_manifest(&manifest, &executor, &state_file).is_err());
    let state = temp.path().join("state");
    fs::create_dir(&state).expect("state");
    fs::write(state.join("restores"), b"not a history directory").expect("blocked history");
    assert!(
        restore_execution_manifest(&manifest, &executor, &state)
            .expect_err("history unavailable")
            .to_string()
            .contains("failed to read manifest history")
    );
    assert!(executor.restored_paths().is_empty());
}

#[test]
fn legacy_restore_v1_history_still_prevents_duplicate_restore() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = sample_manifest(temp.path(), 1);
    let first = restore_execution_manifest(&manifest, &FakeRestoreExecutor::default(), temp.path())
        .expect("first restore");
    let mut legacy = serde_json::to_value(&first).expect("json");
    legacy["schema_version"] = "cleanr.restore.v1".into();
    legacy["summary"]
        .as_object_mut()
        .expect("summary")
        .remove("pending");
    legacy["summary"]
        .as_object_mut()
        .expect("summary")
        .remove("not_attempted");
    fs::write(
        temp.path()
            .join("restores")
            .join(format!("{}.json", first.restore_id)),
        serde_json::to_vec(&legacy).expect("legacy bytes"),
    )
    .expect("legacy fixture");
    let history = ManifestRepository::new(temp.path())
        .list_restores()
        .expect("v1 history");
    assert_eq!(history[0].summary.pending, 0);
    let restarted = FakeRestoreExecutor::default();
    assert_eq!(
        restore_execution_manifest(&manifest, &restarted, temp.path())
            .expect("v1 replay")
            .summary
            .attempted,
        0
    );
    assert!(restarted.restored_paths().is_empty());
}

#[test]
fn unknown_restore_history_schema_blocks_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = sample_manifest(temp.path(), 1);
    let mut restore =
        restore_execution_manifest(&manifest, &FakeRestoreExecutor::default(), temp.path())
            .expect("restore fixture");
    restore.schema_version = "cleanr.restore.v999".to_string();
    ManifestRepository::new(temp.path())
        .write_restore(&restore)
        .expect("unknown schema fixture");
    let executor = FakeRestoreExecutor::default();
    assert!(
        restore_execution_manifest(&manifest, &executor, temp.path())
            .expect_err("unknown history cannot be assumed safe")
            .to_string()
            .contains("unsupported restore manifest schema")
    );
    assert!(executor.restored_paths().is_empty());
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
#[test]
#[ignore = "explicit platform acceptance only: moves one test-owned directory through system Trash"]
fn system_trash_round_trip_records_receipt() {
    let temp = std::env::var_os("CLEANR_SYSTEM_TRASH_TEST_PARENT").map_or_else(
        || tempfile::tempdir().expect("tempdir"),
        |parent| {
            tempfile::Builder::new()
                .prefix(".cleanr-trash-round-trip-")
                .tempdir_in(parent)
                .expect("test volume")
        },
    );
    let target = temp.path().join("cache 空格");
    fs::create_dir(&target).expect("test directory");
    fs::write(target.join("artifact.bin"), b"platform acceptance").expect("test contents");
    let receipt =
        crate::platform::trash_with_receipt(&target).expect("move isolated directory to Trash");
    let source_removed = !target.try_exists().expect("source status");
    crate::platform::restore_from_system_trash(&target, &receipt, chrono::Utc::now().timestamp())
        .expect("restore isolated directory");
    assert!(source_removed);
    assert_eq!(
        fs::read(target.join("artifact.bin")).expect("restored contents"),
        b"platform acceptance"
    );
}
