# shellcheck shell=sh
# SPDX-License-Identifier: GPL-2.0
#
# openfuse-mok.sh - Secure Boot / MOK helpers shared by build-module.sh
# (DKMS PRE_BUILD guard), the package postinst and openfuse-setup.
# POSIX sh; sourced, not executed. All names start with of_ / OF_.
#
# The signing key and certificate are chosen the way dkms (3.x) chooses
# them, so that the key checked or enrolled here is the one dkms signs
# with:
#   1. mok_signing_key / mok_certificate from /etc/dkms/framework.conf
#      and /etc/dkms/framework.conf.d/*.conf (last definition wins),
#   2. else the same variables from the environment,
#   3. else, if no key is configured: Ubuntu: the shim-signed MOK
#      (/var/lib/shim-signed/mok/MOK.priv + MOK.der); other distributions:
#      /var/lib/dkms/mok.key (+ /var/lib/dkms/mok.pub if no certificate
#      is configured).

OF_UBUNTU_MOK_KEY=/var/lib/shim-signed/mok/MOK.priv
OF_UBUNTU_MOK_CERT=/var/lib/shim-signed/mok/MOK.der

of_distro_id() {
	(
		[ -r /etc/os-release ] || exit 0
		. /etc/os-release
		echo "${ID:-}"
	)
}

# Last value of a dkms framework variable; $1 is a fixed name, never
# user input.
of_framework_var() {
	_of_val=""
	for _of_f in /etc/dkms/framework.conf /etc/dkms/framework.conf.d/*.conf; do
		[ -r "$_of_f" ] || continue
		_of_x="$(sed -n "s/^[[:space:]]*$1=[\"']\{0,1\}\([^\"']*\)[\"']\{0,1\}[[:space:]]*\$/\1/p" "$_of_f" | tail -n 1)"
		[ -n "$_of_x" ] && _of_val="$_of_x"
	done
	echo "$_of_val"
}

# Sets OF_MOK_KEY and OF_MOK_CERT.
of_mok_paths() {
	OF_MOK_KEY="$(of_framework_var mok_signing_key)"
	OF_MOK_CERT="$(of_framework_var mok_certificate)"
	[ -n "$OF_MOK_KEY" ] || OF_MOK_KEY="${mok_signing_key:-}"
	[ -n "$OF_MOK_CERT" ] || OF_MOK_CERT="${mok_certificate:-}"
	if [ -z "$OF_MOK_KEY" ]; then
		case "$(of_distro_id)" in
		ubuntu*)
			OF_MOK_KEY="$OF_UBUNTU_MOK_KEY"
			OF_MOK_CERT="$OF_UBUNTU_MOK_CERT"
			;;
		*)
			OF_MOK_KEY=/var/lib/dkms/mok.key
			;;
		esac
	fi
	[ -n "$OF_MOK_CERT" ] || OF_MOK_CERT=/var/lib/dkms/mok.pub
}

# enabled | disabled | unknown
of_secure_boot() {
	if ! command -v mokutil >/dev/null 2>&1; then
		echo unknown
		return 0
	fi
	case "$(mokutil --sb-state 2>/dev/null || true)" in
	*"SecureBoot enabled"*) echo enabled ;;
	*"SecureBoot disabled"*) echo disabled ;;
	*) echo unknown ;;
	esac
}

# Succeeds if modules must be signed by a trusted key to load.
of_sig_enforced() {
	[ "$(of_secure_boot)" = enabled ] && return 0
	[ -r /sys/kernel/security/lockdown ] &&
		! grep -q '\[none\]' /sys/kernel/security/lockdown
}

# Enrolment state of certificate $1:
#   enrolled      in MokListRT (or db): modules signed with it load
#   pending       queued with mokutil --import; enrolled at the next boot
#                 if the user confirms it in MokManager
#   not-enrolled  neither
#   blocked       in dbx or MokListX
#   no-key        certificate file missing
#   unknown       mokutil missing or unexpected answer
of_key_state() {
	if [ ! -f "$1" ]; then
		echo no-key
		return 0
	fi
	if ! command -v mokutil >/dev/null 2>&1; then
		echo unknown
		return 0
	fi
	case "$(mokutil --test-key "$1" 2>&1 || true)" in
	*"is already enrolled"*|*"is already in db"*) echo enrolled ;;
	*"is already in the enrollment request"*) echo pending ;;
	*"is blocked"*) echo blocked ;;
	*"is not enrolled"*) echo not-enrolled ;;
	*) echo unknown ;;
	esac
}

# Create a key + certificate restricted to kernel module signing:
# extendedKeyUsage codeSigning + 1.3.6.1.4.1.2312.16.1.2 (module signing
# only; shim refuses such keys for boot loaders and kernels), CA:FALSE,
# keyUsage digitalSignature. Same extensions as Ubuntu's
# /usr/lib/shim/mok/openssl.cnf. Never overwrites existing files.
of_create_modsign_key() {	# key cert
	[ -e "$1" ] || [ -e "$2" ] && return 1
	command -v openssl >/dev/null 2>&1 || return 1
	mkdir -p "$(dirname "$1")" "$(dirname "$2")" || return 1
	_of_cnf="$(mktemp "$(dirname "$1")/.openfuse-modsign.XXXXXX")" || return 1
	cat > "$_of_cnf" <<'OFCNF'
[ req ]
distinguished_name = req_dn
x509_extensions = v3_modsign
string_mask = utf8only
prompt = no

[ req_dn ]
CN = OpenFuse DKMS module signing key

[ v3_modsign ]
basicConstraints = critical,CA:FALSE
keyUsage = digitalSignature
extendedKeyUsage = codeSigning,1.3.6.1.4.1.2312.16.1.2
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always,issuer
nsComment = "OpenFuse generated module signing key"
OFCNF
	(
		umask 077
		openssl req -config "$_of_cnf" -new -x509 -noenc -days 36500 \
			-newkey rsa:2048 -keyout "$1" -outform DER -out "$2" >/dev/null 2>&1 ||
		openssl req -config "$_of_cnf" -new -x509 -nodes -days 36500 \
			-newkey rsa:2048 -keyout "$1" -outform DER -out "$2" >/dev/null 2>&1
	)
	_of_rc=$?
	rm -f "$_of_cnf"
	if [ $_of_rc -ne 0 ] || [ ! -s "$1" ] || [ ! -s "$2" ]; then
		# do not leave a half-created pair behind
		[ -s "$2" ] || rm -f "$1" "$2"
		return 1
	fi
	chmod 600 "$1" 2>/dev/null
	chmod 644 "$2" 2>/dev/null
	return 0
}

# Is certificate $1 restricted to module signing? yes | no | unknown
of_key_modsign_only() {
	[ -f "$1" ] || { echo unknown; return 0; }
	command -v openssl >/dev/null 2>&1 || { echo unknown; return 0; }
	_of_eku="$(openssl x509 -in "$1" -inform DER -noout -ext extendedKeyUsage 2>/dev/null)" ||
		_of_eku="$(openssl x509 -in "$1" -inform PEM -noout -ext extendedKeyUsage 2>/dev/null)" ||
		{ echo unknown; return 0; }
	case "$_of_eku" in
	*1.3.6.1.4.1.2312.16.1.2*) echo yes ;;
	*) echo no ;;
	esac
}

# Make sure the key dkms signs with exists, without enrolling anything.
# Existing keys are never replaced or modified (other DKMS modules may
# depend on them); a half-present pair is left alone and reported as a
# failure. Must run as root.
#   Ubuntu default path: update-secureboot-policy --new-key (shim-signed;
#     its openssl.cnf applies the same module-signing-only restriction)
#   otherwise (or if that tool is missing): of_create_modsign_key
of_ensure_key() {
	of_mok_paths
	[ -f "$OF_MOK_KEY" ] && [ -f "$OF_MOK_CERT" ] && return 0
	[ -e "$OF_MOK_KEY" ] || [ -e "$OF_MOK_CERT" ] && return 1

	if [ "$OF_MOK_CERT" = "$OF_UBUNTU_MOK_CERT" ] &&
	   [ "$OF_MOK_KEY" = "$OF_UBUNTU_MOK_KEY" ] &&
	   command -v update-secureboot-policy >/dev/null 2>&1; then
		SHIM_NOTRIGGER=y update-secureboot-policy --new-key >/dev/null 2>&1 || true
		[ -f "$OF_MOK_KEY" ] && [ -f "$OF_MOK_CERT" ] && return 0
		[ -e "$OF_MOK_KEY" ] || [ -e "$OF_MOK_CERT" ] && return 1
	fi
	of_create_modsign_key "$OF_MOK_KEY" "$OF_MOK_CERT"
}
