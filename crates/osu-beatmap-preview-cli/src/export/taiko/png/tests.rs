#![cfg(test)]
use super::compute_row_start_positions;

#[test]
fn row_break_measure_anchors_are_scale_invariant_for_all_bpm_tiers() {
    let logical_measures = [0.0, 600.0, 1_200.0, 1_800.0, 2_400.0, 3_000.0, 3_600.0];
    let logical_chart_width = 3_900.0;

    for multiplier in [1.0, 1.15, 1.3, 1.45] {
        let logical_row_width = 1_300.0 * multiplier;
        let baseline = compute_row_start_positions(
            &logical_measures,
            logical_chart_width,
            osu_beatmap_preview_core::processing::parse::round_half_even(logical_row_width),
        );

        for scale in [0.5, 1.0, 1.5, 2.0] {
            let measures: Vec<f64> = logical_measures
                .iter()
                .map(|position| position * scale)
                .collect();
            let starts = compute_row_start_positions(
                &measures,
                logical_chart_width * scale,
                osu_beatmap_preview_core::processing::parse::round_half_even(
                    logical_row_width * scale,
                ),
            );

            assert_eq!(starts.len(), baseline.len());
            for (actual, expected) in starts.iter().zip(&baseline) {
                assert!((actual / scale - expected).abs() <= 1.0);
            }
        }
    }
}
