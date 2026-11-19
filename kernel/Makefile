# SPDX-License-Identifier: GPL-2.0
#
# Out-of-tree build of snd-usb-audio with the OpenFuse MiniFuse mixer quirk.
#
#   make                  build for the running kernel
#   make KVER=<release>   build for another installed kernel
#   make deb              build dist/openfuse-dkms_<version>_all.deb
#   make test             failure-mode tests (normal user, stubs, no root)
#   make checkpatch       run checkpatch.pl on the quirk sources
#   make clean            remove build output (keeps the download cache)
#   make distclean        also remove the download cache and dist/
#
# The patched sound/usb tree is prepared in build/$(KVER) by
# fetch-sources.sh (called from build-module.sh); the module ends up in
# build/$(KVER)/snd-usb-audio.ko.

KVER	?= $(shell uname -r)
KDIR	?= /lib/modules/$(KVER)/build
HERE	:= $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
VERSION	:= $(shell sed -n 's/^PACKAGE_VERSION="\(.*\)"/\1/p' $(HERE)/dkms.conf)
# Debian revision of openfuse-dkms: the package is <VERSION>-<DEB_REVISION>.
# Bump it for every rebuild that ships changed contents with the same VERSION
# (0.1.0-1 -> 0.1.0-2); reset it to 1 when VERSION changes.
DEB_REVISION := 1

CHECKPATCH ?= $(firstword $(wildcard $(KDIR)/scripts/checkpatch.pl) checkpatch.pl)

.PHONY: all modules clean distclean checkpatch deb test

all: modules

modules:
	$(HERE)/build-module.sh $(KVER) $(KDIR)

test:
	$(HERE)/tests/failure-modes.sh

deb:
	$(HERE)/packaging/mkdeb.sh $(VERSION) $(DEB_REVISION)

checkpatch:
	$(CHECKPATCH) --no-tree --strict -f $(HERE)/mixer_minifuse.c $(HERE)/mixer_minifuse.h
	$(CHECKPATCH) --no-tree --strict $(HERE)/patches/*.patch

clean:
	rm -rf $(HERE)/build

distclean: clean
	rm -rf $(HERE)/.cache $(HERE)/dist
