#!/bin/bash
# SPDX-License-Identifier: GPL-2.0
#
# fetch-sources.sh - prepare a patched sound/usb tree for an out-of-tree
# build of snd-usb-audio with the OpenFuse MiniFuse mixer quirk.
#
# Usage: fetch-sources.sh [KVER] [OUTDIR]
#
#   KVER    kernel release to build for (default: uname -r)
#   OUTDIR  build directory to create (default: <this dir>/build/KVER)
#
# The script walks an ordered list of source candidates, takes the first
# one that can be obtained and that the patch applies to, copies it to
# OUTDIR, applies patches/*.patch, adds mixer_minifuse.[ch] and writes a
# Kbuild file.
#
# By default ONLY EXACT matches are used: a module built from sources that
# do not exactly match the running kernel could subtly change behaviour
# for every USB audio device. Exact candidates:
#
#   0. $KSRC, a local kernel tree (only if set; e.g. "apt-get source linux")
#   1. EXACT: the Ubuntu git tag of the exact package version of
#      linux-headers-KVER (e.g. Ubuntu-7.3.0-10.10), looked up in the
#      download cache, then in the sources bundled with the package
#      (sources/), then downloaded from Launchpad (cgit, then git)
#   1u. EXACT-UPSTREAM: for kernels that are not Ubuntu packages only,
#      kernel.org stable at exactly v<VERSION>.<PATCHLEVEL>[.<SUBLEVEL>]
#
# Opt-in, non-exact fallbacks (/etc/openfuse/build.conf, see below):
#
#   2. CACHE: other sources in the download cache for the same
#      VERSION.PATCHLEVEL series, newest first (Ubuntu before upstream)
#   3. BUNDLED: sources bundled in sources/ for the same series, newest first
#      (2 and 3: allow_series_fallback=yes)
#   4. UPSTREAM for an Ubuntu kernel: kernel.org at the base version
#      (allow_upstream_fallback=yes)
#
# A non-exact build prints a warning, which is also recorded in
# OUTDIR/SOURCE_INFO and in the module's "openfuse" modinfo field.
#
# /etc/openfuse/build.conf (KEY=VALUE lines, parsed, never sourced):
#   allow_series_fallback=no|yes     candidates 2 and 3 (default no)
#   allow_upstream_fallback=no|yes   candidate 4 (default no)
#
# OPENFUSE_SOURCE_INDEX=<n> starts the walk at candidate position n (used
# by build-module.sh to try the next candidate when a build fails). The
# position that was used is written to OUTDIR/.source_index.
#
# Exit status: 0 = prepared; 77 = no usable sources (nothing prepared,
# OUTDIR removed); 1 = usage or environment error.
#
# Environment overrides:
#   KSRC=<dir>            local source tree
#   UBUNTU_SERIES=<name>  Launchpad series (default: VERSION_CODENAME)
#   UBUNTU_TAG=<tag>      Ubuntu git tag to use as the exact match
#   UPSTREAM_TAG=<tag>    kernel.org tag to use
#   ALLOW_FALLBACK=0|1    override allow_series_fallback
#   ALLOW_UPSTREAM=0|1    override allow_upstream_fallback
#   OPENFUSE_BUILD_CONF=<file>  configuration file (default /etc/openfuse/build.conf)
#   UBUNTU_TAG=-          treat the kernel as not being an Ubuntu package
#   OPENFUSE_CACHE=<dir>  download cache (default: <this dir>/.cache)
#   OFFLINE=1             no downloads

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KVER="${1:-$(uname -r)}"
OUTDIR="${2:-$HERE/build/$KVER}"
KDIR="${KDIR:-/lib/modules/$KVER/build}"
CACHE="${OPENFUSE_CACHE:-$HERE/.cache}"
BUNDLED="$HERE/sources"
BUILD_CONF="${OPENFUSE_BUILD_CONF:-/etc/openfuse/build.conf}"

# yes/no value of KEY in the build configuration (parsed, never sourced)
conf_bool() {	# key
	local v=""
	[ -r "$BUILD_CONF" ] &&
		v="$(sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\([A-Za-z0-9]*\).*/\1/p" "$BUILD_CONF" | tail -n1)"
	case "$v" in
	[Yy]es|[Tt]rue|1|[Oo]n) echo 1 ;;
	*) echo 0 ;;
	esac
}
ALLOW_FALLBACK="${ALLOW_FALLBACK:-$(conf_bool allow_series_fallback)}"
ALLOW_UPSTREAM="${ALLOW_UPSTREAM:-$(conf_bool allow_upstream_fallback)}"
OFFLINE="${OFFLINE:-0}"
START="${OPENFUSE_SOURCE_INDEX:-0}"

LP_GIT="https://git.launchpad.net"
KORG_GIT="https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git"

# Files without which snd-usb-audio cannot be built
REQUIRED="Makefile card.c mixer.c mixer.h mixer_quirks.c usbaudio.h helper.c helper.h"

EXIT_NOSRC=77

log() { echo "fetch-sources: $*" >&2; }
die() { log "error: $*"; exit 1; }

[ -f "$KDIR/Makefile" ] || die "no kernel headers at $KDIR (install linux-headers-$KVER)"
mkdir -p "$CACHE" || die "cannot create cache directory $CACHE"

# All temporary files live under the cache, never in /tmp.
TMPROOT="$(mktemp -d "$CACHE/.tmp.XXXXXX")"
trap 'rm -rf "$TMPROOT"' EXIT

# ---------------------------------------------------------------------------
# Version detection

mk_var() { sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$KDIR/Makefile" | head -n1; }
SERIES="$(mk_var VERSION).$(mk_var PATCHLEVEL)"
UP_SUBLEVEL="$(mk_var SUBLEVEL)"
if [ -z "$UP_SUBLEVEL" ] || [ "$UP_SUBLEVEL" = 0 ]; then
	UP_TAG="${UPSTREAM_TAG:-v$SERIES}"
else
	UP_TAG="${UPSTREAM_TAG:-v$SERIES.$UP_SUBLEVEL}"
fi

# Ubuntu: the headers package version is the source package version,
# e.g. linux-headers-7.3.0-10-generic 7.3.0-10.10 from source "linux".
UB_VERSION=""
UB_SRCPKG=""
if command -v dpkg-query >/dev/null 2>&1; then
	for pkg in "linux-headers-$KVER" "linux-image-unsigned-$KVER" "linux-image-$KVER"; do
		if v="$(dpkg-query -W -f='${Version} ${source:Package}' "$pkg" 2>/dev/null)" && [ -n "$v" ]; then
			UB_VERSION="${v%% *}"
			UB_SRCPKG="${v##* }"
			# signed image packages come from linux-signed[-flavour]
			UB_SRCPKG="${UB_SRCPKG/#linux-signed/linux}"
			break
		fi
	done
fi
# /proc/version_signature: "Ubuntu 7.3.0-10.10-generic 7.3.0"
if [ -z "$UB_VERSION" ] && [ "$KVER" = "$(uname -r)" ] && [ -r /proc/version_signature ]; then
	sig="$(awk '{print $2}' /proc/version_signature)"
	UB_VERSION="${sig%-*}"
	UB_SRCPKG="linux"
fi

UB_SERIES="${UBUNTU_SERIES:-}"
if [ -z "$UB_SERIES" ] && [ -r /etc/os-release ]; then
	UB_SERIES="$(. /etc/os-release && echo "${VERSION_CODENAME:-}")"
fi

UB_TAG="${UBUNTU_TAG:-}"
if [ "$UB_TAG" = "-" ]; then
	UB_TAG=""
	UB_VERSION=""
	UB_SRCPKG=""
elif [ -z "$UB_TAG" ] && [ -n "$UB_VERSION" ]; then
	# Ubuntu tags replace '~' with '_'; derivative kernels carry their
	# flavour: linux-hwe-7.3 7.3.0-10.10~26.04.1 -> Ubuntu-hwe-7.3-7.3.0-10.10_26.04.1
	if [ "$UB_SRCPKG" = "linux" ] || [ -z "$UB_SRCPKG" ]; then
		UB_TAG="Ubuntu-${UB_VERSION//\~/_}"
	else
		UB_TAG="Ubuntu-${UB_SRCPKG#linux-}-${UB_VERSION//\~/_}"
	fi
fi

log "kernel $KVER, headers $KDIR, series $SERIES, upstream base $UP_TAG"
if [ -n "$UB_TAG" ]; then
	log "Ubuntu package ${UB_SRCPKG:-?} $UB_VERSION, Launchpad series ${UB_SERIES:-?}, tag $UB_TAG"
fi

# ---------------------------------------------------------------------------
# Helpers

http_get() {	# url outfile
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL --retry 2 -m 120 -o "$2" "$1"
	elif command -v wget >/dev/null 2>&1; then
		wget -q -T 120 -O "$2" "$1"
	else
		return 1
	fi
}

check_tree() {	# dir
	local f
	for f in $REQUIRED; do
		[ -s "$1/$f" ] || return 1
	done
}

# a usable source directory: complete and with provenance
valid_src() {	# dir
	[ -f "$1/.origin" ] && check_tree "$1"
}

copy_usb_files() {	# srcdir dest
	mkdir -p "$2"
	find "$1" -maxdepth 1 -type f \
		\( -name '*.c' -o -name '*.h' -o -name Makefile -o -name Kconfig \) \
		-exec cp {} "$2/" \;
}

# series of a source directory: .series file, else derived from its name
# (Ubuntu-7.3.0-10.10 -> 7.3, Ubuntu-hwe-7.3-7.3.0-... -> 7.3, upstream-v7.3.5 -> 7.3)
dir_series() {	# dir
	if [ -s "$1/.series" ]; then
		cat "$1/.series"
	else
		basename "$1" | grep -oE '(^|[^0-9.])v?[0-9]+\.[0-9]+(\.[0-9]+-|\.[0-9]+$|$)' |
			head -n1 | grep -oE '[0-9]+\.[0-9]+' | head -n1
	fi
}

# version key for sorting a source directory name, newest first
dir_key() {	# name
	echo "$1" | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?([-_.][0-9]+)*' | tail -n1
}

# same-series directories under a root, Ubuntu first, newest first
series_dirs() {	# root
	local d n
	[ -d "$1" ] || return 0
	for kind in Ubuntu upstream; do
		for d in "$1"/"$kind"-*; do
			[ -d "$d" ] || continue
			n="$(basename "$d")"
			[ "$n" = "$UB_TAG" ] && continue		# covered by EXACT
			[ "$n" = "upstream-$UP_TAG" ] && continue	# covered by UPSTREAM
			valid_src "$d" || continue
			[ "$(dir_series "$d")" = "$SERIES" ] || continue
			echo "$(dir_key "$n") $d"
		done | sort -rV -k1,1 | cut -d' ' -f2-
	done
}

# cgit: list sound/usb at REF and download the top-level files
fetch_cgit() {	# repo ref dest
	local repo="$1" ref="$2" dest="$3" tmp list f
	tmp="$(mktemp -d "$TMPROOT/cgit.XXXXXX")"
	http_get "$repo/tree/sound/usb?h=$ref" "$tmp/index.html" || return 1
	list="$(grep -o "plain/sound/usb/[^?\"'/]*" "$tmp/index.html" |
		sed 's#^plain/sound/usb/##' | sort -u |
		grep -E '^([A-Za-z0-9_.+-]+\.[ch]|Makefile|Kconfig)$' || true)"
	[ -n "$list" ] || return 1
	mkdir -p "$tmp/usb"
	for f in $list; do
		http_get "$repo/plain/sound/usb/$f?h=$ref" "$tmp/usb/$f" || return 1
	done
	check_tree "$tmp/usb" || return 1
	copy_usb_files "$tmp/usb" "$dest"
}

# sparse, blob-less shallow clone of only sound/usb (slower, ~300 MB)
fetch_git() {	# repo ref dest
	local repo="$1" ref="$2" dest="$3" tmp
	command -v git >/dev/null 2>&1 || return 1
	tmp="$(mktemp -d "$TMPROOT/git.XXXXXX")"
	git clone -q --depth 1 --filter=blob:none --sparse --branch "$ref" \
		"$repo" "$tmp/linux" 2>/dev/null || return 1
	git -C "$tmp/linux" sparse-checkout set sound/usb 2>/dev/null || return 1
	check_tree "$tmp/linux/sound/usb" || return 1
	copy_usb_files "$tmp/linux/sound/usb" "$dest"
}

# download REF from REPO into the cache as NAME; prints the directory
download() {	# repo ref name
	local repo="$1" ref="$2" dest="$CACHE/$3" stage
	[ "$OFFLINE" = 1 ] && return 1
	stage="$(mktemp -d "$TMPROOT/dl.XXXXXX")"
	log "downloading $ref from $repo"
	if fetch_cgit "$repo" "$ref" "$stage/usb" || fetch_git "$repo" "$ref" "$stage/usb"; then
		echo "$4" > "$stage/usb/.origin"
		echo "$SERIES" > "$stage/usb/.series"
		rm -rf "$dest"
		mv "$stage/usb" "$dest"
		echo "$dest"
		return 0
	fi
	return 1
}

# EXACT candidate: cache, bundled, then Launchpad
get_exact() {
	local repos="" series repo
	[ -n "$UB_TAG" ] || return 1
	if valid_src "$CACHE/$UB_TAG"; then echo "$CACHE/$UB_TAG"; return 0; fi
	if valid_src "$BUNDLED/$UB_TAG"; then echo "$BUNDLED/$UB_TAG"; return 0; fi
	for series in $UB_SERIES; do
		repos="$repos $LP_GIT/~ubuntu-kernel/ubuntu/+source/${UB_SRCPKG:-linux}/+git/$series"
		if [ "${UB_SRCPKG:-linux}" != linux ]; then
			repos="$repos $LP_GIT/~ubuntu-kernel/ubuntu/+source/linux/+git/$series"
		fi
		repos="$repos $LP_GIT/~canonical-kernel/ubuntu/+source/${UB_SRCPKG:-linux}/+git/$series"
	done
	for repo in $repos; do
		download "$repo" "$UB_TAG" "$UB_TAG" "Ubuntu $repo tag $UB_TAG" && return 0
	done
	return 1
}

# UPSTREAM candidate: cache, bundled, then kernel.org
get_upstream() {
	local name="upstream-$UP_TAG"
	if valid_src "$CACHE/$name"; then echo "$CACHE/$name"; return 0; fi
	if valid_src "$BUNDLED/$name"; then echo "$BUNDLED/$name"; return 0; fi
	download "$KORG_GIT" "$UP_TAG" "$name" "upstream $KORG_GIT tag $UP_TAG"
}

# ---------------------------------------------------------------------------
# Candidate list: "<kind> <spec>" lines

CANDIDATES="$TMPROOT/candidates"
{
	[ -n "${KSRC:-}" ] && echo "ksrc $KSRC"
	if [ -n "$UB_TAG" ]; then
		echo "exact -"
	else
		# not an Ubuntu package: the upstream release is the exact match
		echo "upstream-exact -"
	fi
	if [ "$ALLOW_FALLBACK" = 1 ]; then
		series_dirs "$CACHE" | sed 's/^/cache /'
		series_dirs "$BUNDLED" | sed 's/^/bundled /'
	fi
	if [ -n "$UB_TAG" ] && [ "$ALLOW_UPSTREAM" = 1 ]; then
		echo "upstream -"
	fi
} > "$CANDIDATES" || true

# Prepare OUTDIR from a pristine source dir; fails if a patch does not apply
prepare() {	# srcdir origin match
	local src="$1" p pkgver
	rm -rf "$OUTDIR"
	mkdir -p "$OUTDIR"
	find "$src" -maxdepth 1 -type f ! -name '.*' -exec cp {} "$OUTDIR/" \;

	for p in "$HERE"/patches/*.patch; do
		[ -e "$p" ] || continue
		# patches are against the kernel tree root: strip a/sound/usb/
		if ! patch -d "$OUTDIR" -p3 --forward --batch --no-backup-if-mismatch -s < "$p" >&2; then
			log "$(basename "$p") does not apply to $src"
			rm -rf "$OUTDIR"
			return 1
		fi
		log "applied $(basename "$p")"
	done

	cp "$HERE/mixer_minifuse.c" "$HERE/mixer_minifuse.h" "$OUTDIR/"

	pkgver="$(sed -n 's/^PACKAGE_VERSION="\(.*\)"/\1/p' "$HERE/dkms.conf" 2>/dev/null || true)"

	# Identifies the module as the OpenFuse build in modinfo; not part
	# of the upstream submission.
	cat > "$OUTDIR/openfuse_info.c" <<-EOF
	// SPDX-License-Identifier: GPL-2.0
	/* Generated by fetch-sources.sh */
	#include <linux/module.h>
	MODULE_INFO(openfuse, "${pkgver:-unknown}; $3; sound/usb from: ${2//\"/}");
	EOF

	# Reuse the (patched) upstream Makefile for the object list, so the
	# set of files and config options always matches the sources, but
	# build only snd-usb-audio; snd-usbmidi-lib and the other drivers
	# stay the distribution's.
	cat > "$OUTDIR/Kbuild" <<-'EOF'
	# SPDX-License-Identifier: GPL-2.0
	# Generated by fetch-sources.sh
	include $(src)/Makefile
	obj-y :=
	obj-m := snd-usb-audio.o
	snd-usb-audio-y += openfuse_info.o
	EOF

	{
		echo "kernel: $KVER"
		echo "headers: $KDIR"
		echo "match: $3"
		echo "sources: $2"
		echo "pristine: $src"
		echo "ubuntu: ${UB_SRCPKG:-} ${UB_VERSION:-} tag ${UB_TAG:-}"
		echo "prepared: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
	} > "$OUTDIR/SOURCE_INFO"
}

# ---------------------------------------------------------------------------
# Walk the candidates

pos=-1
while read -r kind spec; do
	pos=$((pos + 1))
	[ "$pos" -ge "$START" ] || continue

	src=""
	match=""
	case "$kind" in
	ksrc)
		if check_tree "$spec/sound/usb"; then
			src="$TMPROOT/ksrc"
			copy_usb_files "$spec/sound/usb" "$src"
			echo "local tree $spec" > "$src/.origin"
			match="local tree"
		else
			log "KSRC=$spec has no usable sound/usb"
		fi
		;;
	exact)
		if src="$(get_exact)"; then
			match="exact"
		else
			src=""
			[ -n "$UB_TAG" ] && log "exact sources ($UB_TAG) not available"
		fi
		;;
	cache)
		src="$spec"
		match="FALLBACK same-series cached"
		;;
	bundled)
		src="$spec"
		match="FALLBACK same-series bundled"
		;;
	upstream-exact)
		if src="$(get_upstream)"; then
			match="exact upstream $UP_TAG"
		else
			src=""
			log "exact upstream sources ($UP_TAG) not available"
		fi
		;;
	upstream)
		if src="$(get_upstream)"; then
			match="FALLBACK upstream $UP_TAG"
		else
			src=""
			log "upstream sources ($UP_TAG) not available"
		fi
		;;
	esac
	[ -n "$src" ] || continue

	origin="$(cat "$src/.origin")"
	log "candidate $pos ($match): $origin"
	if prepare "$src" "$origin" "$match"; then
		echo "$pos" > "$OUTDIR/.source_index"
		case "$match" in
		exact*|"local tree") ;;
		*)
			log "WARNING: building for $KVER with sources that are NOT an exact match:"
			log "WARNING:   $match: $origin"
			log "WARNING: distribution changes to sound/usb in $KVER may be missing."
			;;
		esac
		log "sources: $origin ($match)"
		log "prepared $OUTDIR"
		exit 0
	fi
done < "$CANDIDATES"

rm -rf "$OUTDIR"
log "no usable sound/usb sources for $KVER (series $SERIES) from candidate $START on"
if [ "$ALLOW_FALLBACK" != 1 ] || [ "$ALLOW_UPSTREAM" != 1 ]; then
	log "(only exact matches are used; non-exact fallbacks are opt-in in $BUILD_CONF)"
fi
exit "$EXIT_NOSRC"
