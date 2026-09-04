#!/usr/bin/env bash
set -euo pipefail
REPO="plinthlol/alloy"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
TARGET_VERSION="${1:-latest}"
EXE=""
detect_platform() {
    case "$(uname -s)" in
        Linux)
            case "$(uname -m)" in
                x86_64) echo "linux-x86_64" ;;
                aarch64) echo "error: linux-aarch64 builds are not published yet (only x86_64); install on x86_64 or build from source" >&2; exit 1 ;;
                *) echo "error: unsupported Linux architecture '$(uname -m)'" >&2; exit 1 ;;
            esac ;;
        Darwin) echo "macos-universal" ;;
        MINGW64*|MSYS*|Cygwin*|Windows_NT) EXE=".exe"; echo "windows-x86_64" ;;
        *) echo "error: unsupported platform '$(uname -s)'" >&2; exit 1 ;;
    esac
}
if [ -n "${ALLOY_VARIANT:-}" ]; then
    VARIANT="$ALLOY_VARIANT"
elif [ -r /dev/tty ]; then
    echo "Select Alloy variant:"
    echo "  1) alloysh (TUI, recommended)"
    echo "  2) alloyctl (CLI)"
    printf "Enter choice [1-2]: "
    read -r choice < /dev/tty
    case "$choice" in
        1|"") VARIANT="tui" ;;
        2) VARIANT="cli" ;;
        *) echo "error: invalid choice '$choice'" >&2; exit 1 ;;
    esac
else
    echo "No terminal detected, defaulting to alloysh (TUI). Set ALLOY_VARIANT=tui|cli to choose explicitly."
    VARIANT="tui"
fi
case "$VARIANT" in
    tui) BINARY="alloysh${EXE}" ;;
    cli) BINARY="alloyctl${EXE}" ;;
    *) echo "error: unknown variant '$VARIANT' (use tui or cli)" >&2; exit 1 ;;
esac
PLATFORM=$(detect_platform)
ARTIFACT="${BINARY%"$EXE"}-${PLATFORM}${EXE}"
mkdir -p "$BIN_DIR" || { echo "error: cannot create directory '$BIN_DIR'" >&2; exit 1; }
if ! touch "$BIN_DIR/.alloy-write-test" 2>/dev/null; then
    echo "error: cannot write to '$BIN_DIR' (permission denied)" >&2
    ls -ld "$BIN_DIR" >&2 || true
    exit 1
fi
rm -f "$BIN_DIR/.alloy-write-test"
TARGET="$BIN_DIR/$BINARY"
if [ "$TARGET_VERSION" = "latest" ]; then
    BASE="https://github.com/$REPO/releases/latest/download"
else
    BASE="https://github.com/$REPO/releases/download/$TARGET_VERSION"
fi
echo "Downloading $ARTIFACT..."
TMP="$TARGET.tmp.$$"
rm -f "$TMP"
if ! curl -fSL --show-error --retry 3 -o "$TMP" "$BASE/$ARTIFACT"; then
    rc=$?
    rm -f "$TMP"
    echo "error: download failed (curl exit $rc)" >&2
    if [ "$rc" -eq 23 ]; then
        echo "hint: curl could not write the download to disk." >&2
        echo "hint: check disk space with: df -h \"$BIN_DIR\"" >&2
        echo "hint: check permissions with: ls -ld \"$BIN_DIR\"; ls -l \"$TARGET\"" >&2
        echo "hint: if $BINARY is currently running, quit it and retry." >&2
    fi
    exit 1
fi
mv -f "$TMP" "$TARGET"
chmod +x "$TARGET"
if curl -fsSL -o "$TARGET.sha256" "$BASE/$ARTIFACT.sha256" 2>/dev/null; then
    expected="$(head -c 64 "$TARGET.sha256" | tr -d '[:space:]')"
    if command -v sha256sum >/dev/null; then
        actual="$(sha256sum "$TARGET" | awk '{print $1}')"
    elif command -v shasum >/dev/null; then
        actual="$(shasum -a 256 "$TARGET" | awk '{print $1}')"
    else
        actual=""
    fi
    rm -f "$TARGET.sha256"
    if [ -n "$actual" ] && [ "$actual" != "$expected" ]; then
        rm -f "$TARGET"
        echo "error: checksum mismatch" >&2
        exit 1
    fi
else
    echo "warning: checksum verification skipped"
fi
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        case "${SHELL:-}" in
            *fish) rc="$HOME/.config/fish/config.fish" ;;
            *zsh) rc="$HOME/.zshrc" ;;
            *bash) rc="$([ "$(uname -s)" = "Darwin" ] && echo "$HOME/.bash_profile" || echo "$HOME/.bashrc")" ;;
            *) rc="$HOME/.profile" ;;
        esac
        if ! grep -qsF "$BIN_DIR" "$rc" 2>/dev/null; then
            if [[ "$rc" == *fish* ]]; then
                echo "fish_add_path $BIN_DIR" >> "$rc"
            else
                echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$rc"
            fi
            echo "Updated $rc — restart shell or run: source $rc"
        fi
        ;;
esac
if [ "$(uname -s)" = "Linux" ] && [ "$VARIANT" = "tui" ]; then
    DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
    ICON_BASE="https://raw.githubusercontent.com/$REPO/HEAD/assets"
    icon_ok=true
    for size in 128 256 512; do
        icon_dir="$DATA_DIR/icons/hicolor/${size}x${size}/apps"
        mkdir -p "$icon_dir"
        if curl -fsSL -o "$icon_dir/alloy.png" "$ICON_BASE/icon-$size.png" 2>/dev/null; then
            continue
        fi
        rm -f "$icon_dir/alloy.png"
        icon_ok=false
        break
    done
    if $icon_ok; then
        if command -v gtk-update-icon-cache >/dev/null 2>&1; then
            gtk-update-icon-cache -q -f -t "$DATA_DIR/icons/hicolor" 2>/dev/null || true
        fi
    else
        echo "warning: could not download launcher icon; desktop entry will use a generic icon" >&2
    fi
    TERM_EMU=""
    TERM_EXEC_FLAG="-e"
    if [ -n "${TERMINAL:-}" ] && command -v "${TERMINAL%% *}" >/dev/null 2>&1; then
        TERM_EMU="$TERMINAL"
    else
        for t in foot kitty alacritty wezterm konsole gnome-terminal xfce4-terminal xterm; do
            if command -v "$t" >/dev/null 2>&1; then
                TERM_EMU="$t"
                [ "$t" = "gnome-terminal" ] && TERM_EXEC_FLAG="--"
                break
            fi
        done
    fi
    mkdir -p "$DATA_DIR/applications"
    if [ -n "$TERM_EMU" ]; then
        EXEC_LINE="$TERM_EMU $TERM_EXEC_FLAG $TARGET"
        TERMINAL_KEY="false"
    else
        EXEC_LINE="$TARGET"
        TERMINAL_KEY="true"
        echo "warning: no terminal emulator found; desktop entry uses Terminal=true," >&2
        echo "         which some launchers (fuzzel, rofi, wofi, ...) don't honor." >&2
        echo "         Install a terminal (foot, kitty, alacritty, ...) and re-run" >&2
        echo "         this installer to fix launcher entries automatically." >&2
    fi
    cat > "$DATA_DIR/applications/alloy.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Alloy
GenericName=Minecraft Launcher
Comment=A Minecraft launcher. Minimal but featureful.
Exec=$EXEC_LINE
TryExec=$TARGET
Terminal=$TERMINAL_KEY
Icon=alloy
Categories=Game;Utility;
Keywords=minecraft;mods;modpack;launcher;
StartupNotify=false
EOF
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$DATA_DIR/applications" 2>/dev/null || true
    fi
    if [ -n "$TERM_EMU" ]; then
        echo "✓ Created desktop entry at $DATA_DIR/applications/alloy.desktop (using $TERM_EMU)"
    else
        echo "✓ Created desktop entry at $DATA_DIR/applications/alloy.desktop"
    fi
fi
echo "✓ Installed $BINARY to $TARGET"
