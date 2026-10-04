use std::{
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant},
};

use image::{Rgba, RgbaImage};
use sniplet_core::{
    Annotation, AnnotationId, AnnotationKind, AnnotationStyle, ArrowVariant, Color, Document,
    ImageRect, Point, Project, RenderOptions,
};

const SIZES: &[(u32, u32)] = &[(1600, 900), (2560, 1440), (3840, 2160)];
const FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSans.ttf");

#[derive(Clone, Copy)]
struct Measurement {
    median_ms: f64,
    p95_ms: f64,
    mean_ms: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let output = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts/interactive-performance/render-benchmark.csv"));
    let project = arguments.next().map(PathBuf::from);
    let project_only = arguments.any(|argument| argument == "--project-only");
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut csv = String::from("width,height,scenario,iterations,median_ms,p95_ms,mean_ms\n");
    println!("interactive renderer benchmark (release builds only)");

    if !project_only {
        for &(width, height) in SIZES {
            let iterations = iterations_for(width);
            let blank = Document::new(background(width, height));
            record_render(&mut csv, width, height, "blank", iterations, || {
                blank.render(&editor_options(&[]))
            })?;
            record_render(&mut csv, width, height, "blank_source", iterations, || {
                blank.render(&source_options(&[]))
            })?;

            let straight = document_with_arrow(width, height, false);
            record_render(
                &mut csv,
                width,
                height,
                "straight_arrow",
                iterations,
                || straight.render(&editor_options(&[])),
            )?;

            let curved = document_with_arrow(width, height, true);
            record_render(&mut csv, width, height, "curved_arrow", iterations, || {
                curved.render(&editor_options(&[]))
            })?;

            let several = document_with_annotations(width, height);
            record_render(
                &mut csv,
                width,
                height,
                "several_annotations",
                iterations,
                || several.render(&editor_options(&[])),
            )?;

            let preview = preview_arrow(width, height, 0);
            record_render(
                &mut csv,
                width,
                height,
                "several_plus_preview",
                iterations,
                || several.render(&editor_options(std::slice::from_ref(&preview))),
            )?;

            let preview_blank = Document::new(background(width, height));
            let mut preview_frame = 0_u32;
            record_render(&mut csv, width, height, "preview_drag", iterations, || {
                let preview = preview_arrow(width, height, preview_frame);
                preview_frame = preview_frame.wrapping_add(1);
                preview_blank.render(&editor_options(std::slice::from_ref(&preview)))
            })?;
            let mut preview_bgra_frame = 0_u32;
            record_render(
                &mut csv,
                width,
                height,
                "preview_drag_render_and_bgra",
                iterations,
                || {
                    let preview = preview_arrow(width, height, preview_bgra_frame);
                    preview_bgra_frame = preview_bgra_frame.wrapping_add(1);
                    let mut image =
                        preview_blank.render(&editor_options(std::slice::from_ref(&preview)))?;
                    swap_red_blue(&mut image);
                    Ok::<_, sniplet_core::SnipletError>(image)
                },
            )?;

            let mut edit_document = document_with_annotations(width, height);
            let arrow_id = edit_document.annotations()[0].id;
            edit_document.begin_group()?;
            let mut edit_frame = 0_u32;
            record_render(
                &mut csv,
                width,
                height,
                "anchor_drag_update_and_render",
                iterations,
                || {
                    move_arrow_anchor(&mut edit_document, arrow_id, width, height, edit_frame)?;
                    edit_frame = edit_frame.wrapping_add(1);
                    edit_document.render(&editor_options(&[]))
                },
            )?;
            edit_document.end_group()?;

            let mut bgra_document = document_with_annotations(width, height);
            let bgra_arrow_id = bgra_document.annotations()[0].id;
            bgra_document.begin_group()?;
            let mut bgra_frame = 0_u32;
            record_render(
                &mut csv,
                width,
                height,
                "anchor_drag_render_and_bgra",
                iterations,
                || {
                    move_arrow_anchor(
                        &mut bgra_document,
                        bgra_arrow_id,
                        width,
                        height,
                        bgra_frame,
                    )?;
                    bgra_frame = bgra_frame.wrapping_add(1);
                    let mut image = bgra_document.render(&editor_options(&[]))?;
                    swap_red_blue(&mut image);
                    Ok::<_, sniplet_core::SnipletError>(image)
                },
            )?;
            bgra_document.end_group()?;

            let mut conversion_frame = blank.render(&editor_options(&[]))?;
            let conversion = measure(iterations * 4, || {
                swap_red_blue(&mut conversion_frame);
                Ok::<_, std::convert::Infallible>(())
            })
            .expect("infallible conversion benchmark");
            record(
                &mut csv,
                width,
                height,
                "bgra_conversion_only",
                iterations * 4,
                conversion,
            );

            let mut mutation_document = document_with_annotations(width, height);
            let mutation_id = mutation_document.annotations()[0].id;
            mutation_document.begin_group()?;
            let mutation_samples = 100;
            let mutation_batch_size = 1_000;
            let mut mutation_frame = 0_u32;
            let mutation = measure_batched(mutation_samples, mutation_batch_size, || {
                let result = move_arrow_anchor(
                    &mut mutation_document,
                    mutation_id,
                    width,
                    height,
                    mutation_frame,
                );
                mutation_frame = mutation_frame.wrapping_add(1);
                result
            })?;
            mutation_document.end_group()?;
            record(
                &mut csv,
                width,
                height,
                "anchor_update_only",
                mutation_samples * mutation_batch_size as usize,
                mutation,
            );
        }
    }

    if let Some(project) = project {
        benchmark_project(&mut csv, &project)?;
    }
    std::fs::write(&output, csv)?;
    println!("wrote {}", output.display());
    Ok(())
}

fn record_render<E>(
    csv: &mut String,
    width: u32,
    height: u32,
    scenario: &str,
    iterations: usize,
    mut operation: impl FnMut() -> Result<RgbaImage, E>,
) -> Result<(), E> {
    for _ in 0..2 {
        black_box(operation()?);
    }
    let measurement = measure(iterations, operation)?;
    record(csv, width, height, scenario, iterations, measurement);
    Ok(())
}

fn measure<T, E>(
    iterations: usize,
    mut operation: impl FnMut() -> Result<T, E>,
) -> Result<Measurement, E> {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        black_box(operation()?);
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let total: Duration = samples.iter().copied().sum();
    let p95_index = ((samples.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len() - 1);
    Ok(Measurement {
        median_ms: milliseconds(samples[samples.len() / 2]),
        p95_ms: milliseconds(samples[p95_index]),
        mean_ms: milliseconds(total) / samples.len() as f64,
    })
}

fn measure_batched<T, E>(
    samples: usize,
    batch_size: u32,
    mut operation: impl FnMut() -> Result<T, E>,
) -> Result<Measurement, E> {
    let mut durations_ms = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        for _ in 0..batch_size {
            black_box(operation()?);
        }
        durations_ms.push(milliseconds(start.elapsed()) / batch_size as f64);
    }
    durations_ms.sort_by(f64::total_cmp);
    let total: f64 = durations_ms.iter().sum();
    let p95_index = ((durations_ms.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(durations_ms.len() - 1);
    Ok(Measurement {
        median_ms: durations_ms[durations_ms.len() / 2],
        p95_ms: durations_ms[p95_index],
        mean_ms: total / durations_ms.len() as f64,
    })
}

fn record(
    csv: &mut String,
    width: u32,
    height: u32,
    scenario: &str,
    iterations: usize,
    measurement: Measurement,
) {
    println!(
        "{width:4}x{height:<4} {scenario:30} median {:8.3} ms  p95 {:8.3} ms",
        measurement.median_ms, measurement.p95_ms,
    );
    csv.push_str(&format!(
        "{width},{height},{scenario},{iterations},{:.6},{:.6},{:.6}\n",
        measurement.median_ms, measurement.p95_ms, measurement.mean_ms,
    ));
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn iterations_for(width: u32) -> usize {
    match width {
        0..=1600 => 20,
        1601..=2560 => 12,
        _ => 6,
    }
}

fn source_options<'a>(extra_annotations: &'a [Annotation]) -> RenderOptions<'a> {
    RenderOptions {
        font_bytes: None,
        apply_crop: false,
        apply_backdrop: false,
        extra_annotations,
    }
}

fn editor_options<'a>(extra_annotations: &'a [Annotation]) -> RenderOptions<'a> {
    RenderOptions {
        font_bytes: Some(FONT),
        apply_crop: true,
        apply_backdrop: true,
        extra_annotations,
    }
}

fn background(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let shade = (((x / 32) + (y / 32)) % 2) as u8 * 12;
        Rgba([226 + shade, 230 + shade, 236 + shade, 255])
    })
}

fn document_with_arrow(width: u32, height: u32, curved: bool) -> Document {
    let mut document = Document::new(background(width, height));
    document.add_annotation(
        arrow(width, height, curved),
        AnnotationStyle {
            stroke: Color::new(220, 45, 48, 224),
            fill: None,
            stroke_width: 12.0,
        },
    );
    document
}

fn arrow(width: u32, height: u32, curved: bool) -> AnnotationKind {
    let width = width as f32;
    let height = height as f32;
    AnnotationKind::Arrow {
        start: Point::new(width * 0.16, height * 0.72),
        end: Point::new(width * 0.82, height * 0.28),
        bend: curved.then_some(Point::new(width * 0.52, height * 0.12)),
        variant: ArrowVariant::Solid,
    }
}

fn document_with_annotations(width: u32, height: u32) -> Document {
    let mut document = document_with_arrow(width, height, true);
    let width = width as f32;
    let height = height as f32;
    let styles = [
        AnnotationStyle {
            stroke: Color::new(24, 108, 224, 220),
            fill: Some(Color::new(24, 108, 224, 48)),
            stroke_width: 7.0,
        },
        AnnotationStyle {
            stroke: Color::new(38, 170, 94, 220),
            fill: Some(Color::new(38, 170, 94, 48)),
            stroke_width: 7.0,
        },
    ];
    document.add_annotation(
        AnnotationKind::Rectangle {
            rect: ImageRect::new(width * 0.08, height * 0.12, width * 0.25, height * 0.24),
        },
        styles[0],
    );
    document.add_annotation(
        AnnotationKind::Ellipse {
            rect: ImageRect::new(width * 0.62, height * 0.56, width * 0.25, height * 0.27),
        },
        styles[1],
    );
    document.add_annotation(
        AnnotationKind::Highlight {
            rect: ImageRect::new(width * 0.20, height * 0.44, width * 0.36, height * 0.09),
        },
        AnnotationStyle {
            stroke: Color::new(255, 225, 0, 96),
            fill: Some(Color::new(255, 225, 0, 96)),
            stroke_width: 1.0,
        },
    );
    document.add_annotation(
        AnnotationKind::Line {
            start: Point::new(width * 0.10, height * 0.88),
            end: Point::new(width * 0.42, height * 0.78),
        },
        styles[0],
    );
    document.add_annotation(
        AnnotationKind::Freehand {
            points: (0..24)
                .map(|index| {
                    let t = index as f32 / 23.0;
                    Point::new(
                        width * (0.58 + t * 0.31),
                        height * (0.42 + (t * 8.0).sin() * 0.04),
                    )
                })
                .collect(),
        },
        styles[1],
    );
    document.add_annotation(
        AnnotationKind::Redaction {
            rect: ImageRect::new(width * 0.38, height * 0.64, width * 0.16, height * 0.08),
        },
        AnnotationStyle {
            stroke: Color::BLACK,
            fill: Some(Color::BLACK),
            stroke_width: 1.0,
        },
    );
    document
}

fn preview_arrow(width: u32, height: u32, frame: u32) -> Annotation {
    let mut kind = arrow(width, height, true);
    let y = height as f32 * (0.22 + (frame % 9) as f32 * 0.006);
    if let AnnotationKind::Arrow { bend, .. } = &mut kind {
        *bend = Some(Point::new(width as f32 * 0.48, y));
    }
    Annotation {
        id: AnnotationId(u64::MAX),
        kind,
        style: AnnotationStyle {
            stroke: Color::new(142, 55, 220, 224),
            fill: None,
            stroke_width: 12.0,
        },
    }
}

fn move_arrow_anchor(
    document: &mut Document,
    id: AnnotationId,
    width: u32,
    height: u32,
    frame: u32,
) -> sniplet_core::Result<()> {
    let annotation = document
        .annotation(id)
        .expect("benchmark arrow exists")
        .clone();
    let mut kind = annotation.kind;
    if let AnnotationKind::Arrow { bend, .. } = &mut kind {
        *bend = Some(Point::new(
            width as f32 * (0.46 + (frame % 11) as f32 * 0.004),
            height as f32 * (0.12 + (frame % 7) as f32 * 0.006),
        ));
    }
    document.update_annotation(id, kind, annotation.style)
}

fn swap_red_blue(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
}

fn benchmark_project(
    csv: &mut String,
    project_path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let project = Project::load(project_path)?;
    let document = project.open_document()?;
    let width = document.width();
    let height = document.height();
    let iterations = iterations_for(width);
    println!("project fixture: {}", project_path.display());

    record_render(csv, width, height, "fixture_render", iterations, || {
        document.render(&editor_options(&[]))
    })?;

    let mut conversion_frame = document.render(&editor_options(&[]))?;
    let conversion = measure(iterations * 4, || {
        swap_red_blue(&mut conversion_frame);
        Ok::<_, std::convert::Infallible>(())
    })
    .expect("infallible conversion benchmark");
    record(
        csv,
        width,
        height,
        "fixture_bgra_conversion_only",
        iterations * 4,
        conversion,
    );

    let arrow_id = document
        .annotations()
        .iter()
        .find(|annotation| matches!(annotation.kind, AnnotationKind::Arrow { .. }))
        .map(|annotation| annotation.id)
        .ok_or("fixture has no arrow annotation")?;
    let original_arrow = document
        .annotation(arrow_id)
        .expect("fixture arrow exists")
        .clone();
    let mut dragging = document.clone();
    dragging.begin_group()?;
    let mut drag_frame = 0_u32;
    record_render(
        csv,
        width,
        height,
        "fixture_anchor_drag_render_bgra",
        iterations,
        || {
            move_fixture_arrow_bend(&mut dragging, &original_arrow, drag_frame)?;
            drag_frame = drag_frame.wrapping_add(1);
            let mut image = dragging.render(&editor_options(&[]))?;
            swap_red_blue(&mut image);
            Ok::<_, sniplet_core::SnipletError>(image)
        },
    )?;
    dragging.end_group()?;

    let mut preview_frame = 0_u32;
    record_render(
        csv,
        width,
        height,
        "fixture_preview_render_bgra",
        iterations,
        || {
            let preview = preview_arrow(width, height, preview_frame);
            preview_frame = preview_frame.wrapping_add(1);
            let mut image = document.render(&editor_options(std::slice::from_ref(&preview)))?;
            swap_red_blue(&mut image);
            Ok::<_, sniplet_core::SnipletError>(image)
        },
    )?;
    Ok(())
}

fn move_fixture_arrow_bend(
    document: &mut Document,
    original: &Annotation,
    frame: u32,
) -> sniplet_core::Result<()> {
    let mut kind = original.kind.clone();
    if let AnnotationKind::Arrow {
        start, end, bend, ..
    } = &mut kind
    {
        let middle = bend
            .get_or_insert_with(|| Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5));
        let phase = frame % 11;
        middle.x += phase as f32 * 4.0 - 20.0;
        middle.y += (phase as f32 * 0.7).sin() * 12.0;
    }
    document.update_annotation(original.id, kind, original.style)
}
