//! Tests for `datasets verify`.
//!
//! Run against the download tests' in-process server, which speaks the file
//! list and the file contents — so "nothing is downloaded" is checked on the
//! requests the command really made.

use super::super::dataset_download::tests::{Mock, Scratch, ctx, pattern, server};
use super::*;
use std::path::Path;
use std::time::Duration;

const ID: &str = "ds-1";
const VIDEO: &str = "videos/observation.images.head/chunk-000/file-000.mp4";
const DATA: &str = "data/chunk-000/file-000.parquet";

/// What a dataset directory looks like after `datasets upload`: the tree, a
/// README, a file with a non-ASCII name and an empty one.
fn stored() -> Mock {
    let mut mock = Mock::tree();
    mock.files
        .push(("README.md".into(), b"# about this dataset\n".to_vec()));
    mock.files.push((
        "docs/übersicht — späte Schicht.md".into(),
        "Notizen".as_bytes().to_vec(),
    ));
    mock.files.push(("docs/empty.txt".into(), Vec::new()));
    mock
}

/// The same files on disk.
fn local_copy(mock: &Mock, tag: &str) -> Scratch {
    let dir = Scratch::new(tag);
    for (path, bytes) in &mock.files {
        dir.write(path, bytes);
    }
    dir
}

fn options(dir: &Path) -> VerifyOptions {
    let ms = Duration::from_millis;
    VerifyOptions {
        id: ID.into(),
        dir: dir.to_path_buf(),
        rules: ScanRules::default(),
        no_checksum: false,
        require_checksum: false,
        parallel: 4,
        tuning: Tuning {
            backoff_base: ms(1),
            max_pause: ms(5),
            poll_interval: ms(2),
        },
    }
}

fn kinds(outcome: &VerifyOutcome) -> Vec<(&str, DiffKind)> {
    outcome
        .differences
        .iter()
        .map(|d| (d.path.as_str(), d.kind))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_identical_directory_matches_and_no_file_is_downloaded() {
    let mock = stored();
    let dir = local_copy(&mock, "verify-same");
    let s = server(mock).await;

    let outcome = verify(&s.client, &ctx(), &options(&dir.0)).await.unwrap();

    assert!(outcome.matches(), "{:?}", outcome.differences);
    assert_eq!(outcome.local_files, 8);
    assert_eq!(outcome.remote_files, 8);
    assert_eq!(
        outcome.sha256_compared, 8,
        "every file, the empty one included"
    );
    assert_eq!(outcome.size_only, 0);
    assert!(outcome.left_out.is_empty());
    {
        let m = s.mock.lock().unwrap();
        assert!(
            m.log.iter().all(|l| !l.starts_with("GET content")),
            "verify fetched file contents: {:?}",
            m.log
        );
        // the manifest was paged through (two files per page)
        assert!(m.log.iter().filter(|l| l.contains("files")).count() >= 4);
    }

    run_verify(&s.client, &ctx(), options(&dir.0))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_kind_of_difference_is_listed_and_fails_the_command() {
    let mock = stored();
    let dir = local_copy(&mock, "verify-diff");
    // a file the instance never got
    dir.write("notes/calibration.yaml", b"fx: 1.0");
    // the same size, other bytes
    let mut changed = pattern(3000, 2);
    changed[1500] ^= 0xff;
    dir.write(DATA, &changed);
    // another size
    dir.write("meta/info.json", br#"{"fps":25}  "#);
    // gone locally
    std::fs::remove_file(dir.path("README.md")).unwrap();
    let s = server(mock).await;

    let outcome = verify(&s.client, &ctx(), &options(&dir.0)).await.unwrap();

    assert!(!outcome.matches());
    assert_eq!(
        kinds(&outcome),
        [
            ("README.md", DiffKind::OnlyOnInstance),
            (DATA, DiffKind::Sha256),
            ("meta/info.json", DiffKind::Size),
            ("notes/calibration.yaml", DiffKind::MissingOnInstance),
        ]
    );
    let size = &outcome.differences[2];
    assert!(
        size.detail.contains("12 bytes locally, 10 on the instance"),
        "{}",
        size.detail
    );
    // the size mismatch was not hashed; the other six present files were
    assert_eq!(outcome.sha256_compared, 6);
    assert!(
        s.mock
            .lock()
            .unwrap()
            .log
            .iter()
            .all(|l| !l.starts_with("GET content"))
    );

    let err = run_verify(&s.client, &ctx(), options(&dir.0))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("4 difference(s)"), "{err}");
    assert!(err.contains("do not delete the local copy"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_checksums_on_the_instance_sizes_decide_unless_one_is_required() {
    let mut mock = stored();
    mock.with_sha = false;
    let dir = local_copy(&mock, "verify-nosha");
    // same size, other bytes: invisible without a checksum
    let mut changed = pattern(200_000, 3);
    changed[7] ^= 1;
    dir.write(VIDEO, &changed);
    let s = server(mock).await;

    let outcome = verify(&s.client, &ctx(), &options(&dir.0)).await.unwrap();
    assert!(outcome.matches());
    assert_eq!(outcome.sha256_compared, 0);
    assert_eq!(outcome.size_only, 8, "and the command says so");

    let mut strict = options(&dir.0);
    strict.require_checksum = true;
    let outcome = verify(&s.client, &ctx(), &strict).await.unwrap();
    assert_eq!(outcome.differences.len(), 8);
    assert!(
        outcome
            .differences
            .iter()
            .all(|d| d.kind == DiffKind::NoChecksum)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_checksum_compares_sizes_only() {
    let mock = stored();
    let dir = local_copy(&mock, "verify-sizeonly");
    let mut changed = pattern(3000, 2);
    changed[0] ^= 1;
    dir.write(DATA, &changed);
    let s = server(mock).await;

    let mut opts = options(&dir.0);
    opts.no_checksum = true;
    let outcome = verify(&s.client, &ctx(), &opts).await.unwrap();
    assert!(outcome.matches());
    assert_eq!((outcome.sha256_compared, outcome.size_only), (0, 8));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_upload_leaves_out_is_listed_not_counted_as_missing() {
    let mock = stored();
    let dir = local_copy(&mock, "verify-leftout");
    dir.write(".git/HEAD", b"ref: refs/heads/main");
    dir.write("scripts/__pycache__/a.pyc", b"x");
    dir.write("raw/session.bag", b"excluded on upload");
    let s = server(mock).await;

    // without the --exclude the upload was run with, the file is missing
    let outcome = verify(&s.client, &ctx(), &options(&dir.0)).await.unwrap();
    assert_eq!(
        kinds(&outcome),
        [("raw/session.bag", DiffKind::MissingOnInstance)]
    );

    let mut opts = options(&dir.0);
    opts.rules.exclude = vec!["raw".into()];
    let outcome = verify(&s.client, &ctx(), &opts).await.unwrap();
    assert!(outcome.matches(), "{:?}", outcome.differences);
    let left: Vec<&str> = outcome.left_out.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(left, [".git/", "raw/", "scripts/__pycache__/"]);

    // a file that IS on the instance but excluded locally is still a
    // difference — and says why the local side does not have it
    opts.rules.exclude = vec!["raw".into(), "README.md".into()];
    let outcome = verify(&s.client, &ctx(), &opts).await.unwrap();
    assert_eq!(kinds(&outcome), [("README.md", DiffKind::OnlyOnInstance)]);
    assert!(
        outcome.differences[0]
            .detail
            .contains("left out of the local side: --exclude README.md"),
        "{}",
        outcome.differences[0].detail
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dataset_uploaded_before_every_file_was_stored_fails_on_its_readme() {
    // what an older server kept: the tree, no README
    let mock = Mock::tree();
    let dir = local_copy(&mock, "verify-old");
    dir.write("README.md", b"# about this dataset\n");
    let s = server(mock).await;

    let outcome = verify(&s.client, &ctx(), &options(&dir.0)).await.unwrap();
    assert_eq!(
        kinds(&outcome),
        [("README.md", DiffKind::MissingOnInstance)]
    );

    // comparing only the tree, as it was uploaded, matches — and lists the README
    let mut opts = options(&dir.0);
    opts.rules.lerobot_only = true;
    let outcome = verify(&s.client, &ctx(), &opts).await.unwrap();
    assert!(outcome.matches());
    assert_eq!(outcome.left_out.len(), 1);
    assert_eq!(outcome.left_out[0].0, "README.md");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_id_or_directory_is_an_error_not_a_match() {
    let mock = stored();
    let dir = local_copy(&mock, "verify-wrong");
    let s = server(mock).await;

    let mut opts = options(&dir.0);
    opts.id = "no-such-dataset".into();
    assert!(verify(&s.client, &ctx(), &opts).await.is_err());

    let opts = options(&dir.path("not-there"));
    let err = verify(&s.client, &ctx(), &opts)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot open"), "{err}");
}
