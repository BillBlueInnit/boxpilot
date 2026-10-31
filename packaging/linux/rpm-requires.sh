#!/bin/sh
# find-requires for cargo-generate-rpm (`--auto-req packaging/linux/rpm-requires.sh`):
# reads file paths on stdin and prints what rpm's own elfdeps would, i.e. the
# libraries the ELF files link directly and the symbol versions they use.
# The builtin mode runs ldd instead, which also lists every library those
# pull in on the build machine, names other distributions don't share.
while read -r file; do
    readelf -h "$file" >/dev/null 2>&1 || continue
    case "$(readelf -h "$file" | sed -n 's/^ *Class: *//p')" in
        ELF64) mark='(64bit)' ;;
        *) mark='' ;;
    esac
    readelf -d "$file" | sed -n "s/.*(NEEDED).*\[\(.*\)\]/\1()$mark/p"
    readelf -V --wide "$file" | awk -v mark="$mark" '
        /File:/ { for (i = 1; i <= NF; i++) if ($i == "File:") lib = $(i + 1) }
        /Name:/ && lib != "" { for (i = 1; i <= NF; i++) if ($i == "Name:") print lib "(" $(i + 1) ")" mark }
        /^$/ { lib = "" }'
done | sort -u
