#![cfg(test)]
use super::*;

fn test_layout(column_count: i64) -> RenderLayout {
    RenderLayout {
        column_count,
        total_column_height: 100,
        visible_playfield_width: 260,
        image_width: 1_000,
        image_height: 130,
        playfield_scale: 0.5,
        object_scale: 0.5,
        pixels_per_ms: 1.0,
        chart_start_time: 0,
    }
}

fn edge_fruit(x: f64, time: i64) -> RenderObject {
    RenderObject {
        object_type: ObjType::Fruit,
        x,
        start_time: time,
        color: crate::export::catch::constants::LAZER_COMBO_COLORS[0],
        scale_factor: 1.0,
        event_time: Some(time as f64),
        hyper_dash: false,
        edge: true,
        banana_shower_id: None,
        banana_route_x: None,
    }
}

fn route_banana(x: f64, route_x: f64, time: i64, shower_id: usize) -> RenderObject {
    let mut object = edge_fruit(x, time);
    object.object_type = ObjType::Banana;
    object.edge = false;
    object.banana_shower_id = Some(shower_id);
    object.banana_route_x = Some(route_x);
    object
}

#[test]
fn edge_guide_is_split_at_column_boundary() {
    let current = edge_fruit(0.0, 90);
    let next = edge_fruit(200.0, 110);

    let segments = edge_guide_segments(&current, &next, &test_layout(2));

    assert_eq!(segments.len(), 2);
    let chart_top = (crate::config::current()
        .render
        .catch
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .INFO_MARGIN_TOP) as f64;
    let chart_bottom = chart_top + 100.0;
    let ((_, first_start_y), (first_end_x, first_end_y)) = segments[0];
    let ((second_start_x, second_start_y), (_, second_end_y)) = segments[1];
    assert_eq!(first_start_y, chart_bottom - 90.0);
    assert_eq!(first_end_y, chart_top);
    assert_eq!(second_start_y, chart_bottom);
    assert_eq!(second_end_y, chart_bottom - 10.0);
    assert!(second_start_x > first_end_x);
}

#[test]
fn edge_guide_draws_configured_pixels_behind_objects() {
    let layout = test_layout(1);
    let current = edge_fruit(0.0, 10);
    let mut next = edge_fruit(200.0, 20);
    next.edge = false;
    let mut image = Img::new(400, 130, [7, 7, 7, 255]);

    draw_edge_guides(&mut image, &[current, next], &layout);

    let midpoint_x = playfield_left(0) + 50;
    let chart_bottom = crate::config::current()
        .render
        .catch
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .INFO_MARGIN_TOP
        + layout.total_column_height;
    assert_eq!(
        image.get(midpoint_x as u32, (chart_bottom - 15) as u32),
        crate::config::current()
            .render
            .catch
            .png
            .style
            .EDGE_GUIDE_COLOR
    );
}

#[test]
fn banana_route_draws_catcher_center_instead_of_banana_centers() {
    let layout = test_layout(1);
    let current = route_banana(0.0, 100.0, 10, 0);
    let next = route_banana(400.0, 100.0, 20, 0);
    let background = [7, 7, 7, 255];
    let mut image = Img::new(400, 130, background);

    draw_banana_routes(&mut image, &[current, next], &layout);

    let route_x = playfield_left(0) + 50;
    let banana_midpoint_x = playfield_left(0) + 100;
    let chart_bottom = crate::config::current()
        .render
        .catch
        .png
        .sizing
        .PAGE_MARGIN_TOP
        + crate::config::current()
            .render
            .catch
            .png
            .sizing
            .INFO_MARGIN_TOP
        + layout.total_column_height;
    assert_eq!(
        image.get(route_x as u32, (chart_bottom - 15) as u32),
        crate::export::catch::constants::BANANA_ROUTE_LINE_COLOR
    );
    assert_eq!(
        image.get(banana_midpoint_x as u32, (chart_bottom - 15) as u32),
        background
    );
}

#[test]
fn column_height_is_aligned_to_dominant_measure_interval() {
    let timing_lines: Vec<TimingLine> = (0..10)
        .map(|index| TimingLine {
            time: index * 2_000,
            is_measure: true,
            show_label: true,
            bpm: None,
        })
        .collect();

    let height = predominant_measure_aligned_height(&timing_lines, 0.5, 5_500).unwrap();
    assert_eq!(height, 5_000);
    assert_eq!(height % 1_000, 0);
}

#[test]
fn derived_playfield_padding_scales_with_the_column() {
    for scale in [0.5, 1.0, 1.5, 2.0] {
        let column_width = crate::export::geometry::scale_px(315.0, scale);
        let panel_width = crate::export::geometry::scale_px(9.0, scale);
        let playfield_width = crate::export::geometry::scale_px(260.0, scale);
        let padding = playfield_side_padding_for(column_width, panel_width, playfield_width);

        assert!((padding as f64 / scale - 23.0).abs() <= 1.0);
    }
}

#[test]
fn aligned_column_count_is_stable_across_output_scales() {
    let timing_lines: Vec<TimingLine> = (0..10)
        .map(|index| TimingLine {
            time: index * 2_000,
            is_measure: true,
            show_label: true,
            bpm: None,
        })
        .collect();

    for scale in [0.5, 1.0, 1.5, 2.0] {
        let pixels_per_ms = 0.5 * scale;
        let max_area_height = crate::export::geometry::scale_px(5_500.0, scale);
        let aligned =
            predominant_measure_aligned_height(&timing_lines, pixels_per_ms, max_area_height)
                .unwrap();
        let total_height = crate::export::geometry::scale_px(9_000.0, scale);

        assert_eq!(ceil_div(total_height, aligned), 2);
        assert!((aligned as f64 / scale - 5_000.0).abs() <= 1.0);
    }
}

#[test]
fn edge_combo_numbers_ignore_tiny_droplets_and_bananas() {
    let mut first = edge_fruit(20.0, 10);
    first.edge = false;
    let mut tiny = edge_fruit(30.0, 15);
    tiny.object_type = ObjType::TinyDroplet;
    tiny.edge = false;
    let mut droplet = edge_fruit(40.0, 20);
    droplet.object_type = ObjType::Droplet;
    let mut banana = edge_fruit(50.0, 25);
    banana.object_type = ObjType::Banana;
    banana.edge = false;
    let last = edge_fruit(60.0, 30);

    let labels = edge_combo_numbers(&[first, tiny, droplet, banana, last]);

    assert_eq!(labels, vec![(2, 2), (4, 3)]);
}

#[test]
fn edge_combo_label_is_drawn_next_to_the_edge_object() {
    let layout = test_layout(1);
    let current = edge_fruit(100.0, 10);
    let mut next = edge_fruit(200.0, 20);
    next.edge = false;
    let mut image = Img::new(
        400,
        130,
        crate::config::current()
            .render
            .catch
            .png
            .style
            .IMAGE_BACKGROUND,
    );

    draw_edge_combo_labels(&mut image, &[current, next], &layout);

    let has_white_label_pixel = (0..image.h).any(|y| {
        (0..image.w).any(|x| {
            image.get(x, y)
                == crate::config::current()
                    .render
                    .catch
                    .png
                    .style
                    .EDGE_COMBO_LABEL_COLOR
        })
    });
    assert!(has_white_label_pixel);
}
