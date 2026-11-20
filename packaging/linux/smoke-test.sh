#!/bin/sh
# Smoke-test a staged Linux package tree without nfpm, appimagetool or any other
# packaging tool: the three binaries, co-located and executable, the desktop
# entry and AppStream metadata with agreeing ids, and the hicolor icons.
#
# Usage: packaging/linux/smoke-test.sh [STAGE_DIR]
#   STAGE_DIR defaults to $CARGO_TARGET_DIR/linux-package/root, the tree that
#   packaging/linux/package.sh stages for the deb, rpm and AppImage. An extracted
#   -fhs.tar.gz directory works too.
set -eu

fail() {
    printf 'smoke-test: %s\n' "$*" >&2
    exit 1
}

here=$(CDPATH= cd -P -- "$(dirname "$0")" && pwd)
root=$(CDPATH= cd -P -- "$here/../.." && pwd)
stage=${1:-${CARGO_TARGET_DIR:-$root/target}/linux-package/root}
[ -d "$stage" ] || fail "staging directory not found: $stage"

# A literal substring search over lines, so no grep is needed.
contains() {
    file=$1
    needle=$2
    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
            *"$needle"*) return 0 ;;
        esac
    done < "$file"
    return 1
}

# The window, the daemon and the CLI must be one directory: the front ends look
# for the daemon beside themselves (and the daemon starts the window likewise).
for binary in oxidezap oxidezapd oxidezap-cli; do
    path="$stage/usr/bin/$binary"
    [ -f "$path" ] || fail "missing binary: usr/bin/$binary"
    [ -s "$path" ] || fail "empty binary: usr/bin/$binary"
    [ -x "$path" ] || fail "binary is not executable: usr/bin/$binary"
done

desktop="$stage/usr/share/applications/org.oxidezap.client.local.desktop"
[ -s "$desktop" ] || fail "missing or empty desktop entry"
contains "$desktop" 'Exec=oxidezap' || fail "desktop entry has no Exec=oxidezap"
contains "$desktop" 'Icon=org.oxidezap.client.local' || fail "desktop entry has the wrong Icon"
contains "$desktop" 'StartupWMClass=org.oxidezap.client.local' || fail "desktop entry has the wrong StartupWMClass"

metainfo="$stage/usr/share/metainfo/io.github.oxidezap.client.metainfo.xml"
[ -s "$metainfo" ] || fail "missing or empty AppStream metadata"
contains "$metainfo" '<id>io.github.oxidezap.client</id>' || fail "AppStream component id is wrong"
contains "$metainfo" '<launchable type="desktop-id">org.oxidezap.client.local.desktop</launchable>' || fail "AppStream launchable does not name the desktop entry"
contains "$metainfo" '<icon type="stock">org.oxidezap.client.local</icon>' || fail "AppStream icon does not name the desktop icon"
if contains "$metainfo" '@VERSION@' || contains "$metainfo" '@DATE@'; then
    fail "AppStream metadata still has unsubstituted placeholders"
fi

for size in 16 24 32 48 64 128 256; do
    icon="$stage/usr/share/icons/hicolor/${size}x${size}/apps/org.oxidezap.client.local.png"
    [ -s "$icon" ] || fail "missing or empty icon: usr/share/icons/hicolor/${size}x${size}/apps/org.oxidezap.client.local.png"
done

printf 'smoke-test: %s is OK\n' "$stage"
