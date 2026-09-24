#!/bin/sh
# Build the FIFO helper from checksum-pinned sources, preserving its notices.
set -eu
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
out=${1:?usage: build-fifo.sh OUTPUT_DIRECTORY}
mkdir -p "$out"
out=$(CDPATH='' cd -- "$out" && pwd)
archive=${LMBENCH_SOURCE_ARCHIVE:-"$out/lmbench.tar.gz"}
url=https://deb.debian.org/debian/pool/non-free/l/lmbench/lmbench_3.0-a9+debian.1.orig.tar.gz
if [ ! -f "$archive" ]; then
    curl -fL "$url" -o "$archive"
fi
printf '%s  %s\n' cbd5777d15f44eab7666dcac418054c3c09df99826961a397d9acf43d8a2a551 "$archive" | sha256sum -c -
work=$(mktemp -d)
trap 'rm -rf "$work"' 0
trap 'exit 130' INT
trap 'exit 143' TERM
tar -xzf "$archive" -C "$work" --strip-components=1
patch -d "$work" -p1 < "$here/lat_fifo-cleanup.patch"
# Word splitting of compiler flags is intentional, matching make conventions.
# shellcheck disable=SC2086
"${CC:-cc}" ${CPPFLAGS:-} ${CFLAGS:--O2} -DHAVE_socklen_t \
    $(pkg-config --cflags libtirpc) \
    "$work/src/lat_fifo.c" "$work/src/lib_timing.c" \
    "$work/src/lib_mem.c" "$work/src/lib_stats.c" \
    "$work/src/lib_sched.c" "$work/src/getopt.c" \
    ${LDFLAGS:-} -lm -o "$out/lat_fifo"
cp "$work/COPYING" "$out/COPYING.lmbench"
cp "$here/lat_fifo-cleanup.patch" "$out/"
printf 'source=%s\nsource_sha256=%s\n' "$url" \
    cbd5777d15f44eab7666dcac418054c3c09df99826961a397d9acf43d8a2a551 > "$out/build-info.txt"
sha256sum "$out/lat_fifo" "$out/lat_fifo-cleanup.patch" >> "$out/build-info.txt"
