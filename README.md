# Sniplet



## Run

Clone the repository:

```sh
git clone https://github.com/m4tta/sniplet.git
cd sniplet
```

Install stable Rust and the native prerequisites below, then:

```sh
cargo run -p sniplet-app -- --demo
cargo run -p sniplet-app -- path/to/image.png
cargo run -p sniplet-app
```

Capture from the tray menu or use Ctrl/Cmd + Shift + 1 (area), 2 (screen),
3 (window), 4 (scrolling), 5 (repeat area), 6 (active window), or 7
(capture text/QR). Ctrl/Cmd + Shift + 8 reopens the editor. Closing the editor
keeps Sniplet in the tray when the desktop provides one. On macOS, it hides the
window and removes Sniplet from the Dock. Reopen Sniplet restores the same image.
Quit Sniplet or Ctrl/Cmd+Q exits.

The editor supports selection/crop, arrows, lines, text, rectangles, ovals,
freehand, counters, rulers, blur, pixelation, highlight, spotlight, erase,
magnifiers, backdrops, pasted images, and added captures. New magnifiers show a
source circle and a larger lens joined by a line. Move either circle with its
center handle; use its right-edge handle to change size. Press A for Arrow;
arrows start with a bold, smooth solid style. The arrow palette offers Solid,
Hand drawn, Thin, and Double ended styles, plus a tapered size slider (1–30).
Size changes preview live; one drag is one undo step. Drag the middle handle to bend an
arrow, or either endpoint to reposition it. Drag its shaft to move the whole
arrow. Draw, then select with V to move or resize other annotations.

Hold 1 or Left/Right to measure a width. Hold 2 or Up/Down to measure a height.
Move the pointer over an area or gap, then click to place the red ruler and label.
Hold Shift for outer edges. Use the wheel to adjust sensitivity. Click the image
size to switch between points and physical pixels. Release the key or press
Escape to clear the live preview. Selected objects keep their arrow-key controls.

Double-click text to edit it; Alt-drag to
duplicate. Enter crops a raster selection; Escape cancels. Copy/export uses
the selection when present. Undo/redo preserves editing operations.

Right-drag or Space-drag pans. Ctrl/Cmd + wheel zooms at the pointer.
Ctrl/Cmd + 1 fits, 0 uses actual pixels, and 2 fits the selection. Z + click
zooms in (Alt zooms out). Tab copies the sampled color. The application menu
auto-adjusts a raster selection to visible content; Ctrl/Cmd-click selects a monotone region.
The application menu includes text recognition and QR decoding.

Save PNG/JPEG/WebP through the native save dialog. Ctrl/Cmd + Shift + S saves
an editable `.sniplet` project and its `.source.png` companion; keep them together.
Existing `.clippy` projects also open in Sniplet.
The application ID and preferences namespace are `io.github.m4tta.sniplet`.
Drag the dotted file button, labeled `Drag'n'Drop Image`, into Finder or another
application to export a PNG. It includes the current edits, or the selected area
when a raster selection is active. The button is disabled until an image is open.
Pin creates a movable floating image; wheel resizes it, its hover controls change
opacity or return it to the editor, and Escape/right-click closes it. Add Capture
places a new capture to the right on an expanded canvas as an editable object.

## Native prerequisites

- **Windows:** Visual Studio 2022 Build Tools with Desktop development with C++;
  a graphics driver supporting Direct3D. Windows OCR is used when available.
- **macOS:** Xcode with the macOS SDK and Metal toolchain; Screen Recording permission for capture.
  The current OCR fallback requires `tesseract` and its language data.
- **Linux:** Vulkan-capable graphics and X11 or Wayland. On Ubuntu/Debian install
  `build-essential clang cmake pkg-config libxcb1-dev libxkbcommon-dev
  libxkbcommon-x11-dev libwayland-dev libvulkan-dev libfontconfig1-dev
  libasound2-dev libssl-dev libx11-dev libxrandr-dev libdbus-1-dev
  libgbm-dev libegl-dev libpipewire-0.3-dev libclang-dev libwebkit2gtk-4.1-dev`.
  Install `tesseract-ocr` for text recognition. Capture support depends on
  the desktop/compositor; see the platform testing notes.

## Upload and preferences

Use the application menu → Settings for format, automatic clipboard copy,
display selection, editor behavior, and capture shortcuts. Shortcut changes
apply immediately. The macOS menu bar also offers Launch at Startup and
upward scrolling capture under More. Upload is disabled until a destination is configured.
Sniplet follows the system's light or dark appearance by default. Settings →
Appearance lets you choose System, Light, or Dark; changes apply immediately and
remain selected after restarting.
Capture Window (`Ctrl/Cmd + Shift + 3` by default) shows the desktop with a capture
cursor and camera marker while keeping the editor hidden. Point at a visible
window to highlight it, then click to capture it. Escape or right-click cancels
without reopening the editor.
Settings → Cloud upload accepts a signed PUT URL and an optional public image URL.
Click the cloud button to upload the current image and copy its link.

S3-compatible uploads can be configured in the settings file:

```json
{
  "cloud_upload": {
    "type": "s3",
    "bucket": "screenshots",
    "region": "us-west-2",
    "endpoint": null,
    "key_prefix": "sniplet",
    "public_base_url": "https://images.example.com"
  }
}
```

Merge this field into the existing settings. S3 credentials are read from
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and optional `AWS_SESSION_TOKEN`;
they are never written into settings. Without a public base URL, upload returns
a signed GET link valid for up to seven days.

## Package

Create a portable release on the current host:

```powershell
.\scripts\package.ps1
& .\dist\sniplet-x86_64-pc-windows-msvc\Sniplet.exe --demo
```

```sh
./scripts/package.sh
```

The shell script creates `dist/Sniplet.app` on macOS or a portable Linux directory
under `dist/`. Packages contain the executable, licenses, and documentation.
The app icon is included in the macOS bundle and Windows executable. Linux
packages include a desktop entry and icons under `share/icons/hicolor`; install
the `bin` and `share` contents into the matching directories in `~/.local` to
show Sniplet in your app launcher. The Windows system tray uses the same scissors
symbol as the macOS menu bar and changes color with the taskbar theme.

On macOS, the script uses a local signing certificate named `Sniplet Development`
if it is in your Keychain. Keep this certificate and its private key for later
builds so macOS can retain the app's capture permission. A local certificate does
not need an Apple developer account. If the certificate is absent, the script
uses ad hoc signing, which can require a new permission grant after a rebuild.
Set `SNIPLET_SIGNING_IDENTITY` to select a certificate name or SHA-1 fingerprint;
set it to `-` to use ad hoc signing. Keep private keys outside the repository.
Local signing is for development; it does not notarize a public release.

## Testing on a Mac

Install Xcode and select it as the active developer directory. The locked GPUI
dependency invokes `xcrun` to compile Metal shaders. Confirm the compiler is
available with `xcrun -sdk macosx metal --version`. If it reports a missing Metal
toolchain, install it through Xcode's Components settings or
`xcodebuild -downloadComponent MetalToolchain`; see
[Apple's component installation guide](https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components).

After installing Rust and the macOS prerequisites, run the automated checks and
create a debug app bundle for native testing:

```sh
rustup component add rustfmt clippy
bash scripts/verify.sh all
SNIPLET_PACKAGE_PROFILE=debug bash scripts/package.sh
open dist/Sniplet-debug.app
```

Allow screen capture when macOS requests permission. Test area capture with
Cmd+Shift+2, fullscreen capture with Cmd+Shift+1, and window capture with
Cmd+Shift+3. Escape should cancel either interactive capture and leave the
editor hidden; Cmd+Shift+8 reopens it. Press A in the editor to test arrows.

For the bounded startup and real capture diagnostics, run
`bash scripts/verify.sh all --native`. The native diagnostic writes results to
`artifacts/self-test`; the [platform testing guide](docs/platform-testing.md)
lists the remaining manual checks. Generated captures, private settings,
build outputs, and local research downloads are excluded from Git.

## Verification

```powershell
.\scripts\verify.ps1
.\scripts\verify.ps1 -Native
```

```sh
./scripts/verify.sh
./scripts/verify.sh --native
```

Generate a repeatable arrow rendering fixture for visual review:

```sh
cargo run -p sniplet-core --example arrow_gallery -- artifacts/arrow-gallery.png
```

Measure interactive rendering in an optimized build:

```sh
cargo run --release -p sniplet-core --example interactive_render_bench -- artifacts/interactive-performance/render-benchmark.csv
```

The benchmark covers arrow anchors, new-tool previews, and several annotations
at 1600×900, 2560×1440, and 3840×2160. Preview updates coalesce pointer events
into display frames, and replaced GPU images are released.
Pass an editable `.sniplet` project as the second argument to measure its actual
annotations; append `--project-only` to skip the synthetic workloads.

The scripts run formatting, strict linting, workspace tests, and GPUI pointer/
keyboard integration tests. Native mode launches a smoke-test window and runs
a real capture-to-PNG self-test; its output is in `artifacts/self-test`.
Upload tests use a loopback HTTP server and never send screenshots externally.

## Known differences



Platform qualification is a separate gap. Windows native capture, editor input,
and diagnostics have passed locally, while mixed-DPI hardware, tray/hotkey
lifecycle, cross-application clipboard behavior, and pin behavior still need
full desktop records. WSLg can open the editor through Wayland and X11, but its
capture backends are incompatible; physical Linux X11/Wayland desktops have not
been qualified. GitHub Actions builds and tests the workspace on all three
platforms. On an Apple Silicon Mac at commit `3fffbcf`, all 124 tests passed,
along with native screen/area capture, editor input, PNG export, clipboard paste
into Preview, and project save/reload. Window capture selected the wrong target,
and OCR was blocked by missing Tesseract. See the
[Mac test record](docs/platform-testing.md#observed-macos-run-october-4-2026).
The Mac window picker now excludes system layers. Its Dock/Preview test, 25
platform tests, and 49 app tests pass. Final packaged Preview capture needs the
Screen Recording grant for the new app ID. See the
[window picker fix record](docs/platform-testing.md#macos-window-picker-fix-october-4-2026).
The linked magnifier passed native Mac creation, circle movement, size changes,
factor adjustment, undo and PNG export. All 52 core and 50 app tests passed;
see the [magnifier test record](docs/platform-testing.md#macos-linked-magnifier-october-4-2026).
Exact visual parity still needs 1×/2× reference comparisons and golden-image review.

## Research and scope



This is an actively developed independent implementation. A claim of 100%
visual or behavioral parity would require reference comparisons and native
verification on all three platforms; that has not yet been established.

The bundled Noto Sans font is licensed under the SIL Open Font License;
see `assets/fonts/OFL.txt`.
