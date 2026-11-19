#!/bin/bash
# SPDX-License-Identifier: GPL-2.0
#
# build-module.sh - build the patched snd-usb-audio for one kernel.
#
# Usage: build-module.sh [--dkms] KVER [KDIR]
#
# Used by "make" and as the DKMS PRE_BUILD step. Prepares the sources with
# fetch-sources.sh and builds them; if a fallback source tree does not
# compile, the next candidate is tried. On success build/KVER/.built is
# written and the module is build/KVER/snd-usb-audio.ko.
#
# Exit status: 0 = built (or already up to date); 77 = no usable sources
# or nothing compiled. DKMS treats 77 as "skipped": nothing is installed
# for that kernel, the distribution's snd-usb-audio stays in use and
# "dkms autoinstall" (run when a kernel is installed) does not fail.
#
# --dkms (the DKMS PRE_BUILD step) adds a Secure Boot guard: when Secure
# Boot/lockdown is active, the key DKMS signs with must be enrolled or
# queued for enrollment (mokutil --import, confirmed in MokManager at the
# next boot). Otherwise the module would be refused at load time and USB
# audio would disappear, so the build is skipped (77) instead.
# Override: OPENFUSE_SKIP_MOK_CHECK=1.
#
# Download cache: $OPENFUSE_CACHE, else /var/cache/openfuse when run as
# root (DKMS; its build directory is recreated for every build), else
# <this dir>/.cache.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DKMS=0
if [ "${1:-}" = --dkms ]; then
	DKMS=1
	shift
fi
KVER="${1:?usage: build-module.sh [--dkms] KVER [KDIR]}"
KDIR="${2:-/lib/modules/$KVER/build}"
OUT="$HERE/build/$KVER"
SKIP=77

log() { echo "openfuse: $*" >&2; }

if [ -z "${OPENFUSE_CACHE:-}" ]; then
	if [ "$(id -u)" = 0 ]; then
		OPENFUSE_CACHE=/var/cache/openfuse
	else
		OPENFUSE_CACHE="$HERE/.cache"
	fi
fi
export OPENFUSE_CACHE KDIR

if [ ! -f "$KDIR/Makefile" ]; then
	log "no kernel headers for $KVER at $KDIR; skipping (stock snd-usb-audio stays in use)"
	exit $SKIP
fi

# Secure Boot guard (DKMS builds only), see openfuse-mok.sh
. "$HERE/openfuse-mok.sh"
if [ $DKMS = 1 ] && [ "${OPENFUSE_SKIP_MOK_CHECK:-0}" != 1 ] && of_sig_enforced; then
	of_mok_paths
	state="$(of_key_state "$OF_MOK_CERT")"
	case "$state" in
	enrolled)
		log "Secure Boot: signing key $OF_MOK_CERT is enrolled"
		;;
	pending)
		log "Secure Boot: signing key $OF_MOK_CERT is queued for enrollment;"
		log "the module loads after the next boot once the key is confirmed in MokManager."
		;;
	*)
		log "Secure Boot is enabled and the signing key $OF_MOK_CERT is not usable (state: $state)."
		log "Skipping: a module signed with a key that is neither enrolled nor queued"
		log "for enrollment would be refused, and USB audio would disappear."
		log "Enroll it with the OpenFuse installer, or: sudo mokutil --import $OF_MOK_CERT"
		log "then build: sudo /usr/libexec/openfuse/openfuse-setup build"
		exit $SKIP
		;;
	esac
fi

# Use the compiler the kernel was built with if it is installed (e.g.
# x86_64-linux-gnu-gcc on Ubuntu); avoids kbuild's "compiler differs" warning.
CC_ARG=()
kcc="$(sed -n 's/^CONFIG_CC_VERSION_TEXT="\([^ ]*\) .*/\1/p' "$KDIR/.config" 2>/dev/null)"
if [ -n "$kcc" ] && command -v "$kcc" >/dev/null 2>&1; then
	CC_ARG=("CC=$kcc")
fi

# Up to date? (sources unchanged since the last successful build)
INPUTS=("$HERE/fetch-sources.sh" "$HERE/build-module.sh" "$HERE/openfuse-mok.sh" "$HERE/mixer_minifuse.c"
	"$HERE/mixer_minifuse.h" "$HERE"/patches/*.patch)
if [ -f "$OUT/.built" ] && [ -f "$OUT/snd-usb-audio.ko" ]; then
	fresh=1
	for f in "${INPUTS[@]}"; do
		[ "$f" -nt "$OUT/.built" ] && fresh=0
	done
	if [ $fresh = 1 ]; then
		log "build/$KVER is up to date"
		exit 0
	fi
fi

idx=0
while :; do
	OPENFUSE_SOURCE_INDEX=$idx "$HERE/fetch-sources.sh" "$KVER" "$OUT"
	rc=$?
	if [ $rc -ne 0 ]; then
		log "cannot build for $KVER: no usable sound/usb sources (fetch-sources exit $rc)."
		log "Skipping; the distribution's snd-usb-audio stays in use for $KVER."
		rm -rf "$OUT"
		exit $SKIP
	fi
	used="$(cat "$OUT/.source_index")"

	if make -C "$KDIR" M="$OUT" "${CC_ARG[@]}" modules; then
		touch "$OUT/.built"
		log "built $OUT/snd-usb-audio.ko ($(sed -n 's/^match: //p' "$OUT/SOURCE_INFO"))"
		exit 0
	fi

	log "build failed with source candidate $used; trying the next candidate"
	idx=$((used + 1))
done
