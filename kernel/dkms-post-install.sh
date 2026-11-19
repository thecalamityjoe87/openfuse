#!/bin/sh
# SPDX-License-Identifier: GPL-2.0
#
# dkms-post-install.sh - DKMS POST_INSTALL step.
#
# Usage: dkms-post-install.sh KVER
#
# The dkms and module trees are /var/lib/dkms and /lib/modules unless
# dkms_tree / install_tree are set in the DKMS framework configuration
# (OPENFUSE_DKMS_TREE / OPENFUSE_INSTALL_TREE override them for tests).
#
# When dkms installs a module whose name already exists in the kernel's
# tree, it MOVES the distribution's file (here kernel/sound/usb/
# snd-usb-audio.ko.zst) into $DKMS_TREE/openfuse/original_module/ and only
# puts it back on "dkms remove". This puts a COPY back in its original
# place, so that the distribution's module stays available on disk:
# openfuse-modprobe falls back to it if the OpenFuse module cannot be
# loaded, and the system stays usable even if the DKMS state is lost.
# depmod prefers updates/ (search order "updates ubuntu built-in"), so the
# OpenFuse module is still the one modprobe picks. dkms keeps its archived
# copy and restores it as usual on removal.
#
# Runs before dkms' depmod. Never fails the installation (always exit 0).

set -u
LC_ALL=C
export LC_ALL
KVER="${1:-}"
HERE="$(cd "$(dirname "$0")" && pwd)"
DKMS_TREE=""
INSTALL_TREE=""
if [ -r "$HERE/openfuse-mok.sh" ]; then
	. "$HERE/openfuse-mok.sh"
	DKMS_TREE="$(of_framework_var dkms_tree)"
	INSTALL_TREE="$(of_framework_var install_tree)"
fi
DKMS_TREE="${OPENFUSE_DKMS_TREE:-${DKMS_TREE:-/var/lib/dkms}}"
INSTALL_TREE="${OPENFUSE_INSTALL_TREE:-${INSTALL_TREE:-/lib/modules}}"

case "$KVER" in ''|*[!A-Za-z0-9._+-]*) exit 0 ;; esac

for origin in "$DKMS_TREE"/openfuse/original_module/"$KVER"/*/snd-usb-audio.ko*.origin; do
	[ -f "$origin" ] || continue
	mod="${origin%.origin}"
	[ -f "$mod" ] || continue
	dest="$(head -n 1 "$origin")"
	# only in-tree locations of this kernel (never updates/ or extra/,
	# which could hold an older OpenFuse or another third-party build)
	case "$dest" in
	"$INSTALL_TREE/$KVER/kernel/"*) ;;
	*) continue ;;
	esac
	case "$dest" in *..*) continue ;; esac
	[ -e "$dest" ] && continue
	mkdir -p "$(dirname "$dest")" && cp -p "$mod" "$dest" &&
		echo "openfuse: restored a copy of the distribution's $(basename "$dest") for $KVER"
done
exit 0
