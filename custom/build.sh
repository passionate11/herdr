#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
root="${HERDR_CUSTOM_INSTALL_ROOT:-$HOME/.local/share/herdr-custom}"
base_version='0.9.5'
custom_build_id='grouped.1'
while IFS= read -r task_var; do unset "$task_var"; done < <(compgen -v HERDR_)
cd "$repo"
export CARGO_TARGET_DIR="$repo/target"

# Keep tool discovery local to this build, including keg-only Homebrew tools.
PATH="$PATH:/usr/sbin:/sbin"
coreutils_dir=''
for brew_prefix in /usr/local /opt/homebrew; do
    for tool_dir in opt/git/bin opt/python@3.13/libexec/bin opt/rustup/bin; do
        if [ -d "$brew_prefix/$tool_dir" ]; then
            PATH="$brew_prefix/$tool_dir:$PATH"
        fi
    done
    if [ -x "$brew_prefix/opt/coreutils/libexec/gnubin/sha256sum" ]; then
        coreutils_dir="$brew_prefix/opt/coreutils/libexec/gnubin"
    fi
done
export PATH

fail() {
    echo "$*" >&2
    exit 1
}

for tool in python3 rustup just cargo-nextest bun git; do
    command -v "$tool" >/dev/null 2>&1 || fail "Missing build prerequisite: $tool. See custom/README.md."
done
python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' ||
    fail "Build checks require Python 3.11 or later; put a suitable python3 on PATH."
git diff --no-index --default-prefix -- /dev/null /dev/null >/dev/null 2>&1 ||
    fail "Build checks require Git with diff --default-prefix support; install a newer Git."

rust_version=$(python3 -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["toolchain"]["channel"])' rust-toolchain.toml)
rustc_path=$(rustup which --toolchain "$rust_version" rustc) ||
    fail "Rust $rust_version is not installed. Install it with clippy and rustfmt before building."
for rust_tool in cargo rustfmt cargo-clippy; do
    rustup which --toolchain "$rust_version" "$rust_tool" >/dev/null ||
        fail "Rust $rust_version is missing $rust_tool. Install its required component before building."
done
# Use the installed binaries directly so another Cargo installation cannot bypass the pin.
export RUSTUP_TOOLCHAIN="$rust_version"
PATH="$(dirname "$rustc_path"):$PATH"
export PATH
actual_rust=$(rustc --version)
case "$actual_rust" in
    "rustc $rust_version "*) ;;
    *) fail "Expected Rust $rust_version, found $actual_rust." ;;
esac
cargo fmt --version >/dev/null
cargo clippy --version >/dev/null
cargo nextest --version >/dev/null
maintenance_path="$PATH"
if [ -n "$coreutils_dir" ]; then
    maintenance_path="$coreutils_dir:$maintenance_path"
fi
PATH="$maintenance_path" command -v sha256sum >/dev/null 2>&1 ||
    fail "Maintenance checks require sha256sum; install coreutils or put sha256sum on PATH."

if [ -z "${ZIG:-}" ]; then
    bundled="$root/toolchains/zig-aarch64-macos-0.16.0/zig"
    if [ -x "$bundled" ]; then export ZIG="$bundled"; fi
fi
zig_version=$("${ZIG:-zig}" version) || fail "Zig 0.16.0 is required; set ZIG to its executable."
if [ "$zig_version" != 0.16.0 ]; then
    echo "This candidate requires Zig 0.16.0; set ZIG to its executable." >&2
    exit 1
fi

nextest_filter='all()'
if [ "$(uname -s)" = Darwin ]; then
    nextest_filter='not binary(live_handoff)'
fi
just lint
cargo nextest run --locked --test-threads 1 -E "$nextest_filter" --status-level fail --final-status-level fail --failure-output final --success-output never
# GNU chmod lacks macOS ACL flags, so expose coreutils only to installer fixtures.
PATH="$maintenance_path" just maintenance-test
just ui-hot-path-architecture-test
just integration-assets-test
just docs-contract-test
python3 -m unittest discover -s custom -p 'test_*.py'
cargo build --release --locked
binary="$CARGO_TARGET_DIR/release/herdr"
actual_version=$("$binary" --version)
expected_version="herdr $base_version (custom $custom_build_id)"
if [ "$actual_version" != "$expected_version" ]; then
    fail "Candidate identity mismatch: expected '$expected_version', found '$actual_version'."
fi

build_id=$(git rev-parse --short=12 HEAD)
if [ -n "$(git status --porcelain)" ]; then build_id="$build_id-dirty"; fi
candidate="$root/builds/$base_version-$custom_build_id-$build_id"
mkdir -p "$candidate"
install -m 555 "$binary" "$candidate/herdr.next"
mv "$candidate/herdr.next" "$candidate/herdr"
git rev-parse HEAD > "$candidate/source-commit"
python3 - "$candidate/herdr" > "$candidate/sha256" <<'PY'
import hashlib
import sys

with open(sys.argv[1], "rb") as binary:
    print(hashlib.file_digest(binary, "sha256").hexdigest() + "  " + sys.argv[1])
PY
echo "Candidate: $candidate/herdr"
