# Build the OpenFuse app and its Debian package (as a normal user).
#   make          release binary
#   make deb      dist/openfuse_<version>-<revision>_<arch>.deb

CARGO   ?= $(HOME)/.cargo/bin/cargo
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# Debian revision: the package is <VERSION>-<DEB_REVISION>. Bump it for every
# rebuild that ships changed contents with the same VERSION (0.1.0-1 ->
# 0.1.0-2); reset it to 1 when VERSION changes.
DEB_REVISION := 1
DEBVERSION := $(VERSION)-$(DEB_REVISION)
ARCH    := $(shell dpkg --print-architecture)
BIN     := target/release/openfuse
PKG     := dist/openfuse_$(DEBVERSION)_$(ARCH).deb
ROOT    := target/deb-root

.PHONY: all deb clean-deb
all: $(BIN)

# always ask cargo: it rebuilds only what changed (a stale binary must never be packaged)
$(BIN): FORCE
	$(CARGO) build --release

FORCE:

deb: $(BIN)
	rm -rf $(ROOT) && mkdir -p $(ROOT)/DEBIAN dist
	install -Dm755 $(BIN) $(ROOT)/usr/bin/openfuse
	strip $(ROOT)/usr/bin/openfuse
	install -Dm644 packaging/openfuse.desktop $(ROOT)/usr/share/applications/openfuse.desktop
	mkdir -p $(ROOT)/usr/share/icons/hicolor/scalable/apps
	$(CARGO) run -q --release --example export_icon > $(ROOT)/usr/share/icons/hicolor/scalable/apps/openfuse.svg
	install -Dm644 assets/fonts/Inter-LICENSE.txt $(ROOT)/usr/share/doc/openfuse/Inter-LICENSE.txt
	printf 'OpenFuse - control panel for Arturia MiniFuse audio interfaces.\nLicense: GPL-3.0-or-later (see /usr/share/common-licenses/GPL-3).\nArturia and MiniFuse are trademarks of Arturia; OpenFuse is not affiliated with Arturia.\nInter font: SIL Open Font License 1.1 (see Inter-LICENSE.txt).\n' > $(ROOT)/usr/share/doc/openfuse/copyright
	glibc=$$(objdump -T $(BIN) | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -V | tail -1); \
	size=$$(du -sk --exclude=DEBIAN $(ROOT) | cut -f1); \
	sed -e 's/@VERSION@/$(DEBVERSION)/g' -e 's/@ARCH@/$(ARCH)/' -e "s/@SIZE@/$$size/" -e "s/@GLIBC@/$$glibc/" \
	    packaging/control.in > $(ROOT)/DEBIAN/control
	dpkg-deb --root-owner-group -Zxz --build $(ROOT) $(PKG)
	@echo built $(PKG)

clean-deb:
	rm -rf $(ROOT) dist
