# FIFO helper lifecycle fix

The packaged `lat_fifo` writer inherits the benchmark controller's SIGTERM
handler before `benchmp_child` installs its own handler. That inherited handler
only sets a flag which the writer loop never reads. Cleanup closes the peer FIFO,
then sends SIGTERM and waits; the writer repeatedly reports EOF without exiting.

The source patch restores the default SIGTERM action in the writer and reaps it
before closing the peer descriptors. The measurement loop and calculation remain
unchanged. DragonOS runs this explicitly patched helper; the existing Linux
reference driver remains separately selected by `LMBENCH_FIFO_CLEANUP=1`.

`make install` requires a C compiler, libc headers, pkg-config, libtirpc headers,
curl, patch, tar and sha256sum. Set `LMBENCH_SOURCE_ARCHIVE` to reuse a downloaded
archive. The build checks the pinned archive SHA-256 before extraction. Nix uses
the same source and patch. Installed helper files retain the upstream COPYING,
patch and source/binary checksums. The source includes additional publication
conditions in individual benchmark headers: identify results as a patched build
and keep this provenance with them.

Upstream source: https://deb.debian.org/debian/pool/non-free/l/lmbench/lmbench_3.0-a9+debian.1.orig.tar.gz
