//! Release Audit Report Structure Test Suite for Milestone 20.7
//! Verifies that the v1.0.0-release-audit.md contains all required sections:
//! - Project, Version, Date
//! - Architecture Status
//! - Correctness Status
//! - Storage Status
//! - Concurrency Status
//! - Security Status
//! - Performance Status
//! - Cross-Platform Status
//! - Testing Status
//! - Documentation Status
//! - Release Status
//! - Known Limitations
//! - Final Decision: GO / NO-GO

use std::fs;
use std::path::PathBuf;

#[test]
fn test_audit_20_7_release_audit_report_contents() {
    let candidates = [
        PathBuf::from("docs/v1.0.0-release-audit.md"),
        PathBuf::from("../../docs/v1.0.0-release-audit.md"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/v1.0.0-release-audit.md"),
    ];
    let report_path = candidates
        .iter()
        .find(|p| p.is_file())
        .expect("Release audit document must exist at docs/v1.0.0-release-audit.md");

    let content = fs::read_to_string(report_path).expect("Read release audit report");

    let required_headers = vec![
        "Project",
        "Version",
        "Date",
        "Architecture Status",
        "Correctness Status",
        "Storage Status",
        "Concurrency Status",
        "Security Status",
        "Performance Status",
        "Cross-Platform Status",
        "Testing Status",
        "Documentation Status",
        "Release Status",
        "Known Limitations",
        "Final Decision",
        "GO",
    ];

    for header in required_headers {
        assert!(
            content.contains(header),
            "Release audit report missing mandatory element: '{}'",
            header
        );
    }
}