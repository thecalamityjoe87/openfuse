# OpenFuse kernel part: MiniFuse mixer quirk for `snd-usb-audio`

This directory adds Arturia MiniFuse 2 (`1c75:af90`) control support to the
Linux USB audio driver as a device-specific mixer quirk. The front-panel
switches, the loopback mixer and the level meters show up as ordinary ALSA
controls (`amixer`, `alsamixer`, the OpenFuse GUI).

The device's control requests go to USB interface 0, which `snd-usb-audio`
owns, so they have to be sent from inside that driver (protocol spec §2.1).
Because `snd-usb-audio` is built from many source files, the DKMS package
rebuilds the whole module, out of tree, from the `sound/usb` sources of each
kernel plus this quirk. When it is installed, it replaces the distribution's
`snd-usb-audio` for that kernel. Kernels it cannot build for keep the stock
module.

| File | Purpose |
|---|---|
| `mixer_minifuse.c`, `mixer_minifuse.h` | the quirk (upstream style, GPL-2.0) |
| `patches/0001-*.patch` | hooks the quirk into `sound/usb/mixer_quirks.c` and `sound/usb/Makefile` |
| `patches/0002-*.patch` | passes each MiniFuse playback URB to the quirk from `sound/usb/pcm.c`, for the playback meter |
| `fetch-sources.sh` | finds the `sound/usb` sources for a kernel, applies the patch, adds the quirk |
| `build-module.sh` | builds for one kernel; retries other sources if one fails; DKMS `PRE_BUILD` step with the Secure Boot guard |
| `openfuse-mok.sh` | shared Secure Boot / MOK helpers (key paths as dkms chooses them, enrolment state, module-signing-only key creation) |
| `dkms-post-install.sh` | DKMS `POST_INSTALL`: puts a copy of the distribution's module back in `kernel/sound/usb/` (§2) |
| `Makefile` | user-level build, `make deb`, `make checkpatch` |
| `dkms.conf` | DKMS configuration (module name `openfuse`) |
| `packaging/` | `mkdeb.sh`; the Debian control files and maintainer scripts; the installer helper `openfuse-setup.in` and its polkit policy; the load-time fallback `openfuse-modprobe` and `openfuse-fallback.conf` (§9) |
| `tests/failure-modes.sh` | failure-mode tests, run as a normal user with stubs (`make test`, §9) |

An upstream submission is `patches/0001-*.patch` and `patches/0002-*.patch` with the two new files
added to the same commit.

---

## 1. Build (as a normal user)

Requirements: `linux-headers-$(uname -r)`, `build-essential` (gcc, make),
`patch`, and `curl` or `wget` (for downloads). `git` is optional, used as a
download fallback.

```sh
cd kernel
make                       # builds for the running kernel
make KVER=7.3.0-9-generic  # or for another installed kernel
```

The result is `build/<kver>/snd-usb-audio.ko`. Nothing is installed and no
root access is needed. Other targets:

- `make deb`: the Debian package (§2);
- `make checkpatch`;
- `make clean`: removes `build/`;
- `make distclean`: also removes the download cache `.cache/` and `dist/`.

The `pahole version differs` / `Skipping BTF generation` messages are
harmless; they only mean the `dwarves` package is not installed, so the
module has no BTF debug info.

Check that the build is the OpenFuse one, and which sources it used:

```sh
modinfo -F openfuse build/$(uname -r)/snd-usb-audio.ko
# 0.1.0; exact; sound/usb from: Ubuntu https://git.launchpad.net/... tag Ubuntu-7.3.0-10.10
cat build/$(uname -r)/SOURCE_INFO
```

### How the sources are chosen

`fetch-sources.sh [KVER] [OUTDIR]` walks an ordered list of candidates. It
takes the first one it can obtain and that the patch applies to. If that
tree then fails to compile, `build-module.sh` moves on to the next one.

**By default only exact matches are used.** A module built from sources
that do not exactly match the running kernel could subtly change behaviour
for every USB audio device. In that case the build is skipped (exit 77)
and the distribution's module stays in use.

| # | Candidate | Default |
|---|---|---|
| 0 | `KSRC=<dir>`, a local kernel tree (only if set), e.g. from `apt-get source linux` | used (your choice) |
| 1 | **EXACT**: the Ubuntu git tag of this kernel's headers package, from the download cache, then the bundled `sources/`, then Launchpad | used |
| 1u | **EXACT UPSTREAM**: for kernels that are not Ubuntu packages, kernel.org at exactly `v<VERSION>.<PATCHLEVEL>[.<SUBLEVEL>]` | used |
| 2 | CACHE: other trees in the download cache for the same `VERSION.PATCHLEVEL`, newest first | off (`allow_series_fallback=yes`) |
| 3 | BUNDLED: trees in `sources/` for the same series, newest first | off (`allow_series_fallback=yes`) |
| 4 | UPSTREAM for an Ubuntu kernel (base version, without Ubuntu's patches) | off (`allow_upstream_fallback=yes`) |

The opt-in switches are in `/etc/openfuse/build.conf`. Its `KEY=VALUE`
lines are parsed, never sourced, and anything other than `yes`/`true`/
`1`/`on` means no:

```
allow_series_fallback=no
allow_upstream_fallback=no
```

`ALLOW_FALLBACK=1` / `ALLOW_UPSTREAM=1` override the file for one build.

Each step logs which candidate it uses (`fetch-sources: candidate N (...)`).
A non-exact build prints a `WARNING ... NOT an exact match` block. The match
is recorded in `SOURCE_INFO` and in the module's `modinfo -F openfuse`
string. If no candidate works, the script exits with status 77 and leaves
no build directory behind.

How the exact match is found on Ubuntu:

- Ubuntu kernels are built from the source package whose version is the
  version of the `linux-headers-<kver>` package, and every upload is
  tagged in Launchpad git. `dpkg-query -W linux-headers-7.3.0-10-generic`
  gives version `7.3.0-10.10` and source `linux`, so the tag is
  `Ubuntu-7.3.0-10.10`.
- The repo is
  `https://git.launchpad.net/~ubuntu-kernel/ubuntu/+source/linux/+git/<series>`,
  with `<series>` = `VERSION_CODENAME` from `/etc/os-release` (`stonking`
  on 26.10).
- Derivative kernels carry their flavour in the tag, with `~` turned into
  `_` (for example `linux-hwe-7.3 7.3.0-10.10~26.04.1` →
  `Ubuntu-hwe-7.3-7.3.0-10.10_26.04.1`).
- For the running kernel, `/proc/version_signature` is used when dpkg has
  no answer.
- Only the top-level files of `sound/usb/` are downloaded, through cgit's
  `plain/` URLs (about 50 files). The fallback is a shallow, sparse,
  blob-less `git clone` of the tag (slower, about 300 MB).

Overrides: `KSRC`, `UBUNTU_SERIES`, `UBUNTU_TAG` (`-` = not an Ubuntu
kernel), `UPSTREAM_TAG`, `ALLOW_FALLBACK`, `ALLOW_UPSTREAM`,
`OPENFUSE_BUILD_CONF=<file>`, `OPENFUSE_CACHE=<dir>`, `OFFLINE=1` (no
downloads).

The download cache is `.cache/` for user builds and `/var/cache/openfuse`
for DKMS builds; DKMS recreates its build directory for every build. All
temporary files are created inside the cache directory, never in `/tmp`.

The module is built from the patched upstream `Makefile`'s object list, so
the file set and config options always match the sources. Only
`snd-usb-audio.ko` is built; `snd-usbmidi-lib` and the other USB audio
drivers stay the distribution's.

---

## 2. Install (Debian package with DKMS, root required)

Build the package as a normal user:

```sh
make                      # for each kernel whose sources should be bundled
make KVER=7.3.0-9-generic
make deb                  # -> dist/openfuse-dkms_0.1.0_all.deb
dpkg-deb --info dist/openfuse-dkms_0.1.0_all.deb
```

`make deb` bundles the pristine `sound/usb` trees of every kernel built so
far (from `build/*/SOURCE_INFO`) into `/usr/src/openfuse-0.1.0/sources/`.
This is the offline fallback. It uses `fakeroot` if it is installed,
otherwise `dpkg-deb --root-owner-group`.

Package contents:

- `/usr/src/openfuse-0.1.0/`: `dkms.conf`, `Makefile`, the scripts, the
  quirk, `patches/`, and `sources/<tag>/` (each with `.origin` and
  `.series`);
- `/usr/libexec/openfuse/openfuse-setup` (the installer helper, §8),
  `/usr/libexec/openfuse/openfuse-modprobe` (the load-time fallback, §9)
  and `/usr/libexec/openfuse/openfuse-mok.sh`;
- `/usr/lib/modprobe.d/openfuse-fallback.conf` (§9). It is a package file,
  not a conffile, so it disappears with the package;
- `/usr/share/polkit-1/actions/org.openfuse.setup.policy`;
- `/usr/share/doc/openfuse-dkms/`: README, copyright, changelog.

Dependencies:

- **Depends:** `dkms`, `patch`, `make`. dkms itself pulls gcc, kmod and
  dpkg-dev.
- **Recommends:** `curl | wget` (downloads; offline builds use the bundle),
  `mokutil` (Secure Boot enrolment) and `pkexec` (to run the helper).
- **Suggests:** `git`, `zstd` (without zstd, DKMS installs the module
  uncompressed).
- There is deliberately no dependency on a `linux-headers-*` package: the
  installer provides the headers for the running kernel.

Install:

```sh
sudo apt install linux-headers-$(uname -r)    # if not yet installed
sudo apt install ./dist/openfuse-dkms_0.1.0_all.deb
```

The maintainer scripts behave like this:

- **postinst (configure):**
  1. Makes sure the key dkms signs with exists, creating it the way the
     system would, but without enrolling it (§3).
  2. `dkms add openfuse/0.1.0`.
  3. `dkms install -k <kver>` for every kernel in `/lib/modules/*` that
     has headers.

  A kernel that fails or is skipped only produces a warning. The postinst
  always exits 0, so apt never breaks. On a Secure Boot machine whose key
  is not yet enrolled or queued, every kernel is skipped until
  `openfuse-setup enroll` runs.
- **prerm (remove/upgrade):** `dkms remove openfuse/0.1.0 --all`, which
  brings back the stock module. Failures only warn.
- **postrm purge:** deletes `/var/cache/openfuse`.

DKMS installs the module to `/lib/modules/<kver>/updates/dkms/`. `depmod`
searches `updates/` before `kernel/` (`/etc/depmod.d/ubuntu.conf`), so
this copy wins.

dkms (3.2.2) then **moves** the distribution's
`kernel/sound/usb/snd-usb-audio.ko.zst` into
`/var/lib/dkms/openfuse/original_module/<kver>/<arch>/` and puts it back
only on `dkms remove`. Our `POST_INSTALL` step (`dkms-post-install.sh`)
copies it straight back. The stock module therefore always stays on disk
for the fallback and for recovery, even if the DKMS state gets lost; dkms
keeps its own copy. Check with:

```sh
dkms status openfuse
modinfo -n snd-usb-audio        # .../updates/dkms/snd-usb-audio.ko.zst
modinfo -F openfuse snd-usb-audio
modinfo -F signer snd-usb-audio # your MOK, see §3
```

### Kernel updates, offline builds and failures

DKMS runs `build-module.sh --dkms` as its `PRE_BUILD` step for every new
kernel (`AUTOINSTALL=yes`):

- **Network available:** the exact Ubuntu tag is downloaded and cached in
  `/var/cache/openfuse`.
- **Offline:** an exact tree from the cache or the bundle is used if there
  is one. Non-exact trees are only used if enabled in
  `/etc/openfuse/build.conf`.
- **No exact sources, or nothing compiles:** `PRE_BUILD` exits with 77.
  DKMS reports this as **skipped**: nothing is built or installed for that
  kernel, `dkms status` shows `added`, the distribution's `snd-usb-audio`
  is used, and `dkms autoinstall` does not fail the kernel package
  installation. (dkms 3.2 counts only real build failures as errors; those
  would make the kernel's postinst hook fail.) Nothing of ours shadows the
  stock module on that kernel: dkms only archives the original during an
  install, which never happens. The modprobe wrapper simply loads the stock
  module (test W10).
- **Retry later:** when the network or sources are available again, run
  `sudo /usr/libexec/openfuse/openfuse-setup build` (running kernel),
  `sudo dkms install openfuse/0.1.0 -k <kver>`, or
  `sudo dpkg-reconfigure openfuse-dkms` (all kernels).

These paths were tested as a normal user, with the dkms 3.2.2 script and
private `--dkmstree` / `--sourcetree` directories:

- `dkms add` and `dkms build` from the package, offline with an empty
  cache: exact match from the bundle, module built, signed and compressed;
- a non-exact source situation skips with exit 77;
- the Secure Boot guard: an unenrolled key skips with exit 77, and a
  pending key (stubbed mokutil) allows the build.

`make test` covers the fallback wrapper, the post-install step, source
selection, removal/purge, the helper's `status` and the key creation (§9).

Not run, because they need root: `dkms install` (including the real
original-module archiving and our `POST_INSTALL`), the maintainer scripts
on a real system, real `mokutil --import`, `pkexec`, and loading modules.

### Loading the new module

Use one of these:

- reboot; or
- stop audio users (`systemctl --user stop pipewire pipewire-pulse wireplumber`),
  then `sudo modprobe -r snd-usb-audio && sudo modprobe snd-usb-audio`
  (this affects every USB audio device), and replug the MiniFuse.

Then check `sudo dmesg | grep -i minifuse`. Expected output:
`Arturia MiniFuse 2: 27 controls, firmware 1.50`. If the device does not
answer the state request, you see `cannot read device state ..., controls
disabled`, and audio keeps working without the extra controls.

---

## 3. Secure Boot and kernel lockdown (MOK signing)

The target machine runs with Secure Boot and lockdown enabled
(`/sys/kernel/security/lockdown` shows `[integrity]`). The kernel then
refuses modules that are unsigned or signed with an unknown key. For
`snd-usb-audio` that would mean **no USB audio at all**, because the
DKMS copy shadows the stock module.

DKMS signs the module automatically, with the key chosen like this
(`openfuse-mok.sh` applies the same rules):

1. `mok_signing_key` / `mok_certificate` from `/etc/dkms/framework.conf*`;
2. else the same variables from the environment;
3. else, on Ubuntu, `/var/lib/shim-signed/mok/MOK.priv` + `MOK.der`, or on
   other distributions `/var/lib/dkms/mok.key` + `mok.pub`.

**Key creation.** The package postinst (and `openfuse-setup enroll`)
creates the key if it is missing, without enrolling it:

- **Ubuntu default path:** `update-secureboot-policy --new-key`
  (shim-signed). Its `/usr/lib/shim/mok/openssl.cnf` already restricts
  the key.
- **Configured or other paths, or that tool missing:** OpenFuse creates
  the key itself with openssl, at exactly the path dkms is configured to
  use, with these extensions:
  - `extendedKeyUsage = codeSigning, 1.3.6.1.4.1.2312.16.1.2`
    (module signing only);
  - `basicConstraints = critical,CA:FALSE`;
  - `keyUsage = digitalSignature`;
  - subject/authority key identifiers.

  The private key is mode 600, root-only. `dkms generate_mok` is not used.
- **An existing key is never replaced or modified,** because other DKMS
  modules may depend on it. A half-present pair (certificate without key,
  or the reverse) is left alone and reported as a failure.

**What the restriction does.** shim refuses a MOK carrying the module-signing
OID for boot components, so this key can never be used to sign a boot loader
or kernel that shim would start. Linux has no per-module allowlist, though:
an enrolled key can sign *any* kernel module. Its protection is that the
private key stays root-only on this machine. `openfuse-setup status` reports
`key_modsign_only=yes|no|unknown`, by parsing the certificate's
extendedKeyUsage. For an existing, unrestricted key the installer shows a
note; OpenFuse does not touch that key.

**Guard.** When signature enforcement is active (`mokutil --sb-state`, or
lockdown other than `[none]`), the DKMS `PRE_BUILD` step builds only if
`mokutil --test-key <cert>` reports the key as:

- `enrolled` (`is already enrolled` / `is already in db`), or
- `pending` (`is already in the enrollment request`).

Anything else (`not-enrolled`, `blocked`, `no-key`, `unknown`) skips the
build with exit 77: nothing is installed and the stock module stays in use.
Override at your own risk with `OPENFUSE_SKIP_MOK_CHECK=1`.

### Installer flow (one authentication, one reboot)

1. PackageKit installs `openfuse-dkms` (and the app). postinst creates the
   key. The build is skipped because the key is neither enrolled nor
   pending.
2. The installer uses the fixed MOK password **`openfuse`** (all lowercase)
   and tells the user to type it in MokManager.
3. The installer runs once, as root:
   `pkexec /usr/libexec/openfuse/openfuse-setup enroll`, writing
   `openfuse\nopenfuse\n` to its stdin. This queues the key
   (`mokutil --import`), then builds and installs the module for the
   running kernel. The build is allowed now because the key is pending.
4. The user reboots. shim runs before the kernel and shows MokManager:
   *Enroll MOK → Continue → Yes → password*. The key is in MokListRT on
   that same boot, so the new `snd-usb-audio` loads.

**If the user skips or fails the MokManager step,** the key is not
enrolled and the kernel rejects the OpenFuse `snd-usb-audio` on that boot.
USB audio keeps working anyway: the modprobe wrapper (§9) loads the
distribution's module instead, at boot and on every hotplug, and logs
`openfuse: driver rejected (signature rejected: ...), using the standard
snd-usb-audio`. Only the MiniFuse controls are missing. After the reboot,
the installer sees this in `openfuse-setup status`:

- `fallback_active=yes` and `fallback_reason=...`, together with
- `signature_enforced=yes`, `dkms_state=installed`, and `key_state` not
  `enrolled`.

It then offers:

- **re-enrollment:** `enroll` again, then reboot; or
- **restore the stock driver:** `restore-stock`. This removes the module
  for the running kernel, so the failed attempt at every load stops.

The pending request expires only when MokManager handles it. While a key
is pending, future kernels also get the module, and the same fallback
covers them.

**Fixed password trade-off.** The installer always uses `openfuse`. The
password only proves, at the MokManager screen, that the person at the
keyboard approves a request that root queued. A public password means that
software which already has root could queue its own key with the same
password and hope the user approves it at the next boot. Such software
already controls the system, so the fixed password adds no new capability
for it. MokManager also shows the key's details before enrolling.
Manual users can choose their own password; the helper accepts any valid
one.

Manual equivalent:

```sh
printf '%s\n%s\n' "$PW" "$PW" | sudo /usr/libexec/openfuse/openfuse-setup enroll
sudo reboot     # confirm the key in MokManager
/usr/libexec/openfuse/openfuse-setup status
```

To test a hand-built module without DKMS, sign it manually (kernel hash
SHA-512):

```sh
sudo /lib/modules/$(uname -r)/build/scripts/sign-file sha512 \
     /var/lib/shim-signed/mok/MOK.priv /var/lib/shim-signed/mok/MOK.der \
     build/$(uname -r)/snd-usb-audio.ko
```

Do not disable Secure Boot or lockdown to work around this.

---

## 4. Verify with ALSA

```sh
cat /proc/asound/cards                 # find the MiniFuse card number
CARD=$(awk '/MiniFuse/ {print $1; exit}' /proc/asound/cards)
amixer -c $CARD contents
```

Expected controls for the MiniFuse 2:

| Control | Type | Backing (spec) |
|---|---|---|
| `Line In 1 Inst Switch`, `Line In 2 Inst Switch` | boolean | state[0x00] bit 0 / bit 1 |
| `Phantom Power Switch` | boolean | state[0x04] |
| `Direct Monitor Mono Switch` | boolean | state[0x05] |
| `Loopback Switch` | boolean | state[0x13] |
| `Loopback To USB In Switch` | boolean | state[0x14] |
| `Loopback To Monitor Switch` | boolean | state[0x15] |
| `Loopback Ch N Left Volume`, N = 1..6 | integer, raw Q8.8 dB (1/256 dB), dB TLV | state[0x06 + N-1] |
| `Loopback Ch N Right Volume`, N = 1..6 | same | state[0x0C + N-1] |
| `Loopback Ch N Mute`, N = 1..6 | boolean, 1 = muted | state[0x16] bit N-1 |
| `Level Meter` | read-only, volatile, 4 × integer64, raw u32 | GET_METERS: in 1, in 2, loopback L, loopback R |
| `Playback Level Meter` | read-only, volatile, 4 × integer64, raw u32 | not from the device: the driver's peak of the samples it sends, per playback channel (1, 2 = main output), since the previous read |
| `Device State Raw` | read-only bytes (48) | the cached GET_STATE block, little-endian |

`Ch N` in a control name means loopback channel N-1 in the spec (spec
channels are numbered 0–5).

Volume values are the raw device values: `0` = 0 dB, `-768` = −3 dB,
`3072` = +12 dB, and the minimum `-16130` (0xC0FE, −63.008 dB) is shown as
mute. `amixer` also accepts dB through the TLV, for example
`amixer -c $CARD cset name='Loopback Ch 1 Left Volume' -- -6dB`.

Meter values are raw. The format is strongly indicated but unverified (spec
§5): each value is the peak absolute 32-bit sample value, so
`dBFS = 20·log10(v / 2^31)` and 0 means silence. The kernel does not
convert; do it in userspace.

The device does not meter playback (spec §5). `Playback Level Meter` is
computed in the driver instead: `snd_minifuse_playback_tap()` is called from
`prepare_playback_urb()` with the data of every playback URB, so it sees
exactly what leaves the computer, after any software volume, whichever
application plays it (including one that opens `hw:` directly and bypasses
the sound server). Supported sample formats: S16_LE, S24_3LE, S32_LE. Each
value is the peak since the previous read, so there should be one reader;
a second reader (`amixer` while OpenFuse runs) takes peaks from it.

Examples:

```sh
amixer -c $CARD cset name='Phantom Power Switch' off
amixer -c $CARD cget name='Level Meter'
amixer -c $CARD cget name='Device State Raw'
amixer -c $CARD events     # or: alsactl monitor
```

`amixer events` should print a change event within about 0.5 s when you
press INST 1/2, 48V or DIRECT MONO on the device, because the driver polls
the state block.

### Module parameters

These can be changed at runtime through
`/sys/module/snd_usb_audio/parameters/` (root), or set persistently in
`/etc/modprobe.d/openfuse-options.conf` as `options snd_usb_audio ...`
(not `openfuse-fallback.conf`, which belongs to the package).

| Parameter | Default | Meaning |
|---|---|---|
| `minifuse_poll_ms` | 500 | state polling interval in ms; 0 = off (then the cache only refreshes on writes and resume); minimum 50 |
| `minifuse_bitfield_rmw` | N | bitfield write strategy, see Q1 below: N = one bit per request (index = bit number, value 0/1), Y = read-modify-write of the whole bitfield |

Polling does not take a runtime-PM reference, so it does not stop the
device from autosuspending. It stops while the device is suspended and
restarts (with a fresh state read) on resume. On resume the driver adopts
the device's state; it does not push its cached values back.

---

## 5. Uninstall / roll back

```sh
sudo apt remove openfuse-dkms     # back to the distribution's driver
sudo apt purge openfuse-dkms      # also deletes caches and configuration
modinfo -n snd-usb-audio          # .../kernel/sound/usb/snd-usb-audio.ko.zst
```

What remove and purge do (all maintainer scripts tolerate missing dkms
or modules and always exit 0, so apt never breaks):

- **prerm:** `dkms remove openfuse/<v> --all` for every version dkms
  knows. Then, for every kernel in `/lib/modules`, whether or not dkms
  was there:
  - delete any OpenFuse `updates/dkms/snd-usb-audio.ko*` (identified by
    its `openfuse` modinfo field);
  - copy back any distribution module dkms archived and did not restore;
  - run `depmod -a <kver>` for every kernel that changed.
- **dpkg** deletes the package files, including
  `/usr/lib/modprobe.d/openfuse-fallback.conf` and the wrapper.
- **postrm remove:** the same sweep again, in case prerm could not run.
  It also deletes `/run/openfuse` and root build output in
  `/usr/src/openfuse-<v>/`.
- **postrm purge:** additionally deletes `/var/cache/openfuse`,
  `/etc/openfuse` and the dkms tree `/var/lib/dkms/openfuse`, after its
  archived originals have been restored.

Tests R1–R3 (§9) check that after purge the module tree is byte-for-byte
identical to the pre-install snapshot, with and without dkms present.

Not removed: the MOK. It is a system key that other DKMS modules may share.
To remove it, run `sudo mokutil --delete /var/lib/shim-signed/mok/MOK.der`,
then reboot and confirm in MokManager. A user-created
`/etc/modprobe.d/openfuse-options.conf` (for `options snd_usb_audio
minifuse_...`) is also kept, but is harmless: the stock module ignores
unknown parameters, and the wrapper drops them in the fallback.

Manual rollback, if the package scripts cannot run at all:

1. `sudo rm /lib/modules/<kver>/updates/dkms/snd-usb-audio.ko*`;
2. if `kernel/sound/usb/snd-usb-audio.ko.zst` is missing, copy it from
   `/var/lib/dkms/openfuse/original_module/<kver>/<arch>/`;
3. `sudo rm /usr/lib/modprobe.d/openfuse-fallback.conf`;
4. `sudo depmod -a <kver>`.

---

## 6. Hardware test plan for the open questions (spec §8 Q1–Q5)

Everything below uses only ALSA controls, ALSA PCM tools and the module
parameters; no raw USB from userspace. Record every result, with its status
marker, in `docs/protocol.md`.

**Safety first.** Unplug condenser microphones you don't want powered, and
ribbon microphones, before toggling 48 V. Turn monitors and headphones
down before the loopback tests, and keep `Loopback To Monitor Switch` off
unless a test needs it (risk of feedback).

Set up a watcher in a second terminal:

```sh
CARD=$(awk '/MiniFuse/ {print $1; exit}' /proc/asound/cards)
watch -n 0.3 "amixer -c $CARD cget name='Device State Raw' | tail -n1"
sudo dmesg -w | grep -i minifuse     # write-mismatch warnings show up here
```

`Device State Raw` holds byte pairs in little-endian order: bytes 0–1 are
state[0x00], bytes 2–3 are state[0x01], and so on.

### Q1: bitfield writes (INST, mutes)

1. Turn both INST switches off with the buttons. state[0x00] should read
   `0x00 0x00`.
2. With the default `minifuse_bitfield_rmw=N`:
   `amixer -c $CARD cset name='Line In 2 Inst Switch' on`.
   - **INST 2 LED on, state[0x00] = 2**: the index is a bit number. Q1 is
     answered and the default is correct.
   - **INST 1 LED on, state[0x00] = 1**: the device took the value as the
     whole bitfield. `dmesg` shows `wrote 1, device reports 0`. Confirm with
     `echo Y | sudo tee /sys/module/snd_usb_audio/parameters/minifuse_bitfield_rmw`
     and repeat steps 1–2; INST 2 must now switch alone.
   - **Nothing changes**: record it; the write format is something else.
3. Turn it off again the same way and check that only bit 1 clears. Then
   turn on INST 1 and INST 2 one after the other, to check that writing one
   bit keeps the other.
4. Repeat with `Loopback Ch 2 Mute` (state[0x16] bit 1) to check that the
   mutes behave the same.
5. Put the INST switches back as they were. If RMW turned out to be right,
   change `MINIFUSE_BITFIELD_RMW_DEFAULT` in `mixer_minifuse.c`.

### Q2: unknown entries state[0x01–0x03], [0x12], [0x17]

Watch `Device State Raw` (bytes 2–7, 36–37 and 46–47) while you do each of
the following, one at a time:

- press every button;
- toggle every loopback switch;
- move every loopback volume;
- change the sample rate (`speaker-test -D hw:$CARD -c 4 -r 44100`, then
  96000 and 192000);
- start and stop capture;
- plug and unplug inputs, headphones and MIDI.

Note which entries move and to what values. For [0x12] (possibly the
loopback output level), check whether it follows any loopback change.
(Spec Q2 also suggests GET_PARAM on these entries. ALSA cannot do that; it
would need a temporary debug control in the driver and is out of scope for
this plan.)

### Q3: loopback level limits

Use `Loopback Ch 1 Left Volume`. After each write, read the control back
and also the raw entry state[0x06] (bytes 12–13 of `Device State Raw`;
the raw state is not clamped):

| Write | Meaning | Check |
|---|---|---|
| `-15360` | −60 dB | stored as is? |
| `-15872` | −62 dB (between the expected minimum and "off") | stored, or rounded to −60 / −63.008? |
| `-16130` | 0xC0FE, the observed "off" | stored as is? |
| `0` | 0 dB | |
| `3072` | +12 dB | stored, or clamped lower? |

`amixer -c $CARD cset name='Loopback Ch 1 Left Volume' -- -15360`, and so
on. Any difference between the written and the stored value is logged in
`dmesg`. The control currently rejects values outside −16130…3072. To probe
beyond +12 dB, raise `MINIFUSE_GAIN_MAX` and rebuild. Afterwards set the
constants `MINIFUSE_GAIN_MIN` and `MINIFUSE_GAIN_MAX` to the confirmed
limits (the TLV follows them automatically). Restore the factory values at
the end: −3.008 dB is `-770`, "off" is `-16130`.

### Q4: loopback channel order

1. `Loopback Switch` on, `Loopback To USB In Switch` on,
   `Loopback To Monitor Switch` off.
2. Set every `Loopback Ch N Left/Right Volume` to the minimum, and set all
   `Loopback Ch N Mute` to off (unmuted).
3. For N = 1..6, set only channel N to `0` (0 dB) on both Left and Right.
   Then, one source at a time:
   - a signal into input 1, then into input 2;
   - a tone on each playback channel:
     `speaker-test -D hw:$CARD -c 4 -t sine -s <ch>` for channels 1–4.

   Record the capture with `arecord -D hw:$CARD -f S32_LE -c 2 -r 48000 /tmp/q4.wav`
   and see which source appears. The `Level Meter` loopback values (3rd and
   4th) give a quick check without recording.
4. The table "channel N ← source" answers Q4. Note whether
   `Loopback To USB In` replaces or adds to the inputs on USB capture 1/2.

### Q5: meter format and order

1. Feed a steady sine of known level into input 1 only. Use a calibrated
   generator, or loop a line output into the input. Set GAIN 1 so that
   `arecord` plus `sox /tmp/in.wav -n stats` reports a peak of about
   −12 dBFS.
2. Sample the meter while recording:
   `for i in $(seq 20); do amixer -c $CARD cget name='Level Meter' | tail -n1; sleep 0.05; done`.
   The first value should follow input 1. Compare `20·log10(v/2^31)` with
   sox's peak dBFS: if they agree, full scale is 2^31.
3. Raise the level until the input clips (sox peak 0 dBFS). The meter value
   at clipping is the true full scale (expected about `0x7FFFFF00` or
   `0x80000000`).
4. Repeat for input 2 (2nd value). For loopback L/R (3rd/4th values), use
   the Q4 routing with a playback tone panned to L only, then R only.
5. Peak reset: play a short burst, then stop. Read the meter twice within a
   few ms (`amixer cget ...; amixer cget ...`). If the second read is
   already back at the noise floor, a read resets the peak. If it falls
   off gradually, the device applies its own decay.

---

## 7. Assumptions in the code (where the spec is silent or unverified)

Each item is a single constant or function in `mixer_minifuse.c`:

- **Bitfield write strategy** (Q1): `minifuse_write_bit()`, with default
  `MINIFUSE_BITFIELD_RMW_DEFAULT = false` (one bit per request). The
  justification is in the function's comment.
- **Multi-channel addressing:** a loopback gain for channel N uses
  wValue = (paramId << 8) | N, with paramId 0x06 (left) or 0x0C (right);
  see `minifuse_wvalue()`. Inferred from the spec's "paramId = first
  entry" rule; not yet observed for N > 0.
- **Gain range:** `MINIFUSE_GAIN_MIN` = −16130 (the observed "off" value,
  shown as mute) and `MINIFUSE_GAIN_MAX` = +12 dB (expected, unverified).
  Values read outside this range are clamped in the control; the raw state
  is not.
- **Meters:** 4-byte little-endian entries exposed raw as u32 (integer64
  controls, so the full 32-bit range fits on every architecture);
  `minifuse_meter_decode()`, `MINIFUSE_METER_BYTES`.
- **1-byte SET_PARAM for paramId 0x0D:** `minifuse_set_param_len()`. Unused
  on the MiniFuse 2.
- **Transfer timeouts:** the driver helper `snd_usb_ctl_msg()` uses the USB
  core defaults (5 s), not the vendor tool's 500 ms / 1000 ms (spec §3).
- **Polling interval:** 500 ms, the upper end of the spec's 250–500 ms
  suggestion.
- **Resume:** adopt the device's state; do not restore the cache.
- **Mute controls** follow the spec's naming (`Loopback Ch N Mute`, value
  1 = muted). ALSA convention prefers `... Switch` with 1 = on; the names
  are all in `minifuse_create_controls()` if upstream review asks for it.
- **`Device State Raw`** is a diagnostic control for the open questions and
  may be dropped from an upstream submission.
- MiniFuse 1 / 4 / 2 OTG are not in `minifuse_models[]` (Q6). Adding one
  means adding a table entry and a `case USB_ID(...)` in `mixer_quirks.c`.

---

## 8. Installer helper interface (`openfuse-setup`)

Path: `/usr/libexec/openfuse/openfuse-setup` (POSIX sh, `set -eu`, fixed
`PATH`, `LC_ALL=C`). It acts only on the running kernel (`uname -r`).
Privileged verbs are run with
`pkexec /usr/libexec/openfuse/openfuse-setup <verb>`.

The polkit action is `org.openfuse.setup`
(`/usr/share/polkit-1/actions/org.openfuse.setup.policy`):

- `org.freedesktop.policykit.exec.path` = the helper;
- `auth_admin_keep` for active sessions, `auth_admin` otherwise;
- message: "Authentication is required to set up the OpenFuse audio
  driver".

**Invocation:** exactly one argument, the verb. Anything else exits with 2.
No other input comes from argv or the environment; the helper ignores the
caller's environment for its own decisions.

| Verb | Root | Does |
|---|---|---|
| `status` | no | prints the state (below); changes nothing |
| `enroll` | yes | reads the password from stdin; makes sure the key exists; queues it with `mokutil --import`; then does what `build` does |
| `build` | yes | (re)builds and installs the module for the running kernel (`dkms remove -k` if already built, then `dkms install -k`), applying the Secure Boot guard |
| `restore-stock` | yes | `dkms remove -k <running kernel>` if built or installed; if no `snd-usb-audio` is loaded, `modprobe snd-usb-audio` (now the stock one) |

**stdin (only `enroll`):** the password twice, newline-terminated:
`"<pw>\n<pw>\n"` (the final newline may be missing).

- 1–256 characters (mokutil's limit), printable ASCII 0x20–0x7E only
  (MokManager reads the password from the firmware keyboard).
- The password is passed to `mokutil --import` only through a pipe from
  the shell builtin `printf`. It never appears in argv or the environment
  of any process.

`enroll` handles the key's current state like this:

| Key state | Action | Output |
|---|---|---|
| `not-enrolled` | imports the key | `enroll=imported` |
| `pending`, and it is the only pending request | `--revoke-import`, then import with the new password | `enroll=re-imported` |
| `pending`, other pending requests exist | no import; the earlier password stays valid | `enroll=already-pending` |
| `enrolled` | nothing to do | `enroll=already-enrolled` |
| Secure Boot not enabled | no import; build only | `enroll=not-needed` |

**stdout:** `key=value` lines. Keys match `[a-z_]+`; values contain no
spaces or newlines, except paths. Diagnostics go to stderr.

`status` prints, in this order:

```
version=0.1.0
kernel=7.3.0-10-generic
secure_boot=enabled|disabled|unknown
signature_enforced=yes|no
cert=/var/lib/shim-signed/mok/MOK.der
key_state=enrolled|pending|not-enrolled|blocked|no-key|unknown
key_modsign_only=yes|no|unknown
headers=yes|no
dkms_state=installed|built|added|not-added|unknown
module_loaded=openfuse|stock|none
fallback_active=yes|no
fallback_reason=<text>            (only if the wrapper fell back since boot)
```

- `key_modsign_only`: whether the certificate's extendedKeyUsage contains
  the module-signing OID 1.3.6.1.4.1.2312.16.1.2.
- `fallback_active=yes`: OpenFuse is installed for the running kernel, but
  the stock module is loaded. `fallback_reason` is free text up to the end
  of the line, from `/run/openfuse/fallback`, written by the wrapper.

`module_loaded=openfuse` means the loaded `snd_usb_audio` has the
`minifuse_poll_ms` parameter.

The other verbs print some of these lines:

| Line | Values | Printed by |
|---|---|---|
| `enroll=` | `imported`, `re-imported`, `already-pending`, `already-enrolled`, `not-needed`, `failed` | `enroll` |
| `cert=` | path | `enroll` (after a successful enroll step) |
| `reboot=` | `required`, `not-required` | `enroll` (after a successful enroll step) |
| `build=` | `installed`, `skipped`, `failed` | `build`, `enroll` |
| `restore=` | `removed`, `nothing-to-do`, `failed` | `restore-stock` |
| `module_loaded=` | as in `status` | `restore-stock` |
| `reason=` | `bad-password`, `no-mokutil`, `no-key`, `key-blocked`, `mokutil-import`, `not-pending-after-import`, `no-headers`, `key-not-enrolled`, `no-sources`, `dkms-add`, `dkms-install` | on failure or skip |
| `log=` | path of the DKMS `make.log` | when the build was skipped or failed in DKMS |

**Exit status:**

| Code | Meaning |
|---|---|
| 0 | success (for `enroll`: key handled and module installed) |
| 1 | internal error (also `restore=failed`) |
| 2 | usage error |
| 3 | skipped: module not installed for the running kernel (`reason=no-headers`, `key-not-enrolled` or `no-sources`) |
| 4 | module build failed (`reason=dkms-add` or `dkms-install`) |
| 5 | enrolment failed; nothing was built |
| 6 | not root (privileged verbs) |
| 7 | dkms or `/usr/src/openfuse-<version>` missing |

`enroll` exits with 3 or 4 when the key was queued (`enroll=imported`) but
the build then did not install anything. The reboot is still needed for
the key; `build` can be retried later.

Typical calls:

```sh
printf '%s\n%s\n' "$PW" "$PW" | pkexec /usr/libexec/openfuse/openfuse-setup enroll
pkexec /usr/libexec/openfuse/openfuse-setup build          # repair / no Secure Boot
pkexec /usr/libexec/openfuse/openfuse-setup restore-stock  # MokManager was skipped
/usr/libexec/openfuse/openfuse-setup status                # no authentication
```

---

## 9. Never breaking USB audio: fallback mechanism and failure-mode matrix

### Load-time fallback

`/usr/lib/modprobe.d/openfuse-fallback.conf` contains a single line:

```
install snd_usb_audio if [ -x /usr/libexec/openfuse/openfuse-modprobe ]; then /usr/libexec/openfuse/openfuse-modprobe $CMDLINE_OPTS; else /sbin/modprobe --ignore-install snd_usb_audio $CMDLINE_OPTS; fi
```

kmod (and udev's builtin, which uses libkmod) runs this command whenever
`snd_usb_audio` is loaded: at boot, on hotplug and by manual `modprobe`.
Ubuntu's own `alsa-base.conf` uses the same mechanism for `snd` and
`snd-pcm`.

`openfuse-modprobe` works like this:

1. `modprobe --ignore-install snd_usb_audio "$@"`, the normal load. With
   the package installed, this resolves to `updates/dkms/`, i.e. OpenFuse.
   On success it exits 0.
2. On failure, if the module modprobe tried is not the distribution's:
   - it finds the distribution's module: first the in-tree
     `kernel/sound/usb/snd-usb-audio.ko{.zst,.xz,.gz,}`, then the copy
     dkms archived. A file with an `openfuse` modinfo field is never
     taken as "stock";
   - it loads that module's dependencies (`modinfo -F depends`, each via
     normal modprobe; normally already loaded by step 1);
   - it runs `insmod` with the options modprobe would have used
     (`--show-depends`: configuration such as Ubuntu's `index=-2` plus
     the command line), minus `minifuse_*` options;
   - it logs `openfuse: driver rejected (<reason>), using the standard
     snd-usb-audio` to `/dev/kmsg` (kernel log and journal), and writes
     `/run/openfuse/fallback`.
   - It exits 0 if a `snd_usb_audio` is loaded at the end.

Design rules for the wrapper:

- no network, no dkms;
- only kmod, coreutils and sed, called by absolute path;
- `set -u` but not `-e`; every non-essential step may fail.

If the wrapper file is missing (for example a stale copy of the
configuration in an initramfs), the configuration line itself falls back
to plain `modprobe --ignore-install`.

Why this mechanism:

- It works for every load path. kmod applies `install` commands for both
  modprobe and udev.
- It needs no extra service or ordering, and does not depend on the
  OpenFuse module's own code.
- It is removed atomically with the package.

The alternative, a second module name, would make two drivers claim the
same devices.

The stock file is guaranteed to be on disk because dkms archives it on
install and `dkms-post-install.sh` puts a copy back (§2).

### Failure-mode matrix

| # | Scenario | Result | Covered by |
|---|---|---|---|
| 1 | OpenFuse module loads | OpenFuse driver; `fallback_active=no` | W1, H2 |
| 2 | Key not enrolled / MokManager skipped (signature rejected under lockdown) | stock driver loaded automatically at every boot and hotplug; log line; `fallback_active=yes` | W2a–e, H1 |
| 3 | Module not signed by a trusted key | stock driver, same | W3 |
| 4 | Unknown symbol / version mismatch | stock driver, same | W9 |
| 5 | Any other modprobe failure | stock driver, same (reason = modprobe's message) | W12 |
| 6 | Stock file is `.ko.xz` / `.ko.gz` / `.ko` instead of `.ko.zst` | found and loaded | W3 |
| 7 | In-tree stock file missing (dkms archived it, post-install copy lost) | dkms's archived copy loaded | W4 |
| 8 | No stock module anywhere | non-zero exit, clear log line; the system was already broken | W5 |
| 9 | Stock module fails as well | non-zero exit, both reasons logged | W6 |
| 10 | OpenFuse not installed for this kernel and the load fails | error passed through unchanged, no second attempt | W7 |
| 11 | Wrapper missing but configuration present | plain `modprobe --ignore-install` | W11b |
| 12 | Kernel upgrade without exact sources | DKMS skip (77): nothing installed, nothing archived; stock module loads through the wrapper; apt not affected | S2, S7, W10; dkms run |
| 13 | Only non-exact sources (default) | skip (77), stock stays | S2, S6, S7; dkms run |
| 14 | Non-exact sources explicitly allowed in `build.conf` | built, marked `FALLBACK` in `SOURCE_INFO` and modinfo | S3, S4 |
| 15 | Non-Ubuntu kernel with an exact upstream tree | built from the exact upstream version | S5 |
| 16 | Exact sources do not compile | skip (77), stock stays | S8 |
| 17 | Secure Boot key neither enrolled nor pending | DKMS skip (77), stock stays | dkms run |
| 18 | dkms install archives the stock file | copy put back by `POST_INSTALL`; never into `updates/` | P1, P2 |
| 19 | `apt remove` with dkms present | our module gone, stock module in place, depmod run, scripts exit 0 | R1a–e |
| 20 | `apt purge` | module tree identical to before installation; cache, config, dkms state and runtime state deleted | R1f–g |
| 21 | Remove/purge after dkms was already removed | stock restored from the dkms archive, our module deleted, depmod run, identical to before | R2a–c |
| 22 | Remove when nothing is installed | exit 0, nothing changed | R3 |
| 23 | Key creation | new keys module-signing-only (EKU, CA:FALSE, digitalSignature, mode 600) | K1a–f |
| 24 | Existing (possibly unrestricted) key | never modified; reported `key_modsign_only=no` | K2a–b, K4b |
| 25 | Half-present key pair | left alone, failure reported | K3 |

`make test` runs all W/P/S/R/H/K cases as a normal user: 54 checks, no
root, no network. They use stub `modprobe`/`insmod`/`modinfo`/`depmod`/
`dkms` and a fake `/lib/modules`, `/sys/module`, `/run` and dkms tree.
The tests substitute the fixed-path blocks of test copies of the scripts;
the shipped scripts honour no environment overrides for system paths.
W11c additionally lets the real kmod parse the modprobe.d file
(`modprobe -C ... --show-depends`, a dry run). "dkms run" means the real
dkms 3.2.2 script, run as a normal user with private trees.

**Not verifiable without root:**

- an actual module load and rejection under lockdown, and `insmod` of the
  stock file by the wrapper on a live system;
- udev invoking the install command at hotplug;
- `dkms install` archiving the original and our `POST_INSTALL`;
- `depmod` on the real tree;
- the maintainer scripts run by dpkg;
- `mokutil --import` and MokManager;
- `pkexec` with the polkit policy;
- `update-secureboot-policy --new-key`.

