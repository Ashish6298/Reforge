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
use std::path::Path;

#[test]
fn test_audit_20_7_release_audit_report_contents() {
    let report_path = Path::new("docs/v1.0.0-release-audit.md");
    assert!(
        report_path.is_file(),
        "Release audit document must exist at docs/v1.0.0-release-audit.md"
    );

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
