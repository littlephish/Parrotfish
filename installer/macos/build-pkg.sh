#!/bin/sh
# Builds Parrotfish on a Mac for both kinds of Mac, makes Parrotfish.app, signs it ad hoc and
# writes dist/Parrotfish-<version>.pkg, an installer package for /Applications, and
# dist/Parrotfish-<version>.dmg, a disk image to drag the app from; neither is signed with a
# certificate. Needs Rust (rustup), the Xcode command line tools and python3.
#
#   sh installer/macos/build-pkg.sh               both kinds of Mac, joined with lipo
#   sh installer/macos/build-pkg.sh --arm64-only  Apple processors only
#   sh installer/macos/build-pkg.sh --skip-build  package what target/<triple>/release holds
set -eu

cd "$(dirname "$0")/../.."
root="$PWD"
name="Parrotfish"
identifier="io.github.littlephish.Parrotfish"
minimum="11.0"
triples="aarch64-apple-darwin x86_64-apple-darwin"
build=1

for argument in "$@"; do
    case "$argument" in
        --arm64-only) triples="aarch64-apple-darwin" ;;
        --skip-build) build=0 ;;
        *) echo "unknown argument: $argument" >&2; exit 2 ;;
    esac
done

fail() {
    echo "error: $*" >&2
    exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "this script runs on a Mac; tools/macos_build.sh builds on Linux"
for tool in cargo lipo codesign sips iconutil pkgbuild productbuild plutil pkgutil hdiutil ditto shasum; do
    command -v "$tool" >/dev/null || fail "$tool was not found"
done

version=$(awk '
    /^\[workspace\.package\]/ { inside = 1; next }
    /^\[/ { inside = 0 }
    inside && /^version[ \t]*=/ { sub(/^[^"]*"/, ""); sub(/".*$/, ""); print; exit }
' Cargo.toml)
[ -n "$version" ] || fail "Cargo.toml has no version under [workspace.package]"

export MACOSX_DEPLOYMENT_TARGET="$minimum"

programs=""
for triple in $triples; do
    if [ "$build" = 1 ]; then
        if command -v rustup >/dev/null; then
            rustup target add "$triple"
        fi
        cargo build --release --locked -p ps-app --target "$triple"
    fi
    program="${CARGO_TARGET_DIR:-target}/$triple/release/ps-app"
    [ -f "$program" ] || fail "$program was not built"
    programs="$programs $program"
done

work="$root/dist/macos"
rm -rf "$work"
mkdir -p "$work"

# shellcheck disable=SC2086
lipo -create -output "$work/$name" $programs
chmod 755 "$work/$name"
echo "$name $version for: $(lipo -archs "$work/$name")"

app="$work/root/$name.app"
contents="$app/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources"
cp "$work/$name" "$contents/MacOS/$name"

sed -e "s/@VERSION@/$version/g" -e "s/@IDENTIFIER@/$identifier/g" -e "s/@MINIMUM@/$minimum/g" \
    installer/macos/Info.plist > "$contents/Info.plist"
plutil -lint "$contents/Info.plist" >/dev/null || fail "Info.plist is not a valid property list"
if grep -q "@[A-Z]*@" "$contents/Info.plist"; then
    fail "Info.plist still has a placeholder in it"
fi
printf 'APPL????' > "$contents/PkgInfo"

iconset="$work/$name.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256; do
    sips -z "$size" "$size" ps-app/ui/app-icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
done
for size in 16 32 128; do
    double=$((size * 2))
    sips -z "$double" "$double" ps-app/ui/app-icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$contents/Resources/$name.icns"

cp LICENSE-MIT LICENSE-APACHE "$contents/Resources/"
if command -v python3 >/dev/null; then
    # shellcheck disable=SC2086
    python3 - "$contents/Resources/THIRD-PARTY-NOTICES.txt" "$version" $triples <<'PYTHON'
import pathlib
import sys
sys.path.insert(0, "tools")
import package_release as release
counts = release.write_notices(pathlib.Path(sys.argv[1]), sys.argv[2], tuple(sys.argv[3:]), release.PORTED[:1])
print(f"THIRD-PARTY-NOTICES.txt: {counts[0]} packages, {counts[1]} licence texts, {counts[2]} without a licence file")
PYTHON
else
    echo "warning: python3 was not found, so the app carries no THIRD-PARTY-NOTICES.txt" >&2
fi

find "$app" -type d -exec chmod 755 {} +
find "$app" -type f -exec chmod 644 {} +
chmod 755 "$contents/MacOS/$name"
xattr -cr "$app"

# An ad hoc signature, with no certificate: a Mac with an Apple processor refuses to start a
# program that carries no signature at all.
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"

# The component list says the app may not be moved, so that installing over an older copy
# puts it back in /Applications and not wherever macOS last saw an app with this identifier.
plist="$work/component.plist"
pkgbuild --analyze --root "$work/root" "$plist"
plutil -replace 0.BundleIsRelocatable -bool NO "$plist"
plutil -replace 0.BundleIsVersionChecked -bool NO "$plist"
component="$work/$name-component.pkg"
pkgbuild --root "$work/root" --component-plist "$plist" --identifier "$identifier.pkg" \
    --version "$version" --install-location /Applications "$component"

pkgutil --payload-files "$component" | grep -q "$name\.app/Contents/MacOS/$name\$" \
    || fail "the package does not hold $name.app/Contents/MacOS/$name"

distribution="$work/distribution.xml"
productbuild --synthesize --package "$component" "$distribution"
out="$root/dist/$name-$version.pkg"
rm -f "$out"
productbuild --distribution "$distribution" --package-path "$work" "$out"

# The disk image holds the app beside a shortcut to Applications to drag it onto. ditto keeps
# the signature and the extended attributes that cp would drop.
stage="$work/dmg"
rm -rf "$stage"
mkdir -p "$stage"
ditto "$app" "$stage/$name.app"
ln -s /Applications "$stage/Applications"
image="$root/dist/$name-$version.dmg"
rm -f "$image"
hdiutil create -volname "$name $version" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$image"
hdiutil verify "$image"

(cd "$root/dist" && shasum -a 256 "$name-$version.pkg" "$name-$version.dmg" > SHA256SUMS-macos.txt)
echo "  $(wc -c < "$out" | tr -d ' ')  dist/$name-$version.pkg"
echo "  $(wc -c < "$image" | tr -d ' ')  dist/$name-$version.dmg"
echo "  dist/SHA256SUMS-macos.txt"
echo "Neither is signed with a certificate: macOS asks before it opens them and the app."
