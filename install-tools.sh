#!/bin/bash

set -xeu -o pipefail

# Installs the pinned tools that are not cargo dependencies, so Cargo.lock
# cannot hold them. Straight from each vendor's release tarball rather than via
# cargo binstall: binstall resolves these through GitHub artifact probes that
# time out on CI runners, and then falls back to building from source without
# failing - 7m53s for wasmtime. binstall itself had no prebuilt path here at all
# and was compiled on every cache miss, 3m24s.
#
# Usage: ./install-tools.sh [tool...]   (default: every tool below)

BIN_DIR="$HOME/.cargo/bin"

ALL_TOOLS=(cargo-audit cargo-deny cargo-machete cargo-zigbuild wasmtime)

# Digests are of the immutable release assets; refresh them when bumping a
# version. Each one the vendor also publishes as a .sha256 matches it.
AUDIT_VERSION="0.22.2"
AUDIT_SHA256="7fb9497f8594b389e5fce5ef9b92db08432996895b2e0c5a0167a69ed445c428"

DENY_VERSION="0.20.2"
DENY_SHA256="9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f"

MACHETE_VERSION="0.9.2"
MACHETE_SHA256="48200087f54c55aabcd4db4af1e25742b49846c02a1b1bfa134711945b35b2e9"

ZIGBUILD_VERSION="0.23.0"
ZIGBUILD_SHA256="f0aa9cc8220a84788c6e4a9b6d80422f041659227b680fdef982d5a8ddffddb4"

# 47.0.4 rather than the older 47.0.3: it fixes a sandbox escape (GHSA-vqjp-4c8c-hfgg).
WASMTIME_VERSION="47.0.4"
WASMTIME_SHA256="446e8641ba372333670ba0373d5d3083e5cf0dd001b66088afbb3983db0f768f"

# Prints "version|url|sha256|path-of-the-binary-inside-the-archive" for a tool
tool_spec() {
	case "$1" in
	cargo-audit)
		# The release tag contains a slash, hence the %2F
		printf '%s|%s|%s|%s' "$AUDIT_VERSION" \
			"https://github.com/rustsec/rustsec/releases/download/cargo-audit%2Fv${AUDIT_VERSION}/cargo-audit-x86_64-unknown-linux-musl-v${AUDIT_VERSION}.tgz" \
			"$AUDIT_SHA256" \
			"cargo-audit-x86_64-unknown-linux-musl-v${AUDIT_VERSION}/cargo-audit"
		;;
	cargo-deny)
		printf '%s|%s|%s|%s' "$DENY_VERSION" \
			"https://github.com/EmbarkStudios/cargo-deny/releases/download/${DENY_VERSION}/cargo-deny-${DENY_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
			"$DENY_SHA256" \
			"cargo-deny-${DENY_VERSION}-x86_64-unknown-linux-musl/cargo-deny"
		;;
	cargo-machete)
		printf '%s|%s|%s|%s' "$MACHETE_VERSION" \
			"https://github.com/bnjbvr/cargo-machete/releases/download/v${MACHETE_VERSION}/cargo-machete-v${MACHETE_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
			"$MACHETE_SHA256" \
			"cargo-machete-v${MACHETE_VERSION}-x86_64-unknown-linux-musl/cargo-machete"
		;;
	cargo-zigbuild)
		printf '%s|%s|%s|%s' "$ZIGBUILD_VERSION" \
			"https://github.com/rust-cross/cargo-zigbuild/releases/download/v${ZIGBUILD_VERSION}/cargo-zigbuild-x86_64-unknown-linux-musl.tar.xz" \
			"$ZIGBUILD_SHA256" \
			"cargo-zigbuild-x86_64-unknown-linux-musl/cargo-zigbuild"
		;;
	wasmtime)
		printf '%s|%s|%s|%s' "$WASMTIME_VERSION" \
			"https://github.com/bytecodealliance/wasmtime/releases/download/v${WASMTIME_VERSION}/wasmtime-v${WASMTIME_VERSION}-x86_64-linux.tar.xz" \
			"$WASMTIME_SHA256" \
			"wasmtime-v${WASMTIME_VERSION}-x86_64-linux/wasmtime"
		;;
	*)
		echo "Error: no such tool: $1 (known: ${ALL_TOOLS[*]})" >&2
		exit 1
		;;
	esac
}

# True when the pinned version is already on PATH, so a restored cache is reused
# and a stale one is replaced.
has_version() {
	local cmd="$1" want="$2" reported
	command -v "$cmd" >/dev/null || return 1
	# stderr is captured, not discarded: a binary that is present but broken
	# reports its error in the trace and then gets reinstalled.
	reported="$("$cmd" --version 2>&1 || true)"
	[[ "$reported" == *"$want"* ]]
}

# Fetches an archive, verifies its digest, and puts one binary in BIN_DIR
install_tool() {
	local name="$1" url="$2" sha256="$3" path_in_archive="$4" tmp
	tmp="$(mktemp -d)"
	curl -sfL -o "$tmp/archive" "$url"
	echo "$sha256  $tmp/archive" | sha256sum -c -
	# --no-same-owner: extracting as root would otherwise try to restore the
	# archive's uid/gid, which fails outside a full-privileged container.
	tar --no-same-owner -xf "$tmp/archive" -C "$tmp" "$path_in_archive"
	install -m 0755 "$tmp/$path_in_archive" "$BIN_DIR/$name"
	rm -rf "$tmp"
}

ensure_tool() {
	local name="$1" spec version url sha256 path_in_archive
	# Assigned on its own line: tool_spec runs in a subshell, so its exit status
	# for an unknown name only propagates through the assignment. Inlined into
	# the here-string below it would be lost, and the install would run with an
	# empty URL.
	spec="$(tool_spec "$name")"
	IFS='|' read -r version url sha256 path_in_archive <<<"$spec"
	if has_version "$name" "$version"; then
		return 0
	fi
	install_tool "$name" "$url" "$sha256" "$path_in_archive"
}

tools=("$@")
if [ ${#tools[@]} -eq 0 ]; then
	tools=("${ALL_TOOLS[@]}")
fi

mkdir -p "$BIN_DIR"
for tool in "${tools[@]}"; do
	ensure_tool "$tool"
done
