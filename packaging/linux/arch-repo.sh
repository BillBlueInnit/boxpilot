#!/usr/bin/env bash
# Arch: builds the boxpilot-bin package from a release archive, plus the
# pacman database that lets a GitHub release serve as a package repository
# (no AUR account needed):
#
#   packaging/linux/arch-repo.sh dist/BoxPilot-<version>-linux-x64.tar.gz [out-dir]
#
# writes boxpilot-bin-<version>-1-x86_64.pkg.tar.zst (and a copy without the
# version), boxpilot.db and boxpilot.files to out-dir (default: the archive's
# folder). Users add the repository to /etc/pacman.conf once:
#
#   [boxpilot]
#   SigLevel = Optional TrustAll
#   Server = https://github.com/BillBlueInnit/boxpilot/releases/latest/download
#
# then `sudo pacman -Sy boxpilot-bin` installs it and `pacman -Syu` updates it.
set -euo pipefail

tarball=$(realpath "${1:?usage: arch-repo.sh <BoxPilot-<version>-linux-x64.tar.gz> [out-dir]}")
out=$(realpath "${2:-$(dirname "$tarball")}")
root=$(cd "$(dirname "$0")/../.." && pwd)
name=$(basename "$tarball")
version=${name#BoxPilot-}
version=${version%-linux-*}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sed -e "s/^pkgver=.*/pkgver=$version/" \
    -e "s|^source_x86_64=.*|source_x86_64=(\"$name\")|" \
    "$root/packaging/aur/boxpilot-bin/PKGBUILD" >"$work/PKGBUILD"
cp "$tarball" "$work/"
# --nodeps: the runtime dependencies need not be installed to package.
(cd "$work" && makepkg --force --skipchecksums --nodeps --noconfirm >/dev/null)
pkg=$(basename "$(ls "$work"/boxpilot-bin-*.pkg.tar.zst)")
mkdir -p "$out"
cp "$work/$pkg" "$out/"

# pacman fetches <repo>.db and <repo>.files from the Server URL. repo-add
# makes them symlinks to .tar.gz files; release assets must be the files.
cd "$out"
rm -f boxpilot.db boxpilot.files boxpilot.db.tar.gz* boxpilot.files.tar.gz*
repo-add -q boxpilot.db.tar.gz "$pkg"
rm -f boxpilot.db boxpilot.files
mv boxpilot.db.tar.gz boxpilot.db
mv boxpilot.files.tar.gz boxpilot.files
# The same package under a name without the version, so one URL
# (releases/latest/download/…) always fetches the newest for `pacman -U`.
# pacman wants a signature for packages it downloads itself, so that is
# `curl -LO <url> && sudo pacman -U ./boxpilot-bin-x86_64.pkg.tar.zst`.
cp "$pkg" boxpilot-bin-x86_64.pkg.tar.zst
echo "==> $out/$pkg (and boxpilot-bin-x86_64.pkg.tar.zst), $out/boxpilot.db, $out/boxpilot.files"
