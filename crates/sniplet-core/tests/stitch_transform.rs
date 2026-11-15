use image::{Rgba, RgbaImage};
use sniplet_core::{
    Point, SnipletError, StitchOptions, Stitcher, ViewportTransform, stitch_vertical,
};

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

fn fixture() -> RgbaImage {
    RgbaImage::from_fn(8, 15, |x, y| {
        Rgba([
            (x * 17 + y * 3) as u8,
            (y * 13 + x) as u8,
            (x * 5 + y * 11) as u8,
            255,
        ])
    })
}

/// The fixture cut into three frames that overlap by 2 and 2 rows.
fn fixture_frames(original: &RgbaImage) -> Vec<RgbaImage> {
    vec![
        image::imageops::crop_imm(original, 0, 0, 8, 7).to_image(),
        image::imageops::crop_imm(original, 0, 5, 8, 7).to_image(),
        image::imageops::crop_imm(original, 0, 10, 8, 5).to_image(),
    ]
}

const EXACT: StitchOptions = StitchOptions {
    min_overlap: 2,
    max_overlap: Some(4),
    max_mean_error: 0.0,
};

#[test]
fn deterministic_vertical_stitch_recovers_the_original_fixture() {
    let original = fixture();
    let stitched = stitch_vertical(&fixture_frames(&original), EXACT).unwrap();
    assert_eq!(stitched, original);
}

#[test]
fn stitching_ignores_spare_bytes_after_the_last_row() {
    let original = fixture();
    let padded = fixture_frames(&original)
        .into_iter()
        .map(|frame| {
            let (width, height) = frame.dimensions();
            let mut raw = frame.into_raw();
            raw.extend_from_slice(&[9; 16]);
            RgbaImage::from_raw(width, height, raw).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(stitch_vertical(&padded, EXACT).unwrap(), original);
}

#[test]
fn stitching_rejects_frames_it_cannot_measure() {
    // Both cases used to score NaN, which passed the `error > max_mean_error` check.
    let empty = RgbaImage::new(0, 30);
    let result = stitch_vertical(&[empty.clone(), empty], StitchOptions::default());
    assert!(matches!(
        result,
        Err(SnipletError::StitchOverlapNotFound { index: 1, .. })
    ));

    let black = RgbaImage::from_pixel(4, 8, Rgba([0, 0, 0, 255]));
    let white = RgbaImage::from_pixel(4, 8, Rgba([255, 255, 255, 255]));
    let nan_threshold = StitchOptions {
        max_mean_error: f32::NAN,
        ..EXACT
    };
    let result = stitch_vertical(&[black, white], nan_threshold);
    assert!(matches!(
        result,
        Err(SnipletError::StitchOverlapNotFound { index: 1, .. })
    ));
}

#[test]
fn sticky_header_and_footer_stay_out_of_matching_and_appear_once() {
    let header = Rgba([200, 200, 200, 255]);
    let footer = Rgba([90, 0, 90, 255]);
    let page_row = |y: u32| Rgba([y as u8, (y * 7) as u8, 30, 255]);
    // 16 rows: a 4-row header, 9 page rows from `start`, and a 3-row footer.
    let frame = |start: u32| {
        RgbaImage::from_fn(3, 16, |_, y| match y {
            0..4 => header,
            13.. => footer,
            _ => page_row(start + y - 4),
        })
    };
    // Each frame scrolls 6 page rows, so neighbours share 3.
    let frames = [frame(0), frame(6), frame(12)];
    assert!(
        stitch_vertical(&frames, EXACT).is_err(),
        "full frames never match"
    );

    let mut stitcher = Stitcher::with_sticky_bands(&frames[0], EXACT);
    assert_eq!(stitcher.push(&frames[1]).unwrap(), 6);
    assert_eq!(stitcher.push(&frames[2]).unwrap(), 6);
    assert_eq!(
        stitcher.push(&frames[2]).unwrap(),
        0,
        "a repeated frame adds nothing"
    );
    let expected = RgbaImage::from_fn(3, 28, |_, y| match y {
        0..4 => header,
        25.. => footer,
        _ => page_row(y - 4),
    });
    assert_eq!(stitcher.into_image(), expected);
}

#[test]
fn a_failed_push_keeps_the_page_stitched_so_far() {
    let original = fixture();
    let frames = fixture_frames(&original);
    let mut stitcher = Stitcher::with_sticky_bands(&frames[0], EXACT);
    stitcher.push(&frames[1]).unwrap();
    let unrelated = RgbaImage::from_pixel(8, 7, Rgba([255, 0, 255, 255]));
    assert!(stitcher.push(&unrelated).is_err());
    assert_eq!(
        stitcher.into_image(),
        image::imageops::crop_imm(&original, 0, 0, 8, 12).to_image()
    );
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
