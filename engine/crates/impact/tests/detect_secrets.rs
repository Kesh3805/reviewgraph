//! RISK-002: secret-like literals in added lines. Fake secrets are assembled at run time so no
//! secret-shaped literal sits in the source.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::time::Instant;

use impact::input::{AddedLine, ChangeSet, FileInput};
use impact::risk::{
    detect_all, detect_secret_signals, scan_line, RiskLevel, RiskPolicy, SignalSubject,
};
use review_core::change::FileChangeStatus;
use support::path;

fn file_with(file: &str, lines: &[&str]) -> FileInput {
    let mut input = FileInput::new(path(file), FileChangeStatus::Modified);
    input.added_lines = Some(
        lines
            .iter()
            .enumerate()
            .map(|(idx, text)| AddedLine {
                line: idx as u32 + 1,
                text: (*text).to_owned(),
            })
            .collect(),
    );
    input
}

fn change_of(files: Vec<FileInput>) -> ChangeSet {
    ChangeSet {
        files,
        ..ChangeSet::default()
    }
}

#[test]
fn fake_aws_key_detected_and_value_not_serialized() {
    let key = format!("AKIA{}", "ABCDEFGHIJKLMNOP");
    let line = format!("const accessKey = '{key}';");
    let detection = detect_secret_signals(&change_of(vec![file_with("src/config.ts", &[&line])]));
    assert_eq!(detection.signals.len(), 1);
    let signal = &detection.signals[0];
    assert_eq!(signal.rule_id, "secrets.literal_added");
    assert_eq!(signal.floor_level, Some(RiskLevel::Critical));
    assert_eq!(signal.evidence[0].line, Some(1));
    assert_eq!(
        signal.evidence[0].redacted_preview.as_deref(),
        Some("AKIA****")
    );
    assert_eq!(signal.subject, SignalSubject::File(path("src/config.ts")));

    // The value appears in no serialized artifact, nor in the full detection output.
    let json = serde_json::to_string(&detection.signals).unwrap();
    assert!(!json.contains(&key));
    assert!(!json.contains("ABCDEFGHIJKLMNOP"));
    let all = detect_all(
        &change_of(vec![file_with("src/config.ts", &[&line])]),
        None,
        &RiskPolicy::default(),
    );
    let all_json = serde_json::to_string(&all.signals).unwrap();
    assert!(!all_json.contains("ABCDEFGHIJKLMNOP"));
    assert!(!format!("{all:?}").contains("ABCDEFGHIJKLMNOP"));
}

#[test]
fn vcs_token_pattern() {
    let token = format!("ghp_{}", "a1B2c3D4e5".repeat(4));
    let finding = scan_line(&format!("headers.Authorization = 'token {token}'")).unwrap();
    assert_eq!(finding.rule, "vcs_token");
    assert_eq!(finding.preview, "ghp_****");
    assert!(scan_line("const ghost_value = 'ghp_short';").is_none());
}

#[test]
fn private_key_header() {
    let header = format!("-----BEGIN {} PRIVATE KEY-----", "RSA");
    let finding = scan_line(&header).unwrap();
    assert_eq!(finding.rule, "private_key");
}

#[test]
fn generic_assignment_and_placeholders() {
    let finding = scan_line("const password = 'correct-horse-battery';").unwrap();
    assert_eq!(finding.rule, "generic_assignment");
    assert!(scan_line("const password = 'short';").is_none());
    assert!(scan_line("const token = '${TOKEN_FROM_THE_ENVIRONMENT}';").is_none());
}

#[test]
fn entropy_detector_ignores_short_strings() {
    // 31 characters: below the minimum length.
    let short = "q9Zx7Lk2Vb8Nw4Tr6Yh3Pc5Md1Fg0Js";
    assert_eq!(short.len(), 31);
    assert!(scan_line(&format!("const blob = '{short}';")).is_none());
    // 40 mixed characters: above the length and entropy thresholds.
    let long = "q9Zx7Lk2Vb8Nw4Tr6Yh3Pc5Md1Fg0Js2Ae8Uo4Xz";
    assert_eq!(long.len(), 40);
    let finding = scan_line(&format!("const blob = '{long}';")).unwrap();
    assert_eq!(finding.rule, "high_entropy");
    // Long but repetitive: low entropy.
    let repetitive = "ab".repeat(20);
    assert!(scan_line(&format!("const blob = '{repetitive}';")).is_none());
}

#[test]
fn committed_env_file_flagged_and_example_not() {
    let mut env = FileInput::new(path("config/.env.production"), FileChangeStatus::Added);
    env.added_lines = Some(Vec::new());
    let mut example = FileInput::new(path(".env.example"), FileChangeStatus::Added);
    example.added_lines = Some(vec![AddedLine {
        line: 1,
        text: "API_KEY=\"replace-with-a-real-key-please\"".to_owned(),
    }]);
    let detection = detect_secret_signals(&change_of(vec![env, example]));
    assert_eq!(detection.signals.len(), 1);
    assert_eq!(detection.signals[0].rule_id, "secrets.env_file");
    assert_eq!(detection.signals[0].weight, 0.8);
}

#[test]
fn unreadable_blob_warns_instead_of_failing() {
    let mut unreadable = FileInput::new(path("src/a.ts"), FileChangeStatus::Modified);
    unreadable.added_lines = None;
    let detection = detect_secret_signals(&change_of(vec![unreadable]));
    assert!(detection.signals.is_empty());
    assert_eq!(detection.warnings.len(), 1);
    assert_eq!(detection.warnings[0].code, "secrets_scan_incomplete");
}

#[test]
fn removed_files_are_not_scanned() {
    let key = format!("AKIA{}", "ABCDEFGHIJKLMNOP");
    let mut removed = file_with("src/old.ts", &[&format!("const k = '{key}';")]);
    removed.status = FileChangeStatus::Deleted;
    assert!(detect_secret_signals(&change_of(vec![removed]))
        .signals
        .is_empty());
}

#[test]
fn scan_of_ten_thousand_lines_is_fast() {
    let lines: Vec<String> = (0..10_000)
        .map(|i| format!("const value{i} = compute({i}, 'label-{i}');"))
        .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let change = change_of(vec![file_with("src/big.ts", &refs)]);
    let started = Instant::now();
    let detection = detect_secret_signals(&change);
    assert!(detection.signals.is_empty());
    // The target is 20 ms in a release build; the bound here only guards against blow-ups.
    assert!(started.elapsed().as_millis() < 2_000);
}
