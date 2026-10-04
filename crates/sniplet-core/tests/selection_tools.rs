use image::{Rgba, RgbaImage};
use sniplet_core::{Color, Document, ImageRect, Point};

#[test]
fn source_color_analysis_is_clipped_and_deterministic() {
    let image = RgbaImage::from_fn(6, 4, |x, y| Rgba([(x * 20) as u8, (y * 30) as u8, 40, 255]));
    let document = Document::new(image);
    assert_eq!(
        document.average_color(ImageRect::new(0.0, 0.0, 2.0, 2.0)),
        Some(Color::new(10, 15, 40, 255))
    );
    assert_eq!(
        document.average_color(ImageRect::new(-5.0, -5.0, 1.0, 1.0)),
        None
    );
    assert_eq!(
        document.darkest_color_around(Point::new(2.0, 2.0), 2),
        Some(Color::new(40, 0, 40, 255))
    );
}

#[test]
fn automatic_selection_trims_uniform_background() {
    let mut image = RgbaImage::from_pixel(14, 12, Rgba([245, 245, 245, 255]));
    for y in 3..8 {
        for x in 4..10 {
            image.put_pixel(x, y, Rgba([40, 80, 120, 255]));
        }
    }
    let document = Document::new(image);
    assert_eq!(
        document.auto_adjust_selection(ImageRect::new(1.0, 1.0, 12.0, 10.0)),
        Some(ImageRect::new(4.0, 3.0, 6.0, 5.0))
    );
    assert_eq!(
        document.auto_adjust_selection(ImageRect::new(0.0, 0.0, 3.0, 3.0)),
        None
    );
}

#[test]
fn monotone_selection_flood_fills_only_the_connected_region() {
    let image = RgbaImage::from_fn(10, 6, |x, _| {
        if !(4..8).contains(&x) {
            Rgba([30, 31, 30, 255])
        } else {
            Rgba([120, 120, 120, 255])
        }
    });
    let document = Document::new(image);
    assert_eq!(
        document.select_monotone_at(Point::new(1.0, 2.0), 2),
        Some(ImageRect::new(0.0, 0.0, 4.0, 6.0))
    );
    assert_eq!(
        document.select_monotone_at(Point::new(9.0, 2.0), 2),
        Some(ImageRect::new(8.0, 0.0, 2.0, 6.0))
    );
    assert_eq!(document.select_monotone_at(Point::new(20.0, 2.0), 2), None);
}

#[test]
fn unicode_is_rejected_as_hex_without_panicking() {
    assert_eq!(Color::from_hex("éx"), None);
    assert_eq!(Color::from_hex("１２３"), None);
    assert_eq!(
        Color::from_hex("#00ff80cc"),
        Some(Color::new(0, 255, 128, 204))
    );
}
