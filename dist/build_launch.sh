#!/usr/bin/env bash
# Builds the starter (launch/) for baseline x86-64.
#
# CARGO_ENCODED_RUSTFLAGS, when set, replaces the rustflags in
# .cargo/config.toml. Those flags are x86-64-v3, and a starter built with
# them would crash on the CPU it is supposed to tell about. An empty value
# is that replacement: it is not the same as leaving the variable unset.
# On Windows the static C runtime is kept, without the CPU flag.
#
# Output: target/launch/release/xtrans-launch and xtrans-launch-gui.

set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)

case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    export CARGO_ENCODED_RUSTFLAGS="$(printf -- '-C\037target-feature=+crt-static')"
    ;;
*)
    export CARGO_ENCODED_RUSTFLAGS=
    ;;
esac
export CARGO_TARGET_DIR="$root/target/launch"

echo "==> cargo build --release --manifest-path launch/Cargo.toml (baseline CPU)"
cargo build --release --manifest-path "$root/launch/Cargo.toml"
cargo test --release --manifest-path "$root/launch/Cargo.toml"

if [[ "$(uname -s)" == Linux ]]; then
    # Decoded mnemonics, not the instruction bytes: an immediate can look
    # like a VEX prefix. AVX mnemonics start with v. The programs have
    # them throughout; the starter must have none, or it crashes first.
    starter="$CARGO_TARGET_DIR/release/xtrans-launch"
    vex=$(objdump -d --no-show-raw-insn "$starter" | grep -cE '^[[:space:]]+[0-9a-f]+:[[:space:]]+v[a-z]' || true)
    echo "==> AVX instructions in the starter: $vex"
    if [[ "$vex" != 0 ]]; then
        echo "ERROR: the starter uses AVX and would crash before it can speak"
        exit 1
    fi
fi
