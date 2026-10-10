# Sourced by the build scripts. Rust puts the build machine's folder paths (and so the builder's user name)
# into the program for its error messages. These flags rewrite them to neutral names, so a build carries
# nothing about the computer it was made on.
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)
cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
# Later flags win when several match, so the most specific folders come last.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=/home --remap-path-prefix=$rustup_home=/rustup --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$repo_root=/lute-and-tavern"
