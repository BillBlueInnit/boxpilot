#!/usr/bin/env bash
# Builds the Linux release into dist/:
#   BoxPilot-<version>-linux-<arch>.tar.gz   portable archive (what the in-app updater reads)
#   boxpilot_<version>-1_<arch>.deb          Debian, Ubuntu, Mint…
#   boxpilot-<version>-1.<arch>.rpm          Fedora, openSUSE, RHEL…
#   boxpilot-bin-<version>-1-<arch>.pkg.tar.zst   Arch, with boxpilot.db / boxpilot.files
#                                            (only where makepkg exists)
#
#   packaging/linux/build.sh [tar] [deb] [rpm] [arch]    (default: all that can be built here)
#
# Needs cargo-deb and cargo-generate-rpm (`cargo install cargo-deb cargo-generate-rpm`),
# neither of which needs dpkg or rpmbuild. Packages built on a rolling distribution
# require its glibc; the release workflow builds the distributed ones on Ubuntu 24.04.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH"

version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)
case "$(uname -m)" in
    x86_64) arch=x64 ;;
    aarch64) arch=arm64 ;;
    *) arch=$(uname -m) ;;
esac

want=("$@")
if [ ${#want[@]} -eq 0 ]; then
    want=(tar deb rpm)
    command -v makepkg >/dev/null && want+=(arch)
fi
wants() { [[ " ${want[*]} " == *" $1 "* ]]; }

echo "==> BoxPilot $version (${want[*]})"
cargo build --release --locked -p boxpilot
cargo run --release --locked -q -p boxpilot --example linux_icons -- packaging/linux/icons >/dev/null

mkdir -p dist

# The archive carries the desktop entry and icons in a share/ tree, so it can
# be unpacked into ~/.local or /usr/local as well as run in place.
tarball="dist/BoxPilot-$version-linux-$arch.tar.gz"
if wants tar || wants arch; then
    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT
    install -Dm755 target/release/boxpilot "$stage/boxpilot"
    install -Dm644 LICENSE "$stage/LICENSE"
    install -Dm644 README.md "$stage/README.md"
    install -Dm644 packaging/linux/boxpilot.desktop "$stage/share/applications/boxpilot.desktop"
    install -Dm644 packaging/linux/io.github.billblueinnit.boxpilot.metainfo.xml \
        "$stage/share/metainfo/io.github.billblueinnit.boxpilot.metainfo.xml"
    cp -r packaging/linux/icons "$stage/share/icons"
    tar -czf "$tarball" -C "$stage" .
    echo "==> $tarball"
fi

if wants deb; then
    cargo deb -p boxpilot --no-build --output dist/
fi

if wants rpm; then
    cargo generate-rpm -p crates/app --auto-req "$root/packaging/linux/rpm-requires.sh" --output dist/
fi

# Arch: the boxpilot-bin recipe fed the archive built above, and the pacman
# database that lets a release serve as a repository (see arch-repo.sh).
if wants arch; then
    packaging/linux/arch-repo.sh "$tarball" dist
fi

echo "==> dist/"
ls -la dist
