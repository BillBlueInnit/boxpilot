#!/usr/bin/env bash
# Pushes one of the recipes in packaging/aur to the AUR for a published
# GitHub release, so `yay -S boxpilot-bin` (or `boxpilot`) installs it:
#
#   packaging/aur/publish.sh boxpilot-bin [version]
#   packaging/aur/publish.sh boxpilot [version]
#
# The version defaults to the one in Cargo.toml; its release (or tag, for the
# source package) must already be on GitHub. Needs git, makepkg and an AUR
# account with your SSH public key (https://aur.archlinux.org/account).
# The first push creates the package on the AUR. Set AUR_DRY_RUN=1 to stop
# before the push and look at the result.
set -euo pipefail

pkg=${1:?usage: publish.sh <boxpilot-bin|boxpilot> [version]}
root=$(cd "$(dirname "$0")/../.." && pwd)
version=${2:-$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$root/Cargo.toml")}
version=${version#v}
repo=https://github.com/BillBlueInnit/boxpilot
recipe="$root/packaging/aur/$pkg/PKGBUILD"
[ -f "$recipe" ] || { echo "no recipe at $recipe" >&2; exit 1; }

case "$pkg" in
    boxpilot-bin) array=sha256sums_x86_64 url="$repo/releases/download/v$version/BoxPilot-$version-linux-x64.tar.gz" ;;
    boxpilot) array=sha256sums url="$repo/archive/refs/tags/v$version.tar.gz" ;;
    *) echo "unknown package $pkg" >&2; exit 1 ;;
esac

echo "==> $pkg $version: checksum of $url"
sum=$(curl -fsSL "$url" | sha256sum | cut -d' ' -f1)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git clone -q "ssh://aur@aur.archlinux.org/$pkg.git" "$work/$pkg"
cd "$work/$pkg"
# A new version starts at pkgrel 1; republishing the same version bumps it.
old=$(sed -n 's/^pkgver=//p' PKGBUILD 2>/dev/null || true)
rel=1
if [ "$old" = "$version" ]; then
    rel=$(( $(sed -n 's/^pkgrel=//p' PKGBUILD) + 1 ))
fi
sed -e "s/^pkgver=.*/pkgver=$version/" \
    -e "s/^pkgrel=.*/pkgrel=$rel/" \
    -e "s/^$array=.*/$array=('$sum')/" \
    "$recipe" >PKGBUILD
makepkg --printsrcinfo >.SRCINFO
git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
    echo "==> $pkg is already at $version-$rel"
    exit 0
fi
git -c user.name="${GIT_AUTHOR_NAME:-$(git config user.name || echo BillBlueInnit)}" \
    -c user.email="${GIT_AUTHOR_EMAIL:-$(git config user.email || echo billblueinnit@gmail.com)}" \
    commit -q -m "Update to $version-$rel"
if [ -n "${AUR_DRY_RUN:-}" ]; then
    git --no-pager show --stat HEAD
    cat PKGBUILD
    echo "==> dry run: not pushed"
    exit 0
fi
git push -q origin HEAD:master
echo "==> pushed $pkg $version-$rel: https://aur.archlinux.org/packages/$pkg"
