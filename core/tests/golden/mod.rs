// Helpers are shared across several test files via `mod golden;`; not every
// file uses every helper.
#![allow(dead_code)]

//! Reusable golden-file harness for the pure rendering seam (slices 03–07).
//!
//! Each case is a `(name, Value)`; the harness renders every case and compares
//! the result against a checked-in golden file
//! `tests/golden/<format>/<category>.txt`. Regenerate goldens with
//! `UPDATE_GOLDEN=1 cargo test`. Later rendering slices just call `check_tabular`
//! with their own category and cases.

use std::path::PathBuf;

use mgconsole_core::{render, Value};

fn golden_path(format: &str, category: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format)
        .join(format!("{category}.txt"))
}

/// Compare the tabular rendering of each named case against its golden file.
pub fn check_tabular(category: &str, cases: &[(&str, Value)]) {
    let rendered = cases
        .iter()
        .map(|(name, v)| format!("{name}\t{}", render::tabular(v)))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    let path = golden_path("tabular", category);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read golden {}: {e}; run `UPDATE_GOLDEN=1 cargo test` to create",
            path.display()
        )
    });
    assert_eq!(rendered, expected, "tabular golden mismatch for `{category}`");
}

/// Compare a multi-line block of text (e.g. a whole rendered table) against a
/// golden file `tests/golden/<format>/<name>.txt`.
pub fn check_block(format: &str, name: &str, rendered: &str) {
    let path = golden_path(format, name);
    let rendered = format!("{}\n", rendered.trim_end_matches('\n'));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read golden {}: {e}; run `UPDATE_GOLDEN=1 cargo test` to create",
            path.display()
        )
    });
    assert_eq!(rendered, expected, "block golden mismatch for `{format}/{name}`");
}
