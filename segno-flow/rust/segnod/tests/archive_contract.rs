mod common;

use segnod::{ArchiveBudget, ArchiveError, PackageImporter};

use common::{ExtraEntry, manifest, test_directory, write_package};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn archive_configuration_cannot_disable_hard_ceilings() -> TestResult {
    let fixture = test_directory()?;
    let budget = ArchiveBudget {
        max_entries: 1_001,
        ..ArchiveBudget::default()
    };

    assert!(matches!(
        PackageImporter::new(&fixture.path().join("state"), budget),
        Err(ArchiveError::InvalidBudget)
    ));
    Ok(())
}

#[test]
fn valid_package_is_published_without_executing_stage_code() -> TestResult {
    let fixture = test_directory()?;
    let archive = write_package(
        &fixture.path().join("task.zip"),
        &manifest("forbid", "coalesce"),
        Vec::new(),
    )?;
    let state = fixture.path().join("state");
    std::fs::create_dir_all(&state)?;
    let importer = PackageImporter::new(&state, ArchiveBudget::default())?;
    let published = importer.import(&archive)?;

    assert_eq!(published.manifest.id, "test-task");
    assert!(published.directory.join("scripts/main.py").is_file());
    assert!(!fixture.path().join("must-never-exist").exists());
    Ok(())
}

#[test]
fn existing_digest_path_is_reverified_before_reuse() -> TestResult {
    let fixture = test_directory()?;
    let archive = write_package(
        &fixture.path().join("task.zip"),
        &manifest("forbid", "coalesce"),
        Vec::new(),
    )?;
    let state = fixture.path().join("state");
    std::fs::create_dir_all(&state)?;
    let importer = PackageImporter::new(&state, ArchiveBudget::default())?;
    let published = importer.import(&archive)?;
    std::fs::write(published.directory.join("scripts/main.py"), b"corrupt\n")?;

    assert!(matches!(
        importer.import(&archive),
        Err(ArchiveError::PublishConflict)
    ));
    Ok(())
}

#[test]
fn traversal_and_symlink_are_rejected_before_publication() -> TestResult {
    let fixture = test_directory()?;
    let state = fixture.path().join("state");
    std::fs::create_dir_all(&state)?;
    let importer = PackageImporter::new(&state, ArchiveBudget::default())?;
    let traversal = write_package(
        &fixture.path().join("traversal.zip"),
        &manifest("forbid", "coalesce"),
        vec![ExtraEntry::File("../escape.py".into(), b"pass\n".to_vec())],
    )?;
    assert!(matches!(
        importer.import(&traversal),
        Err(ArchiveError::PathPolicyViolation(_))
    ));

    let symlink = write_package(
        &fixture.path().join("symlink.zip"),
        &manifest("forbid", "coalesce"),
        vec![ExtraEntry::Symlink(
            "scripts/link.py".into(),
            "../../outside.py".into(),
        )],
    )?;
    assert!(matches!(
        importer.import(&symlink),
        Err(ArchiveError::PackageInvalid("symbolic link"))
    ));
    Ok(())
}

#[test]
fn case_and_unicode_normalization_collisions_are_rejected() -> TestResult {
    let fixture = test_directory()?;
    let state = fixture.path().join("state");
    std::fs::create_dir_all(&state)?;
    let importer = PackageImporter::new(&state, ArchiveBudget::default())?;
    let case = write_package(
        &fixture.path().join("case.zip"),
        &manifest("forbid", "coalesce"),
        vec![
            ExtraEntry::File("Data/Value.txt".into(), b"a".to_vec()),
            ExtraEntry::File("data/value.TXT".into(), b"b".to_vec()),
        ],
    )?;
    assert!(matches!(
        importer.import(&case),
        Err(ArchiveError::PathCollision(_))
    ));

    let unicode = write_package(
        &fixture.path().join("unicode.zip"),
        &manifest("forbid", "coalesce"),
        vec![
            ExtraEntry::File("data/caf\u{e9}.txt".into(), b"a".to_vec()),
            ExtraEntry::File("data/cafe\u{301}.txt".into(), b"b".to_vec()),
        ],
    )?;
    assert!(matches!(
        importer.import(&unicode),
        Err(ArchiveError::PathCollision(_))
    ));
    Ok(())
}

#[test]
fn compressed_and_streamed_size_budgets_fail_closed() -> TestResult {
    let fixture = test_directory()?;
    let archive = write_package(
        &fixture.path().join("large.zip"),
        &manifest("forbid", "coalesce"),
        vec![ExtraEntry::File("data/payload.bin".into(), vec![7; 4_096])],
    )?;
    let state = fixture.path().join("state");
    std::fs::create_dir_all(&state)?;
    let budget = ArchiveBudget {
        max_file_bytes: 1_024,
        ..ArchiveBudget::default()
    };
    let importer = PackageImporter::new(&state, budget)?;
    assert!(matches!(
        importer.import(&archive),
        Err(ArchiveError::QuotaExceeded("single file bytes"))
    ));
    assert!(
        std::fs::read_dir(state.join("packages/sha256"))?
            .next()
            .is_none()
    );
    Ok(())
}
