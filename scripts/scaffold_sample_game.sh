#!/usr/bin/env bash
#
# scaffold_sample_game.sh — Fork nova-sample-game into a new standalone project.
#
# Copies crates/nova-sample-game into a user-supplied target directory and
# performs a best-effort, project-name rename across Cargo.toml and the Rust
# sources. This replaces the manual copy/rename-by-hand flow.
#
# It is NOT a perfect rename-everywhere; it renames the crate name and the
# default lib identifier. Any remaining manual steps are printed at the end.
#
# Usage:
#   ./scripts/scaffold_sample_game.sh <target-path> [source-crate]
#
# Example:
#   ./scripts/scaffold_sample_game.sh ../my-awesome-game

set -euo pipefail

# --- Locate repository root (the dir containing a Cargo.toml with [workspace]).
find_repo_root() {
    local dir
    dir="$(pwd)"
    while true; do
        if [ -f "$dir/Cargo.toml" ] && grep -q '\[workspace\]' "$dir/Cargo.toml"; then
            echo "$dir"
            return 0
        fi
        local parent
        parent="$(dirname "$dir")"
        if [ "$parent" = "$dir" ]; then
            echo "Could not locate workspace root (Cargo.toml with [workspace])." >&2
            exit 1
        fi
        dir="$parent"
    done
}

TARGET="${1:-}"
SOURCE="${2:-crates/nova-sample-game}"

if [ -z "$TARGET" ]; then
    echo "Usage: $0 <target-path> [source-crate]" >&2
    exit 1
fi

REPO_ROOT="$(find_repo_root)"
SOURCE_PATH="$REPO_ROOT/$SOURCE"

if [ ! -d "$SOURCE_PATH" ]; then
    echo "Source crate not found: $SOURCE_PATH" >&2
    exit 1
fi

# Resolve destination (mkdir -p would create parents; we only create the leaf).
DEST_PATH="$(cd "$(dirname "$TARGET")" && pwd)/$(basename "$TARGET")"

# New crate name = last path component, lower-cased.
NEW_NAME="$(basename "$DEST_PATH" | tr '[:upper:]' '[:lower:]')"
NEW_LIB="${NEW_NAME//-/_}"

# Old identifiers.
OLD_NAME="nova-sample-game"
OLD_LIB="nova_sample_game"

echo "Repo root : $REPO_ROOT"
echo "Source    : $SOURCE_PATH"
echo "Target    : $DEST_PATH"
echo "New crate : $NEW_NAME  (lib ident: $NEW_LIB)"
echo ""

# --- Safety: never overwrite an existing directory without confirmation.
if [ -e "$DEST_PATH" ]; then
    read -r -p "Target '$DEST_PATH' already exists. Overwrite its contents? [y/N] " reply
    if [[ ! "$reply" =~ ^[yY]$ ]]; then
        echo "Aborted. Nothing was changed."
        exit 0
    fi
    rm -rf "$DEST_PATH"
fi

# --- Copy the crate.
echo "Copying crate..."
mkdir -p "$DEST_PATH"
cp -R "$SOURCE_PATH/." "$DEST_PATH/"

# --- Best-effort rename of the crate name and lib identifier.
echo "Renaming identifiers ($OLD_NAME -> $NEW_NAME, $OLD_LIB -> $NEW_LIB)..."
# Only touch toml/rs/md files to avoid corrupting binaries.
find "$DEST_PATH" -type f \( -name '*.toml' -o -name '*.rs' -o -name '*.md' \) | while read -r f; do
    if grep -q -e "$OLD_NAME" -e "$OLD_LIB" "$f"; then
        sed -i'' -e "s/$OLD_NAME/$NEW_NAME/g" -e "s/$OLD_LIB/$NEW_LIB/g" "$f"
        rel="${f#"$DEST_PATH"/}"
        echo "    updated $rel"
    fi
done

echo ""
echo "Done. Scaffold created at: $DEST_PATH"
echo ""
echo "Remaining manual steps:"
echo "  1. Default lib/binary name is derived from '$NEW_NAME'. If you want a"
echo "     custom [lib] name or [[bin]] path, edit $DEST_PATH/Cargo.toml."
echo "  2. Re-run 'cargo build -p $NEW_NAME' from the repo root to verify."
echo "  3. If you added this as a workspace member, add the target path to the"
echo "     root Cargo.toml [workspace.members] list."
echo "  4. Strings/paths/identifiers not matching '$OLD_NAME'/'$OLD_LIB'"
echo "     (e.g. doc comments, asset file names) were left untouched."
