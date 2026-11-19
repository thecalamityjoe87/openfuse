#!/bin/bash
# SPDX-License-Identifier: GPL-2.0
#
# mkdeb.sh - build dist/openfuse-dkms_<version>_all.deb as a normal user.
#
# Usage: packaging/mkdeb.sh VERSION REVISION
#   VERSION  upstream / DKMS version (0.1.0): source paths, dkms add/remove
#   REVISION Debian revision: the package itself is VERSION-REVISION (0.1.0-1)
#
# Bundles the pristine sound/usb sources of every kernel built so far
# (build/*/SOURCE_INFO, "pristine:" line) under /usr/src/openfuse-VERSION/
# sources/, where fetch-sources.sh uses them as an offline fallback.

set -euo pipefail

VERSION="${1:?usage: mkdeb.sh VERSION REVISION}"
REVISION="${2:?usage: mkdeb.sh VERSION REVISION}"
DEBVERSION="$VERSION-$REVISION"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEB="$HERE/packaging/debian"
PKG=openfuse-dkms
STAGE="$HERE/build/deb/${PKG}_${DEBVERSION}_all"
DIST="$HERE/dist"
OUT="$DIST/${PKG}_${DEBVERSION}_all.deb"
SRC="$STAGE/usr/src/openfuse-$VERSION"
DOC="$STAGE/usr/share/doc/$PKG"

log() { echo "mkdeb: $*" >&2; }

rm -rf "$STAGE"
LIBEXEC="$STAGE/usr/libexec/openfuse"
POLKIT="$STAGE/usr/share/polkit-1/actions"
MODPROBED="$STAGE/usr/lib/modprobe.d"
mkdir -p "$SRC/patches" "$SRC/sources" "$DOC" "$STAGE/DEBIAN" "$DIST" "$LIBEXEC" "$POLKIT" "$MODPROBED"

# DKMS source tree
cp "$HERE/dkms.conf" "$HERE/Makefile" "$HERE/build-module.sh" "$HERE/openfuse-mok.sh" \
   "$HERE/dkms-post-install.sh" \
   "$HERE/fetch-sources.sh" "$HERE/mixer_minifuse.c" "$HERE/mixer_minifuse.h" "$SRC/"

# Installer helper, its library and the polkit action
sed "s/@VERSION@/$VERSION/" "$HERE/packaging/openfuse-setup.in" > "$LIBEXEC/openfuse-setup"
cp "$HERE/openfuse-mok.sh" "$LIBEXEC/openfuse-mok.sh"
cp "$HERE/packaging/org.openfuse.setup.policy" "$POLKIT/org.openfuse.setup.policy"

# Load-time fallback to the distribution's module
cp "$HERE/packaging/openfuse-modprobe" "$LIBEXEC/openfuse-modprobe"
cp "$HERE/packaging/openfuse-fallback.conf" "$MODPROBED/openfuse-fallback.conf"
cp "$HERE"/patches/*.patch "$SRC/patches/"

# Bundled sources: the pristine trees the existing builds were made from
for info in "$HERE"/build/*/SOURCE_INFO; do
	[ -f "$info" ] || continue
	pristine="$(sed -n 's/^pristine: //p' "$info")"
	headers="$(sed -n 's/^headers: //p' "$info")"
	[ -n "$pristine" ] && [ -f "$pristine/.origin" ] || continue
	name="$(basename "$pristine")"
	case "$name" in
	Ubuntu-*|upstream-*) ;;
	*) log "not bundling $pristine (not a tagged tree)"; continue ;;
	esac
	[ -d "$SRC/sources/$name" ] && continue
	mkdir -p "$SRC/sources/$name"
	find "$pristine" -maxdepth 1 -type f \
		\( -name '*.c' -o -name '*.h' -o -name Makefile -o -name Kconfig -o -name .origin \) \
		-exec cp {} "$SRC/sources/$name/" \;
	series="$(sed -n 's/^VERSION = //p' "$headers/Makefile").$(sed -n 's/^PATCHLEVEL = //p' "$headers/Makefile")"
	echo "$series" > "$SRC/sources/$name/.series"
	log "bundling $name (series $series)"
done
[ -n "$(ls -A "$SRC/sources")" ] || log "warning: no sources bundled (run make for some kernels first)"

# Documentation
cp "$DEB/copyright" "$DOC/copyright"
cp "$HERE/../docs/kernel.md" "$DOC/README.md"
sed -e "s/@VERSION@/$DEBVERSION/" -e "s/@DATE@/$(date -R)/" "$DEB/changelog" |
	gzip -9n > "$DOC/changelog.gz"

# Permissions
find "$STAGE" -type d -exec chmod 755 {} +
find "$STAGE" -type f -exec chmod 644 {} +
chmod 755 "$SRC/build-module.sh" "$SRC/fetch-sources.sh" "$SRC/dkms-post-install.sh" \
	"$LIBEXEC/openfuse-setup" "$LIBEXEC/openfuse-modprobe"

# Control files
for s in postinst prerm postrm; do
	sed -e "s/@VERSION@/$VERSION/" -e "/^@CLEANUP@\$/{r $DEB/cleanup.sh.in" -e "d}" \
		"$DEB/$s" > "$STAGE/DEBIAN/$s"
	chmod 755 "$STAGE/DEBIAN/$s"
done
size="$(du -sk --exclude=DEBIAN "$STAGE" | cut -f1)"
sed -e "s/@VERSION@/$DEBVERSION/" -e "s/@SIZE@/$size/" "$DEB/control.in" > "$STAGE/DEBIAN/control"
(cd "$STAGE" && find usr -type f -print0 | sort -z | xargs -0 md5sum) > "$STAGE/DEBIAN/md5sums"
chmod 644 "$STAGE/DEBIAN/control" "$STAGE/DEBIAN/md5sums"

rm -f "$OUT"
if command -v fakeroot >/dev/null 2>&1; then
	fakeroot dpkg-deb --build "$STAGE" "$OUT"
else
	dpkg-deb --root-owner-group --build "$STAGE" "$OUT"
fi
log "built $OUT"
