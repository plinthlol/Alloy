#!/usr/bin/env bash
# Alloy installer - downloads latest release and sets up PATH
# Usage: curl -fsSL https://raw.githubusercontent.com/plinthlol/alloy/HEAD/assets/install.sh | bash
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

# Variant selection: ALLOY_VARIANT env var > interactive picker (via /dev/tty, so
# this still works when the script itself is streamed in over stdin, e.g.
# `curl ... | bash`) > default to tui when no terminal is available.
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
mkdir -p "$BIN_DIR"
TARGET="$BIN_DIR/$BINARY"

if [ "$TARGET_VERSION" = "latest" ]; then
    BASE="https://github.com/$REPO/releases/latest/download"
else
    BASE="https://github.com/$REPO/releases/download/$TARGET_VERSION"
fi

echo "Downloading $ARTIFACT..."
curl -fsSL -o "$TARGET" "$BASE/$ARTIFACT" || { rm -f "$TARGET"; echo "error: download failed" >&2; exit 1; }
chmod +x "$TARGET"

# Verify checksum (best-effort; skipped if no .sha256 asset is published)
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

# Add BIN_DIR to PATH in the user's shell rc file, if not already present
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

echo "✓ Installed $BINARY to $TARGET"
