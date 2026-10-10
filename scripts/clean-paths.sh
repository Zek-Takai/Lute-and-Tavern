# Sourced by the build scripts. Rust puts the build machine's folder paths (and so the builder's user name)
# into the program for its error messages. These flags rewrite them to neutral names, so a build carries
# nothing about the computer it was made on.
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)
cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
# Later flags win when several match, so the most specific folders come last.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=/home --remap-path-prefix=$rustup_home=/rustup --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$repo_root=/lute-and-tavern"
# The same for C code some libraries compile (e.g. aws-lc). clang-cl (Windows builds) takes clang's own flags
# behind /clang:.
c_map="-ffile-prefix-map=$cargo_home=/cargo -ffile-prefix-map=$repo_root=/lute-and-tavern -ffile-prefix-map=$HOME=/home"
if [[ "${LT_TARGET:-}" == *windows-msvc ]]; then
  c_map=$(printf '/clang:%s ' $c_map)
fi
export CFLAGS="${CFLAGS:-} $c_map" CXXFLAGS="${CXXFLAGS:-} $c_map"
