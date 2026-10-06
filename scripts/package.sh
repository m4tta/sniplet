#!/usr/bin/env sh
set -eu

workspace_root=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
output_name="${1:-dist}"
profile="${SNIPLET_PACKAGE_PROFILE:-release}"
app_id="io.github.m4tta.sniplet"

case "$output_name" in
    ""|/*|..|../*|*/..|*/../*)
        echo "output directory must be a relative path inside the workspace" >&2
        exit 2
        ;;
esac

case "$profile" in
    release)
        bundle_suffix=""
        ;;
    debug)
        bundle_suffix="-debug"
        ;;
    *)
        echo "SNIPLET_PACKAGE_PROFILE must be release or debug" >&2
        exit 2
        ;;
esac

output_root="$workspace_root/$output_name"
cd "$workspace_root"

if [ "${SNIPLET_PACKAGE_SKIP_BUILD:-0}" != "1" ]; then
    if [ "$profile" = "release" ]; then
        printf '> cargo build -p sniplet-app --release --locked\n'
        cargo build -p sniplet-app --release --locked
    else
        printf '> cargo build -p sniplet-app --locked\n'
        cargo build -p sniplet-app --locked
    fi
fi

target_dir=$(cargo metadata --format-version 1 --no-deps --locked \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
host_target=$(rustc -vV | sed -n 's/^host: //p')
app_version=$(awk '
    /^\[/ { in_package = ($0 == "[workspace.package]") }
    in_package && /^version = / {
        sub(/^version = "/, ""); sub(/".*$/, ""); print
    }
' Cargo.toml)

if [ -z "$target_dir" ] || [ -z "$host_target" ] || [ -z "$app_version" ]; then
    echo "could not determine Cargo target directory, Rust host target, or app version" >&2
    exit 1
fi

copy_documentation() {
    destination=$1
    mkdir -p "$destination"
    cp "$workspace_root"/docs/*.md "$destination/"
}

case "$(uname -s)" in
    Darwin)
        binary="$target_dir/$profile/sniplet"
        bundle="$output_root/Sniplet$bundle_suffix.app"
        [ -f "$binary" ] || { echo "$profile executable not found at $binary" >&2; exit 1; }

        signing_identity="${SNIPLET_SIGNING_IDENTITY:-}"
        if [ -z "$signing_identity" ]; then
            # Reuse only the local Sniplet certificate, never another developer identity.
            signing_identity=$(security find-certificate -c "Sniplet Development" -Z 2>/dev/null \
                | sed -n 's/^SHA-1 hash: //p')
            signing_identity="${signing_identity:--}"
        fi
        if [ "$signing_identity" = "-" ]; then
            echo "Using ad hoc signing; macOS permissions may need a new grant after rebuilding."
        fi

        rm -rf "$bundle"
        mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources/licenses"
        install -m 0755 "$binary" "$bundle/Contents/MacOS/Sniplet"
        cp "$workspace_root/README.md" "$bundle/Contents/Resources/README.md"
        cp "$workspace_root/LICENSE" "$bundle/Contents/Resources/LICENSE"
        cp "$workspace_root/assets/fonts/OFL.txt" "$bundle/Contents/Resources/licenses/NotoSans-OFL.txt"
        cp "$workspace_root/assets/icons/LICENSE-LUCIDE" "$bundle/Contents/Resources/licenses/Lucide-ISC.txt"
        cp "$workspace_root/assets/icons/sniplet.icns" "$bundle/Contents/Resources/Sniplet.icns"
        copy_documentation "$bundle/Contents/Resources/documentation"
        cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key><string>Sniplet</string>
  <key>CFBundleExecutable</key><string>Sniplet</string>
  <key>CFBundleIdentifier</key><string>$app_id</string>
  <key>CFBundleName</key><string>Sniplet</string>
  <key>CFBundleIconFile</key><string>Sniplet.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$app_version</string>
  <key>CFBundleVersion</key><string>$app_version</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSScreenCaptureUsageDescription</key><string>Sniplet captures screen regions and windows that you choose.</string>
</dict>
</plist>
PLIST
        codesign --force --timestamp=none --sign "$signing_identity" "$bundle"
        printf 'Created macOS application bundle: %s\n' "$bundle"
        ;;
    Linux)
        binary="$target_dir/$profile/sniplet"
        bundle="$output_root/sniplet-$host_target$bundle_suffix"
        [ -f "$binary" ] || { echo "$profile executable not found at $binary" >&2; exit 1; }

        rm -rf "$bundle"
        mkdir -p "$bundle/bin" \
            "$bundle/share/applications" \
            "$bundle/share/licenses/sniplet"
        install -m 0755 "$binary" "$bundle/bin/sniplet"
        cp "$workspace_root/README.md" "$bundle/README.md"
        cp "$workspace_root/LICENSE" "$bundle/LICENSE"
        cp "$workspace_root/assets/fonts/OFL.txt" "$bundle/share/licenses/sniplet/NotoSans-OFL.txt"
        cp "$workspace_root/assets/icons/LICENSE-LUCIDE" "$bundle/share/licenses/sniplet/Lucide-ISC.txt"
        icon_root="$bundle/share/icons/hicolor"
        mkdir -p "$icon_root/scalable/apps"
        cp "$workspace_root/assets/icons/sniplet.svg" "$icon_root/scalable/apps/$app_id.svg"
        for icon_size in 16 24 32 48 64 128 256 512; do
            mkdir -p "$icon_root/${icon_size}x${icon_size}/apps"
            cp "$workspace_root/assets/icons/sniplet-$icon_size.png" \
                "$icon_root/${icon_size}x${icon_size}/apps/$app_id.png"
        done
        copy_documentation "$bundle/share/doc/sniplet"
        cat > "$bundle/share/applications/$app_id.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Sniplet
Comment=Capture and annotate screenshots
Exec=sniplet
Icon=$app_id
Terminal=false
Categories=Graphics;Utility;
MimeType=image/png;image/jpeg;image/webp;
StartupWMClass=$app_id
DESKTOP
        chmod 0644 "$bundle/share/applications/$app_id.desktop"
        printf 'Created portable Linux package: %s\n' "$bundle"
        ;;
    *)
        echo "package.sh supports macOS and Linux hosts" >&2
        exit 1
        ;;
esac
