# OpenFuse: build everything a user needs into dist/ (as a normal user).
#
#   make release   -> dist/openfuse-<version>-<arch>.tar.gz, the one file users download
#   make test      every test suite (app, installer, kernel)
#
# The tarball holds the installer and, next to it, the two packages it installs
# (driver and app), in plain sight.

CARGO   ?= $(HOME)/.cargo/bin/cargo
export CARGO
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' app/Cargo.toml | head -1)
# Debian package versions: <upstream>-<DEB_REVISION>, the revisions live in app/Makefile and kernel/Makefile
rev      = $(shell sed -n 's/^DEB_REVISION *:= *//p' $(1)/Makefile)
APP_DEB  := $(VERSION)-$(call rev,app)
DKMS_DEB := $(shell sed -n 's/^PACKAGE_VERSION="\(.*\)"/\1/p' kernel/dkms.conf)-$(call rev,kernel)
ARCH    := $(shell uname -m)
NAME    := openfuse-$(VERSION)-$(ARCH)

.PHONY: release test

release:
	$(MAKE) -C kernel deb
	$(MAKE) -C app deb
	cd installer && $(CARGO) build --release
	# assembled in build/ so a copy unpacked (and running) in dist/ is never touched
	mkdir -p build/$(NAME) dist
	cp installer/target/release/openfuse-installer kernel/dist/openfuse-dkms_$(DKMS_DEB)_all.deb app/dist/openfuse_$(APP_DEB)_*.deb build/$(NAME)/
	printf 'OpenFuse $(VERSION)\n\nTo install, update, repair or uninstall OpenFuse, run:\n\n    ./openfuse-installer\n\nIt installs the two packages in this folder (the driver, openfuse-dkms, and the\napp, openfuse); keep them next to it.\n' > build/$(NAME)/README.txt
	tar -C build -czf dist/$(NAME).tar.gz $(NAME)
	rm -f build/$(NAME)/openfuse-installer build/$(NAME)/README.txt build/$(NAME)/openfuse*.deb
	rmdir build/$(NAME)
	@echo; echo "release: dist/$(NAME).tar.gz"; tar -tvzf dist/$(NAME).tar.gz

# The kernel tests stub dkms and friends; a real dkms in an sbin directory on
# PATH would win over the stubs, so sbin is taken off PATH for them.
test:
	cd app && $(CARGO) test
	cd installer && $(CARGO) test
	PATH="$$(echo "$$PATH" | tr ':' '\n' | grep -v sbin | paste -sd:)" $(MAKE) -C kernel test
