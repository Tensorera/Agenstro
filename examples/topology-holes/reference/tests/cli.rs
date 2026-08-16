use std::{path::Path, process::Command};

#[test]
fn two_hole_fixture_matches_the_public_cli_contract() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("reference crate has an example parent");
    let fixture = example.join("fixtures/two-holes.grid");
    let expected = std::fs::read_to_string(example.join("fixtures/two-holes.expected.json"))
        .expect("expected JSON fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_topology-holes"))
        .arg(fixture)
        .output()
        .expect("topology CLI starts");

    assert!(output.status.success(), "CLI failed: {output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)
            .expect("stdout is UTF-8")
            .trim(),
        expected.trim()
    );
    assert!(output.stderr.is_empty());
}
