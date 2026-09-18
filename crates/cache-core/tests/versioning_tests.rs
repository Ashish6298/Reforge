use dcc_core::{is_schema_compatible, DCC_SCHEMA_VERSION, DCC_VERSION};

#[test]
fn test_milestone_19_1_semver_compliance_and_schema_versioning() {
    // 1. Verify SemVer format (Major.Minor.Patch)
    assert!(!DCC_VERSION.is_empty());
    let parts: Vec<&str> = DCC_VERSION.split('.').collect();
    assert_eq!(
        parts.len(),
        3,
        "DCC_VERSION must adhere strictly to SemVer (X.Y.Z), got: {}",
        DCC_VERSION
    );

    for (idx, part) in parts.iter().enumerate() {
        assert!(
            part.chars().all(|c| c.is_ascii_digit()),
            "SemVer component {} ('{}') must be numeric",
            idx,
            part
        );
    }

    assert_eq!(DCC_VERSION, "1.0.0");

    // 2. Schema versioning compatibility
    assert_eq!(DCC_SCHEMA_VERSION, 1);
    assert!(is_schema_compatible(1));
    assert!(!is_schema_compatible(0));
    assert!(!is_schema_compatible(2));
    assert!(!is_schema_compatible(999));
}
