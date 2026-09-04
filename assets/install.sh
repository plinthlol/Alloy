```bash
#!/usr/bin/env bash
set -euo pipefail

REPO="plinthlol/alloy"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
TARGET_VERSION="${1:-latest}"

EXE=""

die() {
    echo "error: $*" >&2
    exit 1
}

detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64)
                    echo "linux-x86_64"
                    ;;
                aarch64|arm64)
                    die "Linux ARM64 builds are not published yet (only x86_64)."
                    ;;
                *)
                    die "unsupported Linux architecture '$arch'"
                    ;;
            esac
            ;;
        Darwin)
            echo "macos-universal"
            ;;
        MINGW64*|MSYS*|Cygwin*|Windows_NT)
            EXE=".exe"
            echo "windows-x86_64"
            ;;
        *)
            die "unsupported platform '$os'"
            ;;
    esac
}

if [[ -n "${ALLOY_VARIANT:-}" ]]; then
    VARIANT="$ALLOY_VARIANT"
elif [[ -r /dev/tty ]]; then
    echo
    echo "Select Alloy variant:"
    echo "  1) alloysh  (TUI, recommended)"
    echo "  2) alloyctl (CLI)"
    printf "Enter choice [1-2]: "

    read -r choice < /dev/tty

    case "$choice" in
        1|"") VARIANT="tui" ;;
        2)    VARIANT="cli" ;;
        *)    die "invalid choice '$choice' (use 1 or 2)" ;;
    esac
else
    echo "No interactive terminal detected; defaulting to alloysh (TUI)." >&2
    echo "Set ALLOY_VARIANT=tui|cli to choose explicitly." >&2
    VARIANT="tui"
fi

case "$VARIANT" in
    tui)
        BINARY="alloysh${EXE}"
        ;;
    cli)
        BINARY="alloyctl${EXE}"
        ;;
    *)
        die "unknown variant '$VARIANT' (use tui or cli)"
        ;;
esac

PLATFORM="$(detect_platform)"
ARTIFACT="${BINARY%"$EXE"}-${PLATFORM}${EXE}"

mkdir -p "$BIN_DIR" || die "cannot create directory '$BIN_DIR'"

if ! touch "$BIN_DIR/.alloy-write-test" 2>/dev/null; then
    echo "error: cannot write to '$BIN_DIR' (permission denied)" >&2
    ls -ld "$BIN_DIR" >&2 || true
    exit 1
fi

rm -f "$BIN_DIR/.alloy-write-test"

TARGET="$BIN_DIR/$BINARY"

if [[ "$TARGET_VERSION" == "latest" ]]; then
    BASE="https://github.com/$REPO/releases/latest/download"
else
    BASE="https://github.com/$REPO/releases/download/$TARGET_VERSION"
fi

echo
echo "Downloading $ARTIFACT"
echo

TMP="$TARGET.tmp.$$"
rm -f "$TMP"

if curl \
    --fail \
    --location \
    --show-error \
    --retry 3 \
    --progress-bar \
    --output "$TMP" \
    "$BASE/$ARTIFACT"
then
    :
else
    rc=$?
    rm -f "$TMP"

    echo
    echo "error: download failed (curl exit $rc)" >&2

    if [[ "$rc" -eq 23 ]]; then
        echo "hint: curl could not write the download to disk." >&2
        echo "hint: check disk space with: df -h \"$BIN_DIR\"" >&2
        echo "hint: check permissions with: ls -ld \"$BIN_DIR\"" >&2
        echo "hint: if $BINARY is currently running, quit it and retry." >&2
    fi

    exit "$rc"
fi

mv -f "$TMP" "$TARGET"
chmod +x "$TARGET"

echo
echo "Download complete."

CHECKSUM_FILE="$TARGET.sha256"

if curl \
    --fail \
    --silent \
    --show-error \
    --location \
    --output "$CHECKSUM_FILE" \
    "$BASE/$ARTIFACT.sha256"
then
    expected="$(awk '{print $1; exit}' "$CHECKSUM_FILE" | tr '[:upper:]' '[:lower:]')"

    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$TARGET" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$TARGET" | awk '{print $1}')"
    else
        actual=""
    fi

    rm -f "$CHECKSUM_FILE"

    if [[ -n "$actual" ]]; then
        if [[ "$actual" != "$expected" ]]; then
            rm -f "$TARGET"
            die "checksum mismatch"
        fi

        echo "✓ Checksum verified"
    else
        echo "warning: no SHA-256 utility found; checksum could not be verified" >&2
    fi
else
    rm -f "$CHECKSUM_FILE"
    echo "warning: checksum verification skipped" >&2
fi

case ":$PATH:" in
    *":$BIN_DIR:"*)
        ;;
    *)
        case "${SHELL:-}" in
            *fish)
                rc="$HOME/.config/fish/config.fish"
                ;;
            *zsh)
                rc="$HOME/.zshrc"
                ;;
            *bash)
                if [[ "$(uname -s)" == "Darwin" ]]; then
                    rc="$HOME/.bash_profile"
                else
                    rc="$HOME/.bashrc"
                fi
                ;;
            *)
                rc="$HOME/.profile"
                ;;
        esac

        mkdir -p "$(dirname "$rc")"

        if ! grep -qsF "$BIN_DIR" "$rc" 2>/dev/null; then
            if [[ "$rc" == *fish* ]]; then
                echo "fish_add_path \"$BIN_DIR\"" >> "$rc"
            else
                echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$rc"
            fi

            echo "Updated $rc"
            echo "Restart your shell or run: source \"$rc\""
        fi
        ;;
esac

if [[ "$(uname -s)" == "Linux" && "$VARIANT" == "tui" ]]; then
    DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
    ICON_BASE="https://raw.githubusercontent.com/$REPO/HEAD/assets"

    icon_ok=true

    for size in 128 256 512; do
        icon_dir="$DATA_DIR/icons/hicolor/${size}x${size}/apps"
        mkdir -p "$icon_dir"

        if curl \
            --fail \
            --silent \
            --show-error \
            --location \
            --output "$icon_dir/alloy.png" \
            "$ICON_BASE/icon-$size.png"
        then
            continue
        fi

        rm -f "$icon_dir/alloy.png"
        icon_ok=false
        break
    done

    if "$icon_ok"; then
        if command -v gtk-update-icon-cache >/dev/null 2>&1; then
            gtk-update-icon-cache \
                -q -f -t \
                "$DATA_DIR/icons/hicolor" \
                2>/dev/null || true
        fi
    else
        echo "warning: could not download launcher icon" >&2
    fi

    mkdir -p "$DATA_DIR/applications"

    cat > "$DATA_DIR/applications/alloy.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Alloy
GenericName=Minecraft Launcher
Comment=A Minecraft launcher. Minimal but featureful.
Exec=$TARGET
TryExec=$TARGET
Terminal=true
Icon=alloy
Categories=Game;Utility;
Keywords=minecraft;mods;modpack;launcher;
StartupNotify=false
EOF

    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database \
            "$DATA_DIR/applications" \
            2>/dev/null || true
    fi

    echo "✓ Created desktop entry at $DATA_DIR/applications/alloy.desktop"
fi

echo
echo "✓ Installed $BINARY to $TARGET"
```
