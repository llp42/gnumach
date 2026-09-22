#!/bin/sh
# GNU Mach ABI test suite -- run the i386 and x86_64 packs in one go.
#
# Copyright (C) 2024 Free Software Foundation
#
# This program is free software ; you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation ; either version 2 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY ; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
# GNU General Public License for more details.

set -u

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd) || exit 2

I386_DIR=
X86_64_DIR=
KEEP=no
KERNEL_I386=
KERNEL_X86_64=

die() {
    printf 'error: %s\n' "$*" >&2
    exit 2
}

usage() {
    cat <<EOF
Usage: $(basename -- "$0") <i386-kernel> <x86_64-kernel> [runner options...]

Extract the sibling i386.tar.gz and x86_64.tar.gz packs, run both ABI test
suites and aggregate the results.  Extra options are passed verbatim to
both run-<arch>.sh runners, e.g. --test hello or --timeout 60.

Options:
  --i386-dir DIR      use an already extracted abi-test-i386 directory
  --x86_64-dir DIR    use an already extracted abi-test-x86_64 directory
  --keep              keep the temporary extraction directory
  -h, --help          show this help

Exit status: 0 if both architectures pass all their tests, 1 if any test
failed, 2 on usage or setup errors.
EOF
}

while [ $# -gt 0 ]; do
    case $1 in
    --i386-dir)
        [ $# -ge 2 ] || die "option --i386-dir requires an argument"
        I386_DIR=$2
        shift 2
        ;;
    --i386-dir=*)
        I386_DIR=${1#--i386-dir=}
        shift
        ;;
    --x86_64-dir)
        [ $# -ge 2 ] || die "option --x86_64-dir requires an argument"
        X86_64_DIR=$2
        shift 2
        ;;
    --x86_64-dir=*)
        X86_64_DIR=${1#--x86_64-dir=}
        shift
        ;;
    --keep)
        KEEP=yes
        shift
        ;;
    -h|--help)
        usage
        exit 0
        ;;
    --)
        shift
        break
        ;;
    -*)
        die "unknown option: $1 (extra runner options go after the two kernel paths)"
        ;;
    *)
        break
        ;;
    esac
done

[ $# -ge 2 ] || {
    usage >&2
    die "missing kernel arguments"
}
KERNEL_I386=$1
KERNEL_X86_64=$2
shift 2
# Extra runner options are word-split on purpose: they are forwarded as-is.
EXTRA=$*

[ -f "$KERNEL_I386" ] || die "i386 kernel image not found: $KERNEL_I386"
[ -f "$KERNEL_X86_64" ] || die "x86_64 kernel image not found: $KERNEL_X86_64"

WORKDIR=$(mktemp -d "${TMPDIR:-/tmp}/abi-test-pack.XXXXXX") ||
    die "cannot create temporary directory"

cleanup() {
    if [ "$KEEP" = yes ]; then
        printf 'temporary directory kept: %s\n' "$WORKDIR"
    else
        rm -rf "$WORKDIR"
    fi
}
trap cleanup EXIT

if [ -n "$I386_DIR" ]; then
    [ -x "$I386_DIR/run-i386.sh" ] ||
        die "no executable run-i386.sh in --i386-dir $I386_DIR"
else
    [ -f "$SCRIPT_DIR/i386.tar.gz" ] || die "missing $SCRIPT_DIR/i386.tar.gz"
    tar xzf "$SCRIPT_DIR/i386.tar.gz" -C "$WORKDIR" ||
        die "cannot extract $SCRIPT_DIR/i386.tar.gz"
    I386_DIR=$WORKDIR/abi-test-i386
    [ -x "$I386_DIR/run-i386.sh" ] ||
        die "extracted i386 pack has no executable run-i386.sh"
fi

if [ -n "$X86_64_DIR" ]; then
    [ -x "$X86_64_DIR/run-x86_64.sh" ] ||
        die "no executable run-x86_64.sh in --x86_64-dir $X86_64_DIR"
else
    [ -f "$SCRIPT_DIR/x86_64.tar.gz" ] || die "missing $SCRIPT_DIR/x86_64.tar.gz"
    tar xzf "$SCRIPT_DIR/x86_64.tar.gz" -C "$WORKDIR" ||
        die "cannot extract $SCRIPT_DIR/x86_64.tar.gz"
    X86_64_DIR=$WORKDIR/abi-test-x86_64
    [ -x "$X86_64_DIR/run-x86_64.sh" ] ||
        die "extracted x86_64 pack has no executable run-x86_64.sh"
fi

out_i386=$WORKDIR/i386.out
out_x86_64=$WORKDIR/x86_64.out

# shellcheck disable=SC2086
"$I386_DIR/run-i386.sh" "$KERNEL_I386" $EXTRA >"$out_i386" 2>&1
rc_i386=$?
# shellcheck disable=SC2086
"$X86_64_DIR/run-x86_64.sh" "$KERNEL_X86_64" $EXTRA >"$out_x86_64" 2>&1
rc_x86_64=$?

cat "$out_i386"
cat "$out_x86_64"

parse_field() {
    sed -n "s/.*$2=\([0-9][0-9]*\).*/\1/p" "$1" | tail -n 1
}

i386_passed=$(parse_field "$out_i386" passed)
i386_total=$(parse_field "$out_i386" total)
x86_64_passed=$(parse_field "$out_x86_64" passed)
x86_64_total=$(parse_field "$out_x86_64" total)
: "${i386_passed:=0}" "${i386_total:=0}"
: "${x86_64_passed:=0}" "${x86_64_total:=0}"

printf '\n================== abi-test summary ==================\n'
printf 'i386:   %s/%s passed (runner exit %s)\n' \
    "$i386_passed" "$i386_total" "$rc_i386"
printf 'x86_64: %s/%s passed (runner exit %s)\n' \
    "$x86_64_passed" "$x86_64_total" "$rc_x86_64"
printf 'TOTAL:  %s/%s passed\n' \
    "$((i386_passed + x86_64_passed))" "$((i386_total + x86_64_total))"
printf '======================================================\n'

if [ "$rc_i386" -eq 2 ] || [ "$rc_x86_64" -eq 2 ]; then
    exit 2
fi
if [ "$rc_i386" -ne 0 ] || [ "$rc_x86_64" -ne 0 ]; then
    exit 1
fi
exit 0
