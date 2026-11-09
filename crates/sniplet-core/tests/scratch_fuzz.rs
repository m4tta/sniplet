//! Throwaway deterministic fuzz harness for project loading and image decoding.
//! Not meant to be merged. Run with:
//!   cargo test -p sniplet-core --test scratch_fuzz -- --test-threads=1 --nocapture

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::RefCell,
    io::Read,
    panic::{AssertUnwindSafe, catch_unwind},
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use image::{
    ExtendedColorType, ImageEncoder, Rgba, RgbaImage,
    codecs::{
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType, PngEncoder},
        webp::WebPEncoder,
    },
};
use serde_json::Value;
use sniplet_core::{
    AnnotationId, AnnotationKind, AnnotationStyle, ArrowVariant, Backdrop, Background, Color,
    Document, ImageRect, ImageSize, Measurement, Point, Project, RenderOptions, Shadow,
    decode_image,
};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSans.ttf");

// ---------------------------------------------------------------------------
// Capped allocator: any single request over 1 GiB fails, which aborts with
// "memory allocation of N bytes failed". This keeps a bomb from thrashing the
// shared machine and makes the oversize request visible.
// ---------------------------------------------------------------------------
const ALLOC_CAP: usize = 1 << 30;
struct Capped;
unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > ALLOC_CAP {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() > ALLOC_CAP {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > ALLOC_CAP {
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static GLOBAL: Capped = Capped;

// ---------------------------------------------------------------------------
// RNG and helpers
// ---------------------------------------------------------------------------
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() >> 40) as f32 / (1u64 << 24) as f32 * (hi - lo)
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

thread_local! {
    static LAST_PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}

fn install_quiet_hook() {
    std::panic::set_hook(Box::new(|info| {
        LAST_PANIC.with(|last| *last.borrow_mut() = info.to_string());
    }));
}

/// Runs `f`, returning `Err(panic message)` on a panic.
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f))
        .map_err(|_| LAST_PANIC.with(|last| last.borrow().replace('\n', " ")))
}

fn options() -> RenderOptions<'static> {
    RenderOptions {
        font_bytes: Some(FONT),
        ..RenderOptions::default()
    }
}

fn png(image: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .unwrap();
    bytes
}

fn base_image() -> RgbaImage {
    RgbaImage::from_fn(64, 48, |x, y| {
        Rgba([(x * 4) as u8, (y * 5) as u8, ((x + y) * 2) as u8, 255])
    })
}

// ---------------------------------------------------------------------------
// Random documents
// ---------------------------------------------------------------------------
fn random_point(rng: &mut Rng, w: f32, h: f32) -> Point {
    Point::new(rng.range(-0.3 * w, 1.3 * w), rng.range(-0.3 * h, 1.3 * h))
}

fn random_rect(rng: &mut Rng, w: f32, h: f32) -> ImageRect {
    let p = random_point(rng, w, h);
    ImageRect::new(p.x, p.y, rng.range(-w, w), rng.range(-h, h))
}

fn random_color(rng: &mut Rng) -> Color {
    Color::new(rng.byte(), rng.byte(), rng.byte(), rng.byte())
}

fn random_kind(rng: &mut Rng, w: f32, h: f32) -> AnnotationKind {
    match rng.below(17) {
        0 => AnnotationKind::Measurement {
            measurement: Measurement {
                start: random_point(rng, w, h),
                end: random_point(rng, w, h),
                pixels_per_unit: rng.range(0.5, 3.0),
                scale_factor: rng.range(0.5, 3.0),
            },
        },
        1 => AnnotationKind::Line {
            start: random_point(rng, w, h),
            end: random_point(rng, w, h),
        },
        2 => AnnotationKind::Arrow {
            start: random_point(rng, w, h),
            end: random_point(rng, w, h),
            bend: rng.chance(1, 2).then(|| random_point(rng, w, h)),
            variant: [
                ArrowVariant::Solid,
                ArrowVariant::HandDrawn,
                ArrowVariant::Thin,
                ArrowVariant::DoubleEnded,
            ][rng.below(4) as usize],
        },
        3 => AnnotationKind::Rectangle {
            rect: random_rect(rng, w, h),
        },
        4 => AnnotationKind::Ellipse {
            rect: random_rect(rng, w, h),
        },
        5 => AnnotationKind::Text {
            origin: random_point(rng, w, h),
            text: ["", "Hi", "é漢🙂", "line\nbreak", "a b c d e f"][rng.below(5) as usize].into(),
            font_size: rng.range(1.0, 60.0),
        },
        6 => AnnotationKind::Counter {
            center: random_point(rng, w, h),
            value: rng.below(10_000) as u32,
            font_size: rng.range(1.0, 60.0),
        },
        7 => AnnotationKind::Pixelate {
            rect: random_rect(rng, w, h),
            block_size: rng.below(20) as u32,
        },
        8 => AnnotationKind::Blur {
            rect: random_rect(rng, w, h),
            radius: rng.range(0.0, 8.0),
        },
        9 => AnnotationKind::Highlight {
            rect: random_rect(rng, w, h),
        },
        10 => AnnotationKind::Spotlight {
            rect: random_rect(rng, w, h),
        },
        11 => AnnotationKind::Freehand {
            points: (0..rng.below(20))
                .map(|_| random_point(rng, w, h))
                .collect(),
        },
        12 => AnnotationKind::Redaction {
            rect: random_rect(rng, w, h),
        },
        13 => AnnotationKind::RemoveFill {
            rect: random_rect(rng, w, h),
            sample: rng.chance(1, 2).then(|| random_point(rng, w, h)),
        },
        14 => {
            let (pw, ph) = (1 + rng.below(6) as u32, 1 + rng.below(6) as u32);
            let pixels = RgbaImage::from_fn(pw, ph, |_, _| random_color(rng).into());
            AnnotationKind::Image {
                origin: random_point(rng, w, h),
                png_bytes: png(&pixels),
                size: rng.chance(1, 2).then(|| ImageSize {
                    width: rng.below(40) as u32,
                    height: rng.below(40) as u32,
                }),
            }
        }
        _ => AnnotationKind::Magnifier {
            rect: random_rect(rng, w, h),
            zoom: rng.range(0.5, 20.0),
            source: rng.chance(1, 2).then(|| random_point(rng, w, h)),
        },
    }
}

fn random_style(rng: &mut Rng) -> AnnotationStyle {
    AnnotationStyle {
        stroke: random_color(rng),
        fill: rng.chance(1, 2).then(|| random_color(rng)),
        stroke_width: rng.range(0.0, 12.0),
    }
}

fn random_backdrop(rng: &mut Rng) -> Backdrop {
    Backdrop {
        padding: rng.below(40) as u32,
        corner_radius: rng.below(40) as u32,
        background: if rng.chance(1, 2) {
            Background::Solid {
                color: random_color(rng),
            }
        } else {
            Background::LinearGradient {
                start: random_color(rng),
                end: random_color(rng),
                angle_degrees: rng.range(-720.0, 720.0),
            }
        },
        shadow: rng.chance(1, 2).then(|| Shadow {
            offset: Point::new(rng.range(-20.0, 20.0), rng.range(-20.0, 20.0)),
            blur_radius: rng.range(0.0, 10.0),
            color: random_color(rng),
        }),
    }
}

fn random_document(rng: &mut Rng) -> Document {
    let (w, h) = (8 + rng.below(56) as u32, 8 + rng.below(40) as u32);
    let source = RgbaImage::from_fn(w, h, |x, y| {
        Rgba([
            x as u8 * 3,
            y as u8 * 5,
            77,
            if rng.chance(1, 8) { 128 } else { 255 },
        ])
    });
    let mut doc = Document::new(source);
    if rng.chance(1, 3) {
        doc.set_source_scale_factor(rng.range(0.5, 10.0));
    }
    for _ in 0..rng.below(25) {
        let (cw, ch) = (doc.width() as f32, doc.height() as f32);
        let ids: Vec<AnnotationId> = doc.annotations().iter().map(|a| a.id).collect();
        let pick = |rng: &mut Rng| ids[rng.below(ids.len() as u64) as usize];
        match rng.below(10) {
            0..=3 => {
                let kind = random_kind(rng, cw, ch);
                let style = random_style(rng);
                doc.add_annotation(kind, style);
            }
            4 if !ids.is_empty() => {
                let id = pick(rng);
                let _ = doc.move_annotation(
                    id,
                    Point::new(rng.range(-80.0, 80.0), rng.range(-80.0, 80.0)),
                );
            }
            5 if !ids.is_empty() => {
                let id = pick(rng);
                let p = random_point(rng, cw, ch);
                let _ = doc.resize_annotation(
                    id,
                    ImageRect::new(p.x, p.y, rng.range(0.5, cw), rng.range(0.5, ch)),
                );
            }
            6 if !ids.is_empty() => {
                let id = pick(rng);
                let kind = random_kind(rng, cw, ch);
                let style = random_style(rng);
                let _ = doc.update_annotation(id, kind, style);
            }
            7 => doc.set_crop(random_rect(rng, cw, ch)),
            8 => {
                doc.expand_canvas_to(
                    (cw * rng.range(1.0, 2.0)) as u32,
                    (ch * rng.range(1.0, 2.0)) as u32,
                );
            }
            9 => doc.set_backdrop(random_backdrop(rng)),
            _ => {
                if rng.chance(1, 2) {
                    doc.undo();
                } else {
                    doc.redo();
                }
            }
        }
    }
    doc
}

/// A project with one annotation of every kind, used by the mutation tests.
fn rich_document() -> Document {
    let mut doc = Document::new(base_image());
    doc.set_source_scale_factor(2.0);
    doc.expand_canvas_to(80, 60);
    let style = AnnotationStyle {
        stroke: Color::RED,
        fill: Some(Color::new(10, 20, 30, 128)),
        stroke_width: 3.0,
    };
    let kinds = vec![
        AnnotationKind::Measurement {
            measurement: Measurement {
                start: Point::new(2.0, 40.0),
                end: Point::new(50.0, 40.0),
                pixels_per_unit: 2.0,
                scale_factor: 2.0,
            },
        },
        AnnotationKind::Line {
            start: Point::new(1.0, 2.0),
            end: Point::new(30.0, 20.0),
        },
        AnnotationKind::Arrow {
            start: Point::new(5.0, 5.0),
            end: Point::new(40.0, 30.0),
            bend: Some(Point::new(30.0, 5.0)),
            variant: ArrowVariant::DoubleEnded,
        },
        AnnotationKind::Rectangle {
            rect: ImageRect::new(4.0, 4.0, 20.0, 10.0),
        },
        AnnotationKind::Ellipse {
            rect: ImageRect::new(10.0, 10.0, 20.0, 12.0),
        },
        AnnotationKind::Text {
            origin: Point::new(3.0, 3.0),
            text: "Hi é".into(),
            font_size: 12.0,
        },
        AnnotationKind::Counter {
            center: Point::new(50.0, 10.0),
            value: 7,
            font_size: 14.0,
        },
        AnnotationKind::Pixelate {
            rect: ImageRect::new(0.0, 0.0, 16.0, 16.0),
            block_size: 4,
        },
        AnnotationKind::Blur {
            rect: ImageRect::new(30.0, 30.0, 16.0, 16.0),
            radius: 2.0,
        },
        AnnotationKind::Highlight {
            rect: ImageRect::new(20.0, 20.0, 10.0, 5.0),
        },
        AnnotationKind::Spotlight {
            rect: ImageRect::new(5.0, 5.0, 40.0, 30.0),
        },
        AnnotationKind::Freehand {
            points: vec![
                Point::new(1.0, 1.0),
                Point::new(5.0, 9.0),
                Point::new(12.0, 3.0),
            ],
        },
        AnnotationKind::Redaction {
            rect: ImageRect::new(40.0, 2.0, 8.0, 4.0),
        },
        AnnotationKind::RemoveFill {
            rect: ImageRect::new(44.0, 30.0, 6.0, 6.0),
            sample: Some(Point::new(43.0, 29.0)),
        },
        AnnotationKind::Image {
            origin: Point::new(60.0, 40.0),
            png_bytes: png(&RgbaImage::from_pixel(3, 2, Rgba([0, 200, 90, 255]))),
            size: Some(ImageSize {
                width: 6,
                height: 4,
            }),
        },
        AnnotationKind::Magnifier {
            rect: ImageRect::new(50.0, 30.0, 20.0, 20.0),
            zoom: 3.0,
            source: Some(Point::new(10.0, 10.0)),
        },
        AnnotationKind::Magnifier {
            rect: ImageRect::new(20.0, 30.0, 10.0, 10.0),
            zoom: 2.0,
            source: None,
        },
    ];
    for kind in kinds {
        doc.add_annotation(kind, style);
    }
    doc.set_crop(ImageRect::new(1.0, 1.0, 70.0, 50.0));
    doc.set_backdrop(Backdrop {
        padding: 6,
        corner_radius: 4,
        background: Background::LinearGradient {
            start: Color::BLACK,
            end: Color::WHITE,
            angle_degrees: 30.0,
        },
        shadow: Some(Shadow {
            offset: Point::new(2.0, 3.0),
            blur_radius: 2.0,
            color: Color::new(0, 0, 0, 100),
        }),
    });
    doc
}

fn rich_json() -> String {
    rich_document()
        .to_project("source.png")
        .to_json_pretty()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Child-process runner, so an abort or hang in one case does not end the run.
// ---------------------------------------------------------------------------

/// Loads a project JSON against `base_image()` and exercises open, render, save, reload.
fn exercise(json: &str) -> String {
    let project = match Project::from_json(json) {
        Ok(project) => project,
        Err(_) => return "parse_rejected".into(),
    };
    let doc = match Document::from_project(project, base_image()) {
        Ok(doc) => doc,
        Err(_) => return "open_rejected".into(),
    };
    for annotation in doc.annotations() {
        let _ = annotation.kind.bounds();
    }
    let _ = doc.hit_test(Point::new(10.0, 10.0), 2.0);
    let rendered = doc.render(&options());
    let saved = match doc.to_project("source.png").to_json_pretty() {
        Ok(saved) => saved,
        Err(error) => return format!("SAVE_FAILED: {error}"),
    };
    if let Err(error) =
        Project::from_json(&saved).and_then(|p| Document::from_project(p, base_image()))
    {
        return format!("SAVE_THEN_RELOAD_FAILS: {error}");
    }
    match rendered {
        Ok(_) => "ok".into(),
        Err(error) => format!("opens_but_render_err: {error}"),
    }
}

#[test]
#[ignore]
fn child_case() {
    let Ok(path) = std::env::var("FUZZ_CASE_FILE") else {
        return;
    };
    let json = std::fs::read_to_string(path).unwrap();
    println!("OUTCOME={}", exercise(&json));
}

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_child(json: &str) -> String {
    let n = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("sniplet-fuzz-{}-{n}.json", std::process::id()));
    std::fs::write(&path, json).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "child_case",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("FUZZ_CASE_FILE", &path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if start.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let _ = std::fs::remove_file(&path);
    let Some(status) = status else {
        return "HANG (>20s)".into();
    };
    if let Some(index) = stdout.find("OUTCOME=") {
        return stdout[index + 8..].lines().next().unwrap_or("").to_string();
    }
    if let Some(line) = stderr
        .lines()
        .find(|line| line.contains("memory allocation of"))
    {
        return format!("ABORT: {}", line.trim());
    }
    if let Some(index) = stderr.find("panicked at") {
        let message: String = stderr[index..]
            .lines()
            .take(2)
            .collect::<Vec<_>>()
            .join(" | ");
        return format!("PANIC: {message}");
    }
    format!(
        "CRASH: exit {:?} {}",
        status.code(),
        stderr.lines().last().unwrap_or("")
    )
}

/// Runs cases on four worker threads and returns outcomes in order.
fn run_children(cases: &[(String, String)]) -> Vec<String> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(vec![String::new(); cases.len()]);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((_, json)) = cases.get(index) else {
                        break;
                    };
                    let outcome = run_child(json);
                    results.lock().unwrap()[index] = outcome;
                }
            });
        }
    });
    results.into_inner().unwrap()
}

fn is_bad(outcome: &str) -> bool {
    ["ABORT", "PANIC", "HANG", "CRASH", "SAVE_"]
        .iter()
        .any(|prefix| outcome.starts_with(prefix))
}

fn report(title: &str, cases: &[(String, String)], outcomes: &[String]) -> usize {
    let mut summary = std::collections::BTreeMap::<String, usize>::new();
    for outcome in outcomes {
        let class = outcome.split(':').next().unwrap_or("").to_string();
        *summary.entry(class).or_default() += 1;
    }
    println!("\n=== {title}: {} cases -> {summary:?}", cases.len());
    let mut bad = 0;
    for ((label, _), outcome) in cases.iter().zip(outcomes) {
        if is_bad(outcome) || outcome.starts_with("opens_but") {
            if is_bad(outcome) {
                bad += 1;
            }
            println!("  {label:<60} {outcome}");
        }
    }
    bad
}

// ---------------------------------------------------------------------------
// 1. Random document save -> load round trip.
// ---------------------------------------------------------------------------
#[test]
fn fuzz_random_document_round_trip() {
    install_quiet_hook();
    let mut problems = Vec::new();
    let (mut reload_diff, mut pixel_diff) = (0, 0);
    for seed in 0..400u64 {
        let mut rng = Rng::new(seed);
        let result = guarded(|| {
            let mut doc = random_document(&mut rng);
            let project = doc.to_project("source.png");
            let json = project.to_json_pretty().map_err(|e| format!("save: {e}"))?;
            let parsed = Project::from_json(&json).map_err(|e| format!("SAVE_THEN_RELOAD: {e}"))?;
            if parsed != project {
                return Err("json round trip changed the project".to_string());
            }
            let reloaded = Document::from_project(parsed, doc.original().clone())
                .map_err(|e| format!("open: {e}"))?;
            let mut notes = Vec::new();
            if reloaded.to_project("source.png") != project {
                notes.push("reload_changes_project");
            }
            let before = doc.render(&options()).map_err(|e| format!("render: {e}"))?;
            let after = reloaded
                .render(&options())
                .map_err(|e| format!("render reloaded: {e}"))?;
            if before != after {
                notes.push("reload_changes_pixels");
            }
            // Undo everything, then redo everything: must reach the same state.
            if doc.can_redo() {
                return Ok(notes);
            }
            while doc.undo() {}
            while doc.redo() {}
            if doc.to_project("source.png") != project {
                return Err("undo-all/redo-all changed the project".to_string());
            }
            Ok(notes)
        });
        match result {
            Ok(Ok(notes)) => {
                if notes.contains(&"reload_changes_project") {
                    reload_diff += 1;
                }
                if notes.contains(&"reload_changes_pixels") {
                    pixel_diff += 1;
                    if pixel_diff <= 5 {
                        println!("  seed {seed}: reload changes rendered pixels");
                    }
                }
            }
            Ok(Err(error)) => problems.push(format!("seed {seed}: {error}")),
            Err(panic) => problems.push(format!("seed {seed}: PANIC {panic}")),
        }
    }
    println!(
        "\n=== round trip: 400 docs, {} problems, reload changed project in {reload_diff}, changed pixels in {pixel_diff}",
        problems.len()
    );
    for problem in &problems {
        println!("  {problem}");
    }
    assert!(problems.is_empty());
}

// ---------------------------------------------------------------------------
// 2. Truncated JSON at every byte.
// ---------------------------------------------------------------------------
#[test]
fn fuzz_truncated_json_every_byte() {
    install_quiet_hook();
    let json = rich_json();
    assert_eq!(exercise(&json), "ok");
    let mut problems = Vec::new();
    let mut accepted = 0;
    for end in 0..json.len() {
        if !json.is_char_boundary(end) {
            continue;
        }
        let prefix = &json[..end];
        match guarded(|| Project::from_json(prefix).is_ok()) {
            Ok(true) => {
                accepted += 1;
                match guarded(|| exercise(prefix)) {
                    Ok(outcome) if is_bad(&outcome) => problems.push(format!("{end}: {outcome}")),
                    Ok(_) => {}
                    Err(panic) => problems.push(format!("{end}: PANIC {panic}")),
                }
            }
            Ok(false) => {}
            Err(panic) => problems.push(format!("{end}: PANIC {panic}")),
        }
    }
    println!(
        "\n=== truncation: {} prefixes, {accepted} accepted, {} problems",
        json.len(),
        problems.len()
    );
    for problem in &problems {
        println!("  {problem}");
    }
    assert!(problems.is_empty());
}

// ---------------------------------------------------------------------------
// 3. Extreme and non-finite numbers in every numeric field (child processes).
// ---------------------------------------------------------------------------
fn number_pointers(value: &Value, pointer: String, out: &mut Vec<String>) {
    match value {
        Value::Number(_) => out.push(pointer),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                number_pointers(item, format!("{pointer}/{index}"), out);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if key == "png_bytes" {
                    continue;
                }
                number_pointers(item, format!("{pointer}/{key}"), out);
            }
        }
        _ => {}
    }
}

fn extreme_values() -> Vec<(&'static str, Value)> {
    vec![
        ("-1", Value::from(-1)),
        ("0", Value::from(0)),
        ("1e-45", Value::from(1e-45_f64)),
        ("65536", Value::from(65_536)),
        ("1000000", Value::from(1_000_000)),
        ("u32::MAX", Value::from(u32::MAX)),
        ("2^32", Value::from(1_u64 << 32)),
        ("3.4e38", Value::from(3.4e38_f64)),
        ("1e39(f32 inf)", Value::from(1e39_f64)),
        ("-1e39(f32 -inf)", Value::from(-1e39_f64)),
    ]
}

#[test]
fn fuzz_extreme_numbers_every_field() {
    let base: Value = serde_json::from_str(&rich_json()).unwrap();
    let mut pointers = Vec::new();
    number_pointers(&base, String::new(), &mut pointers);
    let mut cases = Vec::new();
    for pointer in &pointers {
        for (label, value) in extreme_values() {
            let mut mutated = base.clone();
            *mutated.pointer_mut(pointer).unwrap() = value;
            cases.push((
                format!("{pointer} = {label}"),
                serde_json::to_string_pretty(&mutated).unwrap(),
            ));
        }
    }
    let outcomes = run_children(&cases);
    let bad = report("extreme numbers", &cases, &outcomes);
    assert_eq!(bad, 0);
}

// ---------------------------------------------------------------------------
// 4. Oversized dimensions and edits with non-finite values.
// ---------------------------------------------------------------------------
fn with(mutate: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(&rich_json()).unwrap();
    mutate(&mut value);
    serde_json::to_string_pretty(&value).unwrap()
}

fn annotation_index(value: &Value, kind: &str) -> usize {
    value["annotations"]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a["kind"]["type"] == kind)
        .unwrap()
}

#[test]
fn fuzz_oversized_dimensions() {
    let image_index = annotation_index(&serde_json::from_str(&rich_json()).unwrap(), "image");
    let text_index = annotation_index(&serde_json::from_str(&rich_json()).unwrap(), "text");
    let counter_index = annotation_index(&serde_json::from_str(&rich_json()).unwrap(), "counter");
    let blur_index = annotation_index(&serde_json::from_str(&rich_json()).unwrap(), "blur");
    let pixelate_index = annotation_index(&serde_json::from_str(&rich_json()).unwrap(), "pixelate");
    let cases: Vec<(String, String)> = vec![
        (
            "canvas u32::MAX x u32::MAX",
            with(|v| v["canvas_size"] = serde_json::json!({"width": u32::MAX, "height": u32::MAX})),
        ),
        (
            "canvas 100000 x 100000",
            with(|v| v["canvas_size"] = serde_json::json!({"width": 100_000, "height": 100_000})),
        ),
        (
            "canvas 1000000 x 64",
            with(|v| v["canvas_size"] = serde_json::json!({"width": 1_000_000, "height": 64})),
        ),
        (
            "canvas 20000 x 20000 (1.6 GB)",
            with(|v| v["canvas_size"] = serde_json::json!({"width": 20_000, "height": 20_000})),
        ),
        (
            "backdrop padding u32::MAX",
            with(|v| v["backdrop"]["padding"] = Value::from(u32::MAX)),
        ),
        (
            "backdrop padding 100000",
            with(|v| v["backdrop"]["padding"] = Value::from(100_000)),
        ),
        (
            "backdrop corner_radius u32::MAX",
            with(|v| v["backdrop"]["corner_radius"] = Value::from(u32::MAX)),
        ),
        (
            "shadow blur_radius 1e30",
            with(|v| v["backdrop"]["shadow"]["blur_radius"] = Value::from(1e30)),
        ),
        (
            "shadow offset 1e30",
            with(|v| v["backdrop"]["shadow"]["offset"]["x"] = Value::from(1e30)),
        ),
        (
            "image size 100000 x 100000",
            with(|v| {
                v["annotations"][image_index]["kind"]["size"] =
                    serde_json::json!({"width": 100_000, "height": 100_000})
            }),
        ),
        (
            "image size u32::MAX x 1",
            with(|v| {
                v["annotations"][image_index]["kind"]["size"] =
                    serde_json::json!({"width": u32::MAX, "height": 1})
            }),
        ),
        (
            "image size None",
            with(|v| v["annotations"][image_index]["kind"]["size"] = Value::Null),
        ),
        (
            "image origin 1e30",
            with(|v| v["annotations"][image_index]["kind"]["origin"]["x"] = Value::from(1e30)),
        ),
        (
            "text font_size 1e6",
            with(|v| v["annotations"][text_index]["kind"]["font_size"] = Value::from(1e6)),
        ),
        (
            "text font_size 3e38",
            with(|v| v["annotations"][text_index]["kind"]["font_size"] = Value::from(3e38)),
        ),
        (
            "counter font_size 1e6",
            with(|v| v["annotations"][counter_index]["kind"]["font_size"] = Value::from(1e6)),
        ),
        (
            "counter font_size 3e9",
            with(|v| v["annotations"][counter_index]["kind"]["font_size"] = Value::from(3e9)),
        ),
        (
            "blur radius 1e30",
            with(|v| v["annotations"][blur_index]["kind"]["radius"] = Value::from(1e30)),
        ),
        (
            "blur radius 1e6",
            with(|v| v["annotations"][blur_index]["kind"]["radius"] = Value::from(1e6)),
        ),
        (
            "pixelate block u32::MAX",
            with(|v| {
                v["annotations"][pixelate_index]["kind"]["block_size"] = Value::from(u32::MAX)
            }),
        ),
        (
            "stroke_width 1e30 (all)",
            with(|v| {
                for a in v["annotations"].as_array_mut().unwrap() {
                    a["style"]["stroke_width"] = Value::from(1e30)
                }
            }),
        ),
        (
            "stroke_width 1e6 (all)",
            with(|v| {
                for a in v["annotations"].as_array_mut().unwrap() {
                    a["style"]["stroke_width"] = Value::from(1e6)
                }
            }),
        ),
        (
            "duplicate annotation ids",
            with(|v| {
                for a in v["annotations"].as_array_mut().unwrap() {
                    a["id"] = Value::from(1)
                }
            }),
        ),
    ]
    .into_iter()
    .map(|(label, json)| (label.to_string(), json))
    .collect();
    let outcomes = run_children(&cases);
    report("oversized", &cases, &outcomes);
    for ((label, _), outcome) in cases.iter().zip(&outcomes) {
        println!("  {label:<60} {outcome}");
    }
    assert!(outcomes.iter().all(|o| !is_bad(o)));
}

#[test]
fn fuzz_non_finite_edits_survive_save_and_load() {
    install_quiet_hook();
    let bad_values = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX];
    let mut problems = Vec::new();
    let mut check = |label: String, edit: &dyn Fn(&mut Document)| {
        let result = guarded(|| {
            let mut doc = rich_document();
            edit(&mut doc);
            let json = doc
                .to_project("source.png")
                .to_json_pretty()
                .map_err(|e| e.to_string())?;
            Project::from_json(&json)
                .and_then(|p| Document::from_project(p, base_image()))
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => problems.push(format!("{label}: SAVE_THEN_RELOAD_FAILS {error}")),
            Err(panic) => problems.push(format!("{label}: PANIC {panic}")),
        }
    };
    for value in bad_values {
        check(format!("move_annotation dx={value}"), &|doc| {
            let id = doc.annotations()[3].id;
            let _ = doc.move_annotation(id, Point::new(value, 0.0));
        });
        check(format!("move twice by f32::MAX/2 then {value}"), &|doc| {
            let id = doc.annotations()[1].id;
            let _ = doc.move_annotation(id, Point::new(f32::MAX, 0.0));
            let _ = doc.move_annotation(id, Point::new(f32::MAX, 0.0));
        });
        check(format!("add rect x={value}"), &|doc| {
            doc.add_annotation(
                AnnotationKind::Rectangle {
                    rect: ImageRect::new(value, 0.0, 5.0, 5.0),
                },
                AnnotationStyle::default(),
            );
        });
        check(format!("add line start.x={value}"), &|doc| {
            doc.add_annotation(
                AnnotationKind::Line {
                    start: Point::new(value, 0.0),
                    end: Point::new(5.0, 5.0),
                },
                AnnotationStyle::default(),
            );
        });
        check(format!("add text font_size={value}"), &|doc| {
            doc.add_annotation(
                AnnotationKind::Text {
                    origin: Point::new(1.0, 1.0),
                    text: "x".into(),
                    font_size: value,
                },
                AnnotationStyle::default(),
            );
        });
        check(format!("add style stroke_width={value}"), &|doc| {
            doc.add_annotation(
                AnnotationKind::Rectangle {
                    rect: ImageRect::new(1.0, 1.0, 5.0, 5.0),
                },
                AnnotationStyle {
                    stroke_width: value,
                    ..AnnotationStyle::default()
                },
            );
        });
        check(format!("update freehand point={value}"), &|doc| {
            let annotation = doc.annotations()[11].clone();
            let _ = doc.update_annotation(
                annotation.id,
                AnnotationKind::Freehand {
                    points: vec![Point::new(value, value)],
                },
                annotation.style,
            );
        });
        check(format!("resize to width {value}"), &|doc| {
            let id = doc.annotations()[5].id;
            let _ = doc.resize_annotation(id, ImageRect::new(1.0, 1.0, value, 10.0));
        });
        check(format!("set_backdrop blur={value}"), &|doc| {
            let mut backdrop = doc.backdrop();
            backdrop.shadow.as_mut().unwrap().blur_radius = value;
            doc.set_backdrop(backdrop);
        });
        check(format!("set_backdrop angle={value}"), &|doc| {
            let mut backdrop = doc.backdrop();
            backdrop.background = Background::LinearGradient {
                start: Color::BLACK,
                end: Color::WHITE,
                angle_degrees: value,
            };
            doc.set_backdrop(backdrop);
        });
        check(format!("set_crop x={value}"), &|doc| {
            doc.set_crop(ImageRect::new(value, 0.0, 10.0, 10.0));
        });
        check(format!("set_source_scale_factor {value}"), &|doc| {
            doc.set_source_scale_factor(value);
        });
        check(format!("magnifier move circle {value}"), &|doc| {
            let annotation = doc.annotations()[15].clone();
            let mut kind = annotation.kind.clone();
            kind.move_magnifier_circle(sniplet_core::MagnifierPart::Source, Point::new(value, 0.0));
            let _ = doc.update_annotation(annotation.id, kind, annotation.style);
        });
    }
    println!("\n=== non-finite edits: {} problems", problems.len());
    for problem in &problems {
        println!("  {problem}");
    }
    assert!(problems.is_empty());
}

// ---------------------------------------------------------------------------
// 5. Image decode: truncated, corrupt, random, and bomb images.
// ---------------------------------------------------------------------------
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A PNG whose IHDR claims `width` x `height` RGBA8, with a tiny IDAT.
fn png_bomb(width: u32, height: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    // zlib stream of a few zero bytes, stored block.
    let data = [0u8; 16];
    let mut idat = vec![0x78, 0x01, 0x01, 16, 0, !16u8, 0xFF];
    idat.extend_from_slice(&data);
    idat.extend_from_slice(&[0, 0x10, 0, 1]); // adler32 of 16 zeros = 0x00100001
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn encoded_samples() -> Vec<(&'static str, Vec<u8>)> {
    let mut rng = Rng::new(42);
    let image = RgbaImage::from_fn(37, 23, |_, _| random_color(&mut rng).into());
    let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 80)
        .write_image(rgb.as_raw(), 37, 23, ExtendedColorType::Rgb8)
        .unwrap();
    let mut webp = Vec::new();
    WebPEncoder::new_lossless(&mut webp)
        .write_image(image.as_raw(), 37, 23, ExtendedColorType::Rgba8)
        .unwrap();
    vec![("png", png(&image)), ("jpeg", jpeg), ("webp", webp)]
}

fn image_annotation_doc(bytes: Vec<u8>, size: Option<ImageSize>) -> Document {
    let mut doc = Document::new(base_image());
    doc.add_annotation(
        AnnotationKind::Image {
            origin: Point::new(1.0, 1.0),
            png_bytes: bytes,
            size,
        },
        AnnotationStyle::default(),
    );
    doc
}

#[test]
fn fuzz_decode_truncated_corrupt_random() {
    install_quiet_hook();
    let mut problems = Vec::new();
    let mut probe = |label: String, bytes: &[u8], render: bool| {
        let started = Instant::now();
        if let Err(panic) = guarded(|| decode_image(bytes).map(|_| ())) {
            problems.push(format!("{label}: decode PANIC {panic}"));
        }
        if render {
            let doc = image_annotation_doc(bytes.to_vec(), None);
            if let Err(panic) = guarded(|| {
                let _ = doc.annotations()[0].kind.bounds();
                doc.render(&options()).map(|_| ())
            }) {
                problems.push(format!("{label}: render PANIC {panic}"));
            }
        }
        if started.elapsed() > Duration::from_secs(2) {
            problems.push(format!("{label}: SLOW {:?}", started.elapsed()));
        }
    };
    let mut cases = 0;
    for (format, bytes) in encoded_samples() {
        for end in 0..bytes.len() {
            probe(
                format!("{format} truncated at {end}"),
                &bytes[..end],
                end % 7 == 0,
            );
            cases += 1;
        }
        let mut rng = Rng::new(7);
        for round in 0..3000 {
            let mut mutated = bytes.clone();
            for _ in 0..1 + rng.below(8) {
                let index = rng.below(mutated.len() as u64) as usize;
                mutated[index] = rng.byte();
            }
            probe(
                format!("{format} mutation {round}"),
                &mutated,
                round % 10 == 0,
            );
            cases += 1;
        }
    }
    let mut rng = Rng::new(9);
    let magics: [&[u8]; 4] = [
        b"",
        b"\x89PNG\r\n\x1a\n",
        b"\xFF\xD8\xFF",
        b"RIFF\0\0\0\0WEBPVP8L",
    ];
    for round in 0..600 {
        let mut blob = magics[round % 4].to_vec();
        for _ in 0..rng.below(512) {
            blob.push(rng.byte());
        }
        probe(format!("random blob {round}"), &blob, round % 5 == 0);
        cases += 1;
    }
    println!(
        "\n=== decode fuzz: {cases} inputs, {} problems",
        problems.len()
    );
    for problem in &problems {
        println!("  {problem}");
    }
    assert!(problems.is_empty());
}

#[test]
fn fuzz_decode_bombs() {
    install_quiet_hook();
    let mut lines = Vec::new();
    let mut bombs: Vec<(String, Vec<u8>)> = [
        (60_000, 60_000),
        (u32::MAX, 1),
        (1, u32::MAX),
        (0, 0),
        (2_147_483_647, 1),
        (65_535, 65_535),
        (100_000, 1),
        (16_384, 8_192),
    ]
    .into_iter()
    .map(|(w, h)| (format!("png IHDR {w}x{h}"), png_bomb(w, h)))
    .collect();
    let samples = encoded_samples();
    let mut jpeg = samples[1].1.clone();
    let sof = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    jpeg[sof + 5..sof + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    bombs.push(("jpeg SOF 65535x65535".into(), jpeg));
    let mut webp = samples[2].1.clone();
    // VP8L: signature byte at 20, then 14 bits width-1 and 14 bits height-1.
    let bits: u32 = 16_383 | (16_383 << 14) | (u32::from(webp[24]) & 0xF0) << 24;
    webp[21..25].copy_from_slice(&bits.to_le_bytes());
    bombs.push(("webp VP8L 16384x16384".into(), webp));
    for (label, bytes) in &bombs {
        let started = Instant::now();
        let outcome = match guarded(|| decode_image(bytes)) {
            Ok(Ok(image)) => format!("DECODED {}x{}", image.width(), image.height()),
            Ok(Err(error)) => format!("rejected: {error}"),
            Err(panic) => format!("PANIC {panic}"),
        };
        lines.push(format!(
            "{label:<28} {outcome} ({:?}, {} byte file)",
            started.elapsed(),
            bytes.len()
        ));
    }

    // Grayscale PNG just above the RGBA budget: 12000x12000 L8 zeros.
    let (w, h) = (12_000u32, 12_000u32);
    let started = Instant::now();
    let gray = vec![0u8; (w * h) as usize];
    let mut encoded = Vec::new();
    PngEncoder::new_with_quality(&mut encoded, CompressionType::Fast, FilterType::NoFilter)
        .write_image(&gray, w, h, ExtendedColorType::L8)
        .unwrap();
    drop(gray);
    lines.push(format!(
        "encoded L8 {w}x{h} to {} bytes in {:?}",
        encoded.len(),
        started.elapsed()
    ));
    let started = Instant::now();
    match decode_image(&encoded) {
        Ok(image) => lines.push(format!(
            "L8 {w}x{h}: decode_image OK, RGBA buffer {} MiB vs 512 MiB decoder limit ({:?})",
            image.as_raw().len() >> 20,
            started.elapsed()
        )),
        Err(error) => lines.push(format!("L8 {w}x{h}: rejected: {error}")),
    }
    // Issue 5: bounds() with size None decodes the whole image.
    let doc = image_annotation_doc(encoded, None);
    let started = Instant::now();
    let bounds = doc.annotations()[0].kind.bounds();
    lines.push(format!(
        "bounds() with size None on L8 {w}x{h}: {bounds:?} took {:?}",
        started.elapsed()
    ));

    println!("\n=== decode bombs");
    for line in &lines {
        println!("  {line}");
    }
    assert!(lines.iter().all(|line| !line.contains("PANIC")));
}

#[test]
fn debug_reload_pixel_diff() {
    for seed in [58u64, 84, 104, 137, 155] {
        let mut rng = Rng::new(seed);
        let doc = random_document(&mut rng);
        let project = doc.to_project("source.png");
        let reloaded = Document::from_project(project.clone(), doc.original().clone()).unwrap();
        let again = reloaded.to_project("source.png");
        // Find the annotation(s) whose removal makes pixels equal.
        let before = doc.render(&options()).unwrap();
        let after = reloaded.render(&options()).unwrap();
        let diff = before
            .pixels()
            .zip(after.pixels())
            .filter(|(a, b)| a != b)
            .count();
        println!(
            "seed {seed}: {diff} px differ, canvas {:?}, crop {:?}",
            doc.dimensions(),
            doc.crop()
        );
        for (a, b) in project.annotations.iter().zip(&again.annotations) {
            if a != b {
                println!("   {:?}\n -> {:?}", a.kind, b.kind);
            }
        }
        if project.crop != again.crop {
            println!("   crop {:?} -> {:?}", project.crop, again.crop);
        }
    }
}
