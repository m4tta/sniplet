use image::{Rgba, RgbaImage};
use sniplet_core::{Point, StitchOptions, ViewportTransform, stitch_vertical};

#[test]
fn cursor_anchored_zoom_round_trips_coordinates() {
    let mut transform = ViewportTransform::new(2.0, Point::new(10.0, -5.0));
    let image_point = Point::new(17.0, 23.0);
    let screen_point = transform.image_to_screen(image_point);
    assert_eq!(transform.screen_to_image(screen_point), image_point);

    let cursor = Point::new(250.0, 140.0);
    let anchored_pixel = transform.screen_to_image(cursor);
    transform.set_zoom_around(5.0, cursor);
    assert_eq!(transform.image_to_screen(anchored_pixel), cursor);
}

#[test]
fn deterministic_vertical_stitch_recovers_the_original_fixture() {
    let original = RgbaImage::from_fn(8, 15, |x, y| {
        Rgba([
            (x * 17 + y * 3) as u8,
            (y * 13 + x) as u8,
            (x * 5 + y * 11) as u8,
            255,
        ])
    });
    let frames = vec![
        image::imageops::crop_imm(&original, 0, 0, 8, 7).to_image(),
        image::imageops::crop_imm(&original, 0, 5, 8, 7).to_image(),
        image::imageops::crop_imm(&original, 0, 10, 8, 5).to_image(),
    ];
    let stitched = stitch_vertical(
        &frames,
        StitchOptions {
            min_overlap: 2,
            max_overlap: Some(4),
            max_mean_error: 0.0,
        },
    )
    .unwrap();
    assert_eq!(stitched, original);
}

#[test]
fn stitching_rejects_unrelated_frames_and_width_changes() {
    let first = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 255]));
    let unrelated = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
    assert!(
        stitch_vertical(
            &[first.clone(), unrelated],
            StitchOptions {
                min_overlap: 2,
                max_overlap: None,
                max_mean_error: 2.0,
            }
        )
        .is_err()
    );
    let wrong_width = RgbaImage::new(3, 4);
    assert!(stitch_vertical(&[first, wrong_width], StitchOptions::default()).is_err());
}
