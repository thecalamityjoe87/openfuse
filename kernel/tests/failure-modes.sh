#!/bin/bash
# SPDX-License-Identifier: GPL-2.0
#
# failure-modes.sh - test the "never break the user's system" paths as a
# normal user: stub modprobe/insmod/modinfo/depmod/dkms/mokutil on PATH or
# substituted into test copies of the scripts, and a fake /lib/modules,
# /sys/module, /run and dkms tree under build/tests/. Touches nothing
# outside build/tests/ (plus read-only use of the running kernel's
# headers for the source-selection tests). No root, no network, no USB.
#
# Usage: tests/failure-modes.sh      (or: make test)

set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
W="$HERE/build/tests"
KVER="$(uname -r)"
PASS=0
FAIL=0
RESULTS=()

rm -rf "$W"
mkdir -p "$W"

result() {	# id description status
	RESULTS+=("$(printf '%-4s %-4s %s' "$1" "$3" "$2")")
	if [ "$3" = PASS ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); fi
}
check() {	# id description command...
	local id="$1" desc="$2"
	shift 2
	if "$@" >/dev/null 2>&1; then result "$id" "$desc" PASS; else result "$id" "$desc" FAIL; fi
}

# ---------------------------------------------------------------------------
# Fake system tree and stubs for the modprobe wrapper

R="$W/root"
S="$W/stub"
C="$W/ctl"		# stub behaviour switches
LOG="$W/calls.log"

new_root() {
	rm -rf "$R" "$C" "$LOG"
	mkdir -p "$R/lib/modules/$KVER/kernel/sound/usb" "$R/lib/modules/$KVER/updates/dkms" \
		"$R/sys/module" "$R/var/lib/dkms" "$R/run" "$C"
	: > "$R/kmsg"
	: > "$LOG"
}

mkdir -p "$S"
cat > "$S/modprobe" <<EOF
#!/bin/sh
echo "modprobe \$*" >> "$LOG"
if [ "\$1" = --ignore-install ] && [ "\$2" = --show-depends ]; then
	shift 3
	echo "insmod $R/lib/modules/$KVER/updates/dkms/snd-usb-audio.ko.zst index=-2 minifuse_poll_ms=100 \$*"
	exit 0
fi
if [ "\$1" = --ignore-install ] && [ "\$2" = snd_usb_audio ]; then
	if [ -f "$C/load_fails" ]; then cat "$C/load_fails" >&2; exit 1; fi
	mkdir -p "$R/sys/module/snd_usb_audio"
	exit 0
fi
exit 0
EOF
cat > "$S/insmod" <<EOF
#!/bin/sh
echo "insmod \$*" >> "$LOG"
if [ -f "$C/stock_fails" ]; then echo "insmod: ERROR: could not insert module \$1: Invalid argument" >&2; exit 1; fi
mkdir -p "$R/sys/module/snd_usb_audio"
EOF
cat > "$S/modinfo" <<EOF
#!/bin/sh
case "\$1" in
-n) cat "$C/resolved" 2>/dev/null || echo "$R/lib/modules/$KVER/updates/dkms/snd-usb-audio.ko.zst" ;;
-F)
	case "\$2" in
	openfuse) case "\$3" in */updates/*) echo "0.1.0; exact" ;; esac ;;
	depends) echo "snd,snd-ump,snd-usbmidi-lib,mc,snd-pcm,snd-hwdep" ;;
	esac ;;
esac
exit 0
EOF
chmod +x "$S"/*

WRAP="$W/openfuse-modprobe"
sed -e "s|^MODPROBE=.*|MODPROBE=$S/modprobe|" -e "s|^INSMOD=.*|INSMOD=$S/insmod|" \
    -e "s|^MODINFO=.*|MODINFO=$S/modinfo|" -e "s|^MODROOT=.*|MODROOT=$R/lib/modules|" \
    -e "s|^SYSMOD=.*|SYSMOD=$R/sys/module|" -e "s|^DKMSTREE=.*|DKMSTREE=$R/var/lib/dkms|" \
    -e "s|^RUNDIR=.*|RUNDIR=$R/run/openfuse|" -e "s|^KMSG=.*|KMSG=$R/kmsg|" \
    -e "s|^LOGGER=.*|LOGGER=/nonexistent|" \
    "$HERE/packaging/openfuse-modprobe" > "$WRAP"
chmod +x "$WRAP"
grep -q "^MODPROBE=$S/modprobe" "$WRAP" || { echo "wrapper path substitution failed"; exit 1; }

STOCKDIR="$R/lib/modules/$KVER/kernel/sound/usb"
OURS="$R/lib/modules/$KVER/updates/dkms/snd-usb-audio.ko.zst"

run_wrap() { "$WRAP" "$@" 2>>"$W/wrap.stderr"; }

# W1: OpenFuse module loads
new_root; touch "$OURS" "$STOCKDIR/snd-usb-audio.ko.zst"
run_wrap; rc=$?
check W1 "OpenFuse module loads: exit 0, no fallback, no log" \
	bash -c "[ $rc = 0 ] && ! grep -q '^insmod' '$LOG' && [ ! -s '$R/kmsg' ] && [ -d '$R/sys/module/snd_usb_audio' ]"

# W2: rejected (key not enrolled) -> stock loaded with configured options
new_root; touch "$OURS" "$STOCKDIR/snd-usb-audio.ko.zst"
echo "modprobe: ERROR: could not insert 'snd_usb_audio': Key was rejected by service" > "$C/load_fails"
run_wrap foo=1 minifuse_bitfield_rmw=1; rc=$?
check W2a "signature rejected: stock .ko.zst loaded, exit 0" \
	bash -c "[ $rc = 0 ] && grep -q '^insmod $STOCKDIR/snd-usb-audio.ko.zst ' '$LOG'"
check W2b "fallback drops minifuse_* options, keeps index=-2 and command line" \
	bash -c "grep '^insmod' '$LOG' | grep -q 'index=-2' && grep '^insmod' '$LOG' | grep -q 'foo=1' && ! grep '^insmod' '$LOG' | grep -q minifuse"
check W2c "kernel log line 'driver rejected (...), using the standard snd-usb-audio'" \
	grep -q "openfuse: driver rejected (signature rejected.*), using the standard snd-usb-audio" "$R/kmsg"
check W2d "fallback recorded in /run/openfuse/fallback" grep -q "^reason=signature rejected" "$R/run/openfuse/fallback"
check W2e "dependencies of the stock module loaded via modprobe" \
	bash -c "for d in snd snd-ump snd-usbmidi-lib mc snd-pcm snd-hwdep; do grep -q \"^modprobe -q \$d\$\" '$LOG' || exit 1; done"

# W3: stock file variants
for ext in ko.xz ko.gz ko; do
	new_root; touch "$OURS" "$STOCKDIR/snd-usb-audio.$ext"
	echo "modprobe: ERROR: could not insert 'snd_usb_audio': Required key not available" > "$C/load_fails"
	run_wrap; rc=$?
	check "W3" "stock variant .$ext found and loaded" \
		bash -c "[ $rc = 0 ] && grep -q '^insmod $STOCKDIR/snd-usb-audio.$ext' '$LOG'"
done

# W4: in-tree file missing (dkms archived it) -> dkms backup copy used
new_root; touch "$OURS"
mkdir -p "$R/var/lib/dkms/openfuse/original_module/$KVER/x86_64"
touch "$R/var/lib/dkms/openfuse/original_module/$KVER/x86_64/snd-usb-audio.ko.zst"
echo "$STOCKDIR/snd-usb-audio.ko.zst" > "$R/var/lib/dkms/openfuse/original_module/$KVER/x86_64/snd-usb-audio.ko.zst.origin"
echo "modprobe: ERROR: could not insert 'snd_usb_audio': Key was rejected by service" > "$C/load_fails"
run_wrap; rc=$?
check W4 "in-tree file missing: dkms-archived stock module loaded" \
	bash -c "[ $rc = 0 ] && grep -q '^insmod $R/var/lib/dkms/openfuse/original_module/$KVER/x86_64/snd-usb-audio.ko.zst' '$LOG'"

# W5: no stock module anywhere
new_root; touch "$OURS"
echo "modprobe: ERROR: could not insert 'snd_usb_audio': Key was rejected by service" > "$C/load_fails"
run_wrap; rc=$?
check W5 "no stock module anywhere: non-zero exit, clear log line" \
	bash -c "[ $rc != 0 ] && grep -q 'standard module for $KVER was not found' '$R/kmsg'"

# W6: stock module fails as well
new_root; touch "$OURS" "$STOCKDIR/snd-usb-audio.ko.zst"; touch "$C/stock_fails"
echo "modprobe: ERROR: could not insert 'snd_usb_audio': Key was rejected by service" > "$C/load_fails"
run_wrap; rc=$?
check W6 "stock module fails too: non-zero exit, logged" \
	bash -c "[ $rc != 0 ] && grep -q 'standard snd-usb-audio failed too' '$R/kmsg'"

# W7: OpenFuse not installed (modprobe resolves to stock) and load fails -> no second attempt
new_root; touch "$STOCKDIR/snd-usb-audio.ko.zst"; echo "$STOCKDIR/snd-usb-audio.ko.zst" > "$C/resolved"
echo "modprobe: ERROR: could not insert 'snd_usb_audio': Invalid argument" > "$C/load_fails"
run_wrap; rc=$?
check W7 "stock module itself fails: error passed through, no duplicate insmod" \
	bash -c "[ $rc != 0 ] && ! grep -q '^insmod' '$LOG'"

# W8: already loaded
new_root; mkdir -p "$R/sys/module/snd_usb_audio"
run_wrap; rc=$?
check W8 "already loaded: exit 0 without calling modprobe" bash -c "[ $rc = 0 ] && [ ! -s '$LOG' ]"

# W9: version/symbol mismatch
new_root; touch "$OURS" "$STOCKDIR/snd-usb-audio.ko.zst"
echo "modprobe: ERROR: could not insert 'snd_usb_audio': Unknown symbol in module, or unknown parameter" > "$C/load_fails"
run_wrap; rc=$?
check W9 "unknown symbol: stock loaded, reason 'symbol/version mismatch'" \
	bash -c "[ $rc = 0 ] && grep -q 'driver rejected (symbol/version mismatch' '$R/kmsg'"

# W12: any other modprobe failure
new_root; touch "$OURS" "$STOCKDIR/snd-usb-audio.ko.zst"
echo "modprobe: ERROR: could not insert 'snd_usb_audio': Device or resource busy" > "$C/load_fails"
run_wrap; rc=$?
check W12 "other modprobe error: stock loaded, modprobe's message used as reason" \
	bash -c "[ $rc = 0 ] && grep -q 'driver rejected (could not insert .snd_usb_audio.: Device or resource busy)' '$R/kmsg'"

# W10: kernel upgrade without exact sources: nothing of ours for that kernel
new_root; touch "$STOCKDIR/snd-usb-audio.ko.zst"; echo "$STOCKDIR/snd-usb-audio.ko.zst" > "$C/resolved"
run_wrap; rc=$?
check W10 "new kernel, OpenFuse skipped: stock module loads normally through the wrapper" \
	bash -c "[ $rc = 0 ] && [ -d '$R/sys/module/snd_usb_audio' ] && ! grep -q '^insmod' '$LOG' && [ ! -s '$R/kmsg' ]"

# W11: modprobe.d line: wrapper present / missing
LINE="$(sed -n 's/^install snd_usb_audio //p' "$HERE/packaging/openfuse-fallback.conf")"
new_root; touch "$OURS" "$STOCKDIR/snd-usb-audio.ko.zst"
L1="${LINE//\/usr\/libexec\/openfuse\/openfuse-modprobe/$WRAP}"
L1="${L1//\/sbin\/modprobe/$S/modprobe}"
L1="${L1//\$CMDLINE_OPTS/}"
sh -c "$L1"; rc=$?
check W11a "modprobe.d install line runs the wrapper when present" bash -c "[ $rc = 0 ] && [ -d '$R/sys/module/snd_usb_audio' ]"
new_root
L2="${LINE//\/usr\/libexec\/openfuse\/openfuse-modprobe/$W/missing-wrapper}"
L2="${L2//\/sbin\/modprobe/$S/modprobe}"
L2="${L2//\$CMDLINE_OPTS/}"
sh -c "$L2"; rc=$?
check W11b "modprobe.d install line: wrapper missing -> plain modprobe --ignore-install" \
	bash -c "[ $rc = 0 ] && grep -q '^modprobe --ignore-install snd_usb_audio' '$LOG'"
mkdir -p "$W/modprobe.d"; cp "$HERE/packaging/openfuse-fallback.conf" "$W/modprobe.d/"
check W11c "real kmod parses the modprobe.d file (dry run: --show-depends)" \
	bash -c "/sbin/modprobe -C '$W/modprobe.d' --show-depends snd_usb_audio | grep -q '^install if \\[ -x /usr/libexec/openfuse/openfuse-modprobe \\]'"

# ---------------------------------------------------------------------------
# dkms POST_INSTALL: copy the archived stock module back

new_root
B="$R/var/lib/dkms/openfuse/original_module/$KVER/x86_64"
mkdir -p "$B"
echo stock > "$B/snd-usb-audio.ko.zst"
echo "$STOCKDIR/snd-usb-audio.ko.zst" > "$B/snd-usb-audio.ko.zst.origin"
OPENFUSE_DKMS_TREE="$R/var/lib/dkms" OPENFUSE_INSTALL_TREE="$R/lib/modules" \
	sh "$HERE/dkms-post-install.sh" "$KVER" >/dev/null; rc=$?
check P1 "post-install: archived stock module copied back to kernel/sound/usb" \
	bash -c "[ $rc = 0 ] && [ \"\$(cat '$STOCKDIR/snd-usb-audio.ko.zst')\" = stock ] && [ -f '$B/snd-usb-audio.ko.zst' ]"
new_root; mkdir -p "$B"; echo old > "$B/snd-usb-audio.ko.zst"
echo "$R/lib/modules/$KVER/updates/snd-usb-audio.ko.zst" > "$B/snd-usb-audio.ko.zst.origin"
OPENFUSE_DKMS_TREE="$R/var/lib/dkms" OPENFUSE_INSTALL_TREE="$R/lib/modules" \
	sh "$HERE/dkms-post-install.sh" "$KVER" >/dev/null
check P2 "post-install: never restores into updates/ (old third-party/OpenFuse builds)" \
	test ! -e "$R/lib/modules/$KVER/updates/snd-usb-audio.ko.zst"

# ---------------------------------------------------------------------------
# Source selection: exact only by default

T="$W/src"
mkdir -p "$T/cache"
SRC_EXACT=""
for d in "$HERE/.cache"/Ubuntu-* "$HERE/sources"/Ubuntu-*; do
	[ -f "$d/.origin" ] && { SRC_EXACT="$d"; break; }
done
if [ -n "$SRC_EXACT" ]; then
	TAG="$(basename "$SRC_EXACT")"
	cp -a "$SRC_EXACT" "$T/cache/$TAG"
	cp -a "$SRC_EXACT" "$T/cache/upstream-v7.3"
	echo "upstream test copy tag v7.3" > "$T/cache/upstream-v7.3/.origin"
	FS() { OFFLINE=1 OPENFUSE_CACHE="$T/cache" OPENFUSE_BUILD_CONF="$T/build.conf" \
		"$HERE/fetch-sources.sh" "$KVER" "$T/out" 2>"$T/fs.log"; }
	: > "$T/build.conf"

	UBUNTU_TAG="$TAG" FS; rc=$?
	check S1 "exact Ubuntu tag available: used (match: exact)" \
		bash -c "[ $rc = 0 ] && grep -q '^match: exact\$' '$T/out/SOURCE_INFO'"
	UBUNTU_TAG=Ubuntu-0.0.0-0.0 FS; rc=$?
	check S2 "exact tag missing, same-series and upstream trees cached: skip (77) by default" \
		bash -c "[ $rc = 77 ] && [ ! -e '$T/out' ]"
	echo "allow_series_fallback=yes" > "$T/build.conf"
	UBUNTU_TAG=Ubuntu-0.0.0-0.0 FS; rc=$?
	check S3 "opt-in allow_series_fallback=yes: same-series fallback used, marked FALLBACK" \
		bash -c "[ $rc = 0 ] && grep -q '^match: FALLBACK same-series' '$T/out/SOURCE_INFO'"
	echo "allow_upstream_fallback=yes" > "$T/build.conf"
	UBUNTU_TAG=Ubuntu-0.0.0-0.0 FS; rc=$?
	check S4 "opt-in allow_upstream_fallback=yes: upstream used for an Ubuntu kernel, marked FALLBACK" \
		bash -c "[ $rc = 0 ] && grep -q '^match: FALLBACK upstream' '$T/out/SOURCE_INFO'"
	: > "$T/build.conf"
	UBUNTU_TAG=- FS; rc=$?
	check S5 "non-Ubuntu kernel: exact upstream version used by default" \
		bash -c "[ $rc = 0 ] && grep -q '^match: exact upstream' '$T/out/SOURCE_INFO'"
	echo "allow_series_fallback = maybe" > "$T/build.conf"
	UBUNTU_TAG=Ubuntu-0.0.0-0.0 FS; rc=$?
	check S6 "invalid config value treated as 'no'" bash -c "[ $rc = 77 ]"

	# build-module.sh skip leaves nothing behind
	P="$W/pkg"
	mkdir -p "$P"
	cp -a "$HERE"/{build-module.sh,fetch-sources.sh,openfuse-mok.sh,mixer_minifuse.c,mixer_minifuse.h,dkms.conf,patches} "$P/"
	UBUNTU_TAG=Ubuntu-0.0.0-0.0 OFFLINE=1 OPENFUSE_CACHE="$T/cache" OPENFUSE_BUILD_CONF="$T/none.conf" \
		"$P/build-module.sh" "$KVER" >/dev/null 2>&1; rc=$?
	check S7 "build-module.sh with non-exact sources only: exit 77, no build output" \
		bash -c "[ $rc = 77 ] && [ ! -e '$P/build/$KVER' ]"
	# exact sources that do not compile -> skip, nothing left
	cp -a "$T/cache/$TAG" "$T/cache/broken"
	echo "#error deliberately broken for the test" >> "$T/cache/broken/card.c"
	rm -rf "$T/cache/$TAG"; mv "$T/cache/broken" "$T/cache/$TAG"
	UBUNTU_TAG="$TAG" OFFLINE=1 OPENFUSE_CACHE="$T/cache" OPENFUSE_BUILD_CONF="$T/none.conf" \
		"$P/build-module.sh" "$KVER" >"$T/s8.log" 2>&1; rc=$?
	check S8 "exact sources that do not compile: exit 77, no build output" \
		bash -c "[ $rc = 77 ] && [ ! -e '$P/build/$KVER' ] && grep -q 'deliberately broken' '$T/s8.log'"
else
	result S0 "source-selection tests (need a cached or bundled Ubuntu tree; run make first)" FAIL
fi

# ---------------------------------------------------------------------------
# Removal: prerm / postrm

mkscript() {	# name -> $W/<name>, cleanup inserted, paths substituted
	sed -e "s/@VERSION@/0.1.0/" -e "/^@CLEANUP@\$/{r $HERE/packaging/debian/cleanup.sh.in" -e "d}" \
	    "$HERE/packaging/debian/$1" |
	sed -e "s|^MODROOT=.*|MODROOT=$R/lib/modules|" -e "s|^DKMSTREE=.*|DKMSTREE=$R/var/lib/dkms|" \
	    -e "s|^DEPMOD=.*|DEPMOD=$S/depmod|" -e "s|^MODINFO=.*|MODINFO=$S/modinfo|" \
	    -e "s|^RUNDIR=.*|RUNDIR=$R/run/openfuse|" -e "s|^CACHEDIR=.*|CACHEDIR=$R/var/cache/openfuse|" \
	    -e "s|^CONFDIR=.*|CONFDIR=$R/etc/openfuse|" -e "s|^SRCROOT=.*|SRCROOT=$R/usr/src|" > "$W/$1"
	chmod +x "$W/$1"
}
mkscript prerm
mkscript postrm
printf '#!/bin/sh\necho "depmod $*" >> "%s"\n' "$LOG" > "$S/depmod"
chmod +x "$S/depmod"
DK="$W/dkmsbin"
mkdir -p "$DK"
cat > "$DK/dkms" <<EOF
#!/bin/sh
# stub dkms: an installed openfuse/0.1.0 for $KVER; remove restores like dkms
echo "dkms \$*" >> "$LOG"
case "\$1" in
status) [ -f "$C/dkms_installed" ] && echo "openfuse/0.1.0, $KVER, x86_64: installed"; exit 0 ;;
remove)
	rm -f "$OURS"
	b="$R/var/lib/dkms/openfuse/original_module/$KVER/x86_64"
	[ -f "\$b/snd-usb-audio.ko.zst" ] && mv -f "\$b/snd-usb-audio.ko.zst" "$STOCKDIR/snd-usb-audio.ko.zst"
	rm -rf "$R/var/lib/dkms/openfuse"; rm -f "$C/dkms_installed"; exit 0 ;;
esac
EOF
chmod +x "$DK/dkms"

baseline() {	# pristine system: only the distribution's module
	new_root
	echo stock > "$STOCKDIR/snd-usb-audio.ko.zst"
	(cd "$R" && find . -path ./sys -prune -o -path ./run -prune -o -type f -print | sort | xargs md5sum) > "$W/baseline.md5"
}
installed_state() {	# what dkms install + our POST_INSTALL leave
	mkdir -p "$B" "$R/run/openfuse" "$R/var/cache/openfuse/x" "$R/etc/openfuse" "$R/usr/src/openfuse-0.1.0/build/x"
	mv "$STOCKDIR/snd-usb-audio.ko.zst" "$B/"
	echo "$STOCKDIR/snd-usb-audio.ko.zst" > "$B/snd-usb-audio.ko.zst.origin"
	cp -p "$B/snd-usb-audio.ko.zst" "$STOCKDIR/"
	echo ours > "$OURS"
	echo "reason=x" > "$R/run/openfuse/fallback"
	echo "allow_series_fallback=no" > "$R/etc/openfuse/build.conf"
	touch "$C/dkms_installed"
}

# R1: remove with dkms present
baseline; installed_state
PATH="$DK:$PATH" "$W/prerm" remove; r1=$?
PATH="$DK:$PATH" "$W/postrm" remove; r2=$?
check R1a "remove (dkms present): scripts exit 0" bash -c "[ $r1 = 0 ] && [ $r2 = 0 ]"
check R1b "remove: OpenFuse module gone, stock module in place" \
	bash -c "[ ! -e '$OURS' ] && [ \"\$(cat '$STOCKDIR/snd-usb-audio.ko.zst')\" = stock ]"
check R1c "remove: depmod run for the affected kernel" grep -q "^depmod -a $KVER\$" "$LOG"
check R1d "remove: runtime state removed" test ! -e "$R/run/openfuse"
check R1e "remove: config kept (Debian semantics), cache kept" \
	bash -c "[ -f '$R/etc/openfuse/build.conf' ] && [ -d '$R/var/cache/openfuse' ]"
PATH="$DK:$PATH" "$W/postrm" purge; r3=$?
(cd "$R" && find . -path ./sys -prune -o -path ./run -prune -o -type f -print | sort | xargs md5sum) > "$W/after.md5"
check R1f "purge: no file of ours left (diff against pre-install snapshot)" diff "$W/after.md5" "$W/baseline.md5"
check R1g "purge: no directories of ours left" \
	bash -c "[ ! -e '$R/var/cache/openfuse' ] && [ ! -e '$R/etc/openfuse' ] && [ ! -e '$R/usr/src/openfuse-0.1.0' ] && [ ! -e '$R/var/lib/dkms/openfuse' ]"

# R2: remove with dkms already gone (stock module only in the dkms archive)
baseline; installed_state; rm -f "$STOCKDIR/snd-usb-audio.ko.zst"
"$W/prerm" remove; r1=$?
"$W/postrm" purge; r2=$?
(cd "$R" && find . -path ./sys -prune -o -path ./run -prune -o -type f -print | sort | xargs md5sum) > "$W/after.md5"
check R2a "dkms missing: prerm/postrm exit 0" bash -c "[ $r1 = 0 ] && [ $r2 = 0 ]"
check R2b "dkms missing: our module removed, stock restored from the dkms archive, depmod run" \
	bash -c "[ ! -e '$OURS' ] && [ \"\$(cat '$STOCKDIR/snd-usb-audio.ko.zst')\" = stock ] && grep -q '^depmod -a $KVER\$' '$LOG'"
check R2c "dkms missing: purge leaves the system identical to before installation" diff "$W/after.md5" "$W/baseline.md5"

# R3: nothing installed (module already gone)
baseline
"$W/prerm" remove; r1=$?
PATH="$DK:$PATH" "$W/postrm" purge; r2=$?
(cd "$R" && find . -path ./sys -prune -o -path ./run -prune -o -type f -print | sort | xargs md5sum) > "$W/after.md5"
check R3 "module/dkms already gone: exit 0, nothing changed, no depmod" \
	bash -c "[ $r1 = 0 ] && [ $r2 = 0 ] && diff -q '$W/after.md5' '$W/baseline.md5' && ! grep -q '^depmod' '$LOG'"

# ---------------------------------------------------------------------------
# Helper status: fallback_active

sed -e "s/@VERSION@/0.1.0/" -e "s|^LIBDIR=.*|LIBDIR=$HERE|" -e "s|^SRCDIR=.*|SRCDIR=$R/usr/src/openfuse-0.1.0|" \
    -e "s|^SYSMOD=.*|SYSMOD=$R/sys/module|" -e "s|^RUNDIR=.*|RUNDIR=$R/run/openfuse|" \
    -e "s|^PATH=.*|PATH=$DK:$S:/usr/sbin:/usr/bin:/sbin:/bin|" \
    "$HERE/packaging/openfuse-setup.in" > "$W/openfuse-setup"
chmod +x "$W/openfuse-setup"
new_root; touch "$C/dkms_installed"; mkdir -p "$R/sys/module/snd_usb_audio" "$R/run/openfuse"
echo "reason=signature rejected: signing key not trusted" > "$R/run/openfuse/fallback"
"$W/openfuse-setup" status > "$W/status.out" 2>/dev/null
check H1 "status: OpenFuse installed + stock loaded -> fallback_active=yes, reason shown" \
	bash -c "grep -qx 'fallback_active=yes' '$W/status.out' && grep -q '^fallback_reason=signature rejected' '$W/status.out'"
mkdir -p "$R/sys/module/snd_usb_audio/parameters"; touch "$R/sys/module/snd_usb_audio/parameters/minifuse_poll_ms"
rm -f "$R/run/openfuse/fallback"
"$W/openfuse-setup" status > "$W/status.out" 2>/dev/null
check H2 "status: OpenFuse module loaded -> fallback_active=no" \
	bash -c "grep -qx 'module_loaded=openfuse' '$W/status.out' && grep -qx 'fallback_active=no' '$W/status.out'"

# ---------------------------------------------------------------------------
# Signing keys: module-signing-only, existing keys never touched

KD="$W/keys"
mkdir -p "$KD"
(
	. "$HERE/openfuse-mok.sh"
	mok_signing_key="$KD/new/mok.key" mok_certificate="$KD/new/mok.der" of_ensure_key
) ; rc=$?
openssl x509 -in "$KD/new/mok.der" -inform DER -noout -text > "$KD/new.txt" 2>/dev/null
check K1a "new key created at the configured dkms path" bash -c "[ $rc = 0 ] && [ -s '$KD/new/mok.key' ] && [ -s '$KD/new/mok.der' ]"
check K1b "new cert: EKU = Code Signing + 1.3.6.1.4.1.2312.16.1.2 (module signing only)" \
	bash -c "grep -A1 'Extended Key Usage' '$KD/new.txt' | grep -q 'Code Signing, 1.3.6.1.4.1.2312.16.1.2'"
check K1c "new cert: basicConstraints critical CA:FALSE, keyUsage Digital Signature, SKI present" \
	bash -c "grep -A1 'Basic Constraints: critical' '$KD/new.txt' | grep -q 'CA:FALSE' && grep -A1 'X509v3 Key Usage' '$KD/new.txt' | grep -q 'Digital Signature' && grep -q 'Subject Key Identifier' '$KD/new.txt'"
check K1d "new private key is mode 600" bash -c "[ \"\$(stat -c %a '$KD/new/mok.key')\" = 600 ]"
check K1e "of_key_modsign_only reports yes for the new cert" \
	bash -c ". '$HERE/openfuse-mok.sh'; [ \"\$(of_key_modsign_only '$KD/new/mok.der')\" = yes ]"
check K1f "no temporary openssl config left next to the key" bash -c "! ls -A '$KD/new' | grep -q '^.openfuse-modsign'"

mkdir -p "$KD/old"
openssl req -new -x509 -noenc -days 1 -subj "/CN=existing unrestricted" -newkey rsa:2048 \
	-keyout "$KD/old/mok.key" -outform DER -out "$KD/old/mok.der" >/dev/null 2>&1
( cd "$KD/old" && md5sum mok.key mok.der ) > "$KD/old.md5"
(
	. "$HERE/openfuse-mok.sh"
	mok_signing_key="$KD/old/mok.key" mok_certificate="$KD/old/mok.der" of_ensure_key
) ; rc=$?
check K2a "existing (unrestricted) key pair: accepted, left byte-for-byte untouched" \
	bash -c "[ $rc = 0 ] && cd '$KD/old' && md5sum -c --quiet '$KD/old.md5'"
check K2b "existing unrestricted cert reported as key_modsign_only=no" \
	bash -c ". '$HERE/openfuse-mok.sh'; [ \"\$(of_key_modsign_only '$KD/old/mok.der')\" = no ]"

mkdir -p "$KD/half"; cp "$KD/old/mok.der" "$KD/half/mok.der"
(
	. "$HERE/openfuse-mok.sh"
	mok_signing_key="$KD/half/mok.key" mok_certificate="$KD/half/mok.der" of_ensure_key
) ; rc=$?
check K3 "half-present pair (cert without key): failure reported, nothing created or deleted" \
	bash -c "[ $rc != 0 ] && [ ! -e '$KD/half/mok.key' ] && cmp -s '$KD/half/mok.der' '$KD/old/mok.der'"

mok_signing_key="$KD/new/mok.key" mok_certificate="$KD/new/mok.der" "$W/openfuse-setup" status > "$W/status.out" 2>/dev/null
check K4a "helper status: key_modsign_only=yes for a restricted key" grep -qx "key_modsign_only=yes" "$W/status.out"
mok_signing_key="$KD/old/mok.key" mok_certificate="$KD/old/mok.der" "$W/openfuse-setup" status > "$W/status.out" 2>/dev/null
check K4b "helper status: key_modsign_only=no for an existing unrestricted key" grep -qx "key_modsign_only=no" "$W/status.out"

# ---------------------------------------------------------------------------

echo
echo "ID   RES  SCENARIO"
printf '%s\n' "${RESULTS[@]}"
echo
echo "passed: $PASS  failed: $FAIL"
[ "$FAIL" = 0 ]
