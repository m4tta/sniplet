use std::path::PathBuf;

use image::{Rgba, RgbaImage};
use sniplet_core::{
    AnnotationKind, AnnotationStyle, ArrowVariant, Color, Document, Point, RenderOptions,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/arrow-variants.png"));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    const WIDTH: u32 = 1120;
    const HEIGHT: u32 = 660;
    const CELL_WIDTH: f32 = 280.0;
    const CELL_HEIGHT: f32 = 165.0;
    let background = RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
        let column = x / CELL_WIDTH as u32;
        let row = y / CELL_HEIGHT as u32;
        if (column + row).is_multiple_of(2) {
            Rgba([247, 248, 250, 255])
        } else {
            Rgba([238, 241, 245, 255])
        }
    });
    let mut document = Document::new(background);

    for (row, variant) in [
        ArrowVariant::Solid,
        ArrowVariant::HandDrawn,
        ArrowVariant::Thin,
        ArrowVariant::DoubleEnded,
    ]
    .into_iter()
    .enumerate()
    {
        for column in 0..4 {
            let left = column as f32 * CELL_WIDTH;
            let top = row as f32 * CELL_HEIGHT;
            let size = if column < 2 { 4.0 } else { 12.0 };
            let start = Point::new(left + 32.0, top + 100.0);
            let end = Point::new(left + 248.0, top + 62.0);
            let bend = (column % 2 == 1).then_some(Point::new(left + 138.0, top + 28.0));
            document.add_annotation(
                AnnotationKind::Arrow {
                    start,
                    end,
                    bend,
                    variant,
                },
                AnnotationStyle {
                    stroke: Color::new(232, 48, 42, 255),
                    fill: None,
                    stroke_width: size,
                },
            );
        }
    }

    document.render(&RenderOptions::default())?.save(&output)?;
    println!("wrote {}", output.display());
    Ok(())
}
