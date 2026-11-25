//! Changelog and release workflow tests.

mod common;

use common::{
    command, normalize_output, run_commands, run_dynamic_commands, today, work_add_acceptance,
    work_move_done, work_new_active, work_tick_acceptance_done,
};
use tempfile::TempDir;

macro_rules! assert_changelog_snapshot {
    ($name:literal, $value:expr) => {{
        let value = $value;
        let snapshot_name = common::named_snapshot_name("test_changelog", $name);
        crate::with_test_snapshot_settings!({
            insta::assert_snapshot!(snapshot_name, value);
        });
    }};
}

mod changelog_tests;

#[cfg(windows)]
#[test]
fn normalize_output_replaces_verbatim_and_portable_temp_paths()
-> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(r"\\?\C:\workspace\govctl");
    for output in [r"\\?\C:\workspace\govctl", "C:/workspace/govctl"] {
        assert_eq!(normalize_output(output, root, "2026-11-25")?, "<TEMPDIR>");
    }
    Ok(())
}
