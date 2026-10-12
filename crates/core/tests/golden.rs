//! Golden files: the grid style's output on small synthetic images, compared to files in
//! `crates/core/golden`.

// Task 11 adds the JSON export this test compares through, writes the files and un-ignores it.
#[test]
#[ignore = "needs the JSON export from Task 11"]
fn grid_matches_golden_files() {
    // Cases: gradient(64, 48) and checkerboard(64, 48, 8); Canvas::new(100, 75, 5, Fit); pen "Black 0.3", 0.3, "#000000";
    // settings: tone.resolution = 64, points.threshold = 0.6, everything else default.
    // Compares to crates/core/golden/{grid_gradient,grid_checkerboard}.json: every top-level key except "layers" equal,
    // same layer count, same pen, same path count, coordinates within 1e-3 in order.
    // PLOTLINE_UPDATE_GOLDEN=1 rewrites the files with serde_json::to_string_pretty.
    todo!("compare through plotline_core::export::json::to_value once it exists")
}
