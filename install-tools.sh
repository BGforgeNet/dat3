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

ALL_TOOLS=(actionlint cargo-deny cargo-machete cargo-zigbuild shellcheck shfmt wasm-bindgen wasmtime zig zizmor)

# zig lives as a whole tree; only a symlink to it goes in BIN_DIR
ZIG_DIR="$HOME/.local/share/zig"

# Digests are of the immutable release assets; refresh them when bumping a
# version. Where the vendor publishes its own checksum file the two agree; the
# zizmor, ShellCheck and shfmt releases carry none, so those digests are computed
# from the downloaded asset.
ACTIONLINT_VERSION="1.7.12"
ACTIONLINT_SHA256="8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8"

# Only a gnu target is published; the CI runner and the gate hosts have glibc.
ZIZMOR_VERSION="1.30.0"
ZIZMOR_SHA256="ec8c95cd800845abb9bbc5f377ec7c57d2eb8e2386a00a201d3a74ee4092e5ed"

SHELLCHECK_VERSION="0.11.0"
SHELLCHECK_SHA256="8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198"

# Published as a bare binary rather than an archive
SHFMT_VERSION="3.14.1"
SHFMT_SHA256="76e77641faa025814b77f153b29796b8e6fa2fca03e0c76a691608b86c7ea7bf"

DENY_VERSION="0.20.2"
DENY_SHA256="9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f"

MACHETE_VERSION="0.9.2"
MACHETE_SHA256="48200087f54c55aabcd4db4af1e25742b49846c02a1b1bfa134711945b35b2e9"

ZIGBUILD_VERSION="0.23.4"
ZIGBUILD_SHA256="9e3cf73485edbd45905c8aadbc0fdf869c7ddc3848f0c898229f2680db52e44b"

# 47.0.4 rather than the older 47.0.3: it fixes a sandbox escape (GHSA-vqjp-4c8c-hfgg).
WASMTIME_VERSION="47.0.4"
WASMTIME_SHA256="446e8641ba372333670ba0373d5d3083e5cf0dd001b66088afbb3983db0f768f"

# Must equal the wasm-bindgen version crates/dat3-wasm pins: the glue it
# generates only works with the matching crate.
WASM_BINDGEN_VERSION="0.2.127"
WASM_BINDGEN_SHA256="61d4a7dc85acfa0d2354ccc0b8361928c7e52a746d17f28ebaa795ed3dc1614a"

# Digest as published in ziglang.org's download index
ZIG_VERSION="0.16.0"
ZIG_SHA256="70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00"

# Prints "version|url|sha256|path-of-the-binary-inside-the-archive" for a tool;
# an empty path means the download is the binary itself.
tool_spec() {
	case "$1" in
	actionlint)
		printf '%s|%s|%s|%s' "$ACTIONLINT_VERSION" \
			"https://github.com/rhysd/actionlint/releases/download/v${ACTIONLINT_VERSION}/actionlint_${ACTIONLINT_VERSION}_linux_amd64.tar.gz" \
			"$ACTIONLINT_SHA256" \
			"actionlint"
		;;
	zizmor)
		printf '%s|%s|%s|%s' "$ZIZMOR_VERSION" \
			"https://github.com/zizmorcore/zizmor/releases/download/v${ZIZMOR_VERSION}/zizmor-x86_64-unknown-linux-gnu.tar.gz" \
			"$ZIZMOR_SHA256" \
			"zizmor"
		;;
	shellcheck)
		printf '%s|%s|%s|%s' "$SHELLCHECK_VERSION" \
			"https://github.com/koalaman/shellcheck/releases/download/v${SHELLCHECK_VERSION}/shellcheck-v${SHELLCHECK_VERSION}.linux.x86_64.tar.xz" \
			"$SHELLCHECK_SHA256" \
			"shellcheck-v${SHELLCHECK_VERSION}/shellcheck"
		;;
	shfmt)
		printf '%s|%s|%s|%s' "$SHFMT_VERSION" \
			"https://github.com/mvdan/sh/releases/download/v${SHFMT_VERSION}/shfmt_v${SHFMT_VERSION}_linux_amd64" \
			"$SHFMT_SHA256" \
			""
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
	wasm-bindgen)
		printf '%s|%s|%s|%s' "$WASM_BINDGEN_VERSION" \
			"https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WASM_BINDGEN_VERSION}/wasm-bindgen-${WASM_BINDGEN_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
			"$WASM_BINDGEN_SHA256" \
			"wasm-bindgen-${WASM_BINDGEN_VERSION}-x86_64-unknown-linux-musl/wasm-bindgen"
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
	local cmd="$1" want="$2" version_arg="$3" reported
	command -v "$cmd" >/dev/null || return 1
	# stderr is captured, not discarded: a binary that is present but broken
	# reports its error in the trace and then gets reinstalled.
	reported="$("$cmd" "$version_arg" 2>&1 || true)"
	[[ "$reported" == *"$want"* ]]
}

# Downloads to $3 and checks its digest
fetch_archive() {
	local url="$1" sha256="$2" dest="$3"
	curl -sfL -o "$dest" "$url"
	echo "$sha256  $dest" | sha256sum -c -
}

# Fetches an archive and puts one binary from it in BIN_DIR
install_tool() {
	local name="$1" url="$2" sha256="$3" path_in_archive="$4" tmp
	tmp="$(mktemp -d)"
	fetch_archive "$url" "$sha256" "$tmp/archive"
	if [ -z "$path_in_archive" ]; then
		install -m 0755 "$tmp/archive" "$BIN_DIR/$name"
	else
		# --no-same-owner: extracting as root would otherwise try to restore the
		# archive's uid/gid, which fails outside a full-privileged container.
		tar --no-same-owner -xf "$tmp/archive" -C "$tmp" "$path_in_archive"
		install -m 0755 "$tmp/$path_in_archive" "$BIN_DIR/$name"
	fi
	rm -rf "$tmp"
}

# zig ships a lib/ tree that has to sit beside the binary, so the tree is kept
# under ZIG_DIR and only a symlink goes on PATH; zig resolves the link to find
# lib/. Relinking an already-extracted tree costs nothing, which is what makes
# this safe to rerun.
install_zig() {
	local dir="$ZIG_DIR/$ZIG_VERSION" unpacked="zig-x86_64-linux-${ZIG_VERSION}" tmp
	if [ ! -x "$dir/zig" ]; then
		tmp="$(mktemp -d)"
		fetch_archive "https://ziglang.org/download/${ZIG_VERSION}/${unpacked}.tar.xz" \
			"$ZIG_SHA256" "$tmp/archive"
		tar --no-same-owner -xf "$tmp/archive" -C "$tmp"
		mkdir -p "$ZIG_DIR"
		rm -rf "${ZIG_DIR:?}/$ZIG_VERSION"
		mv "$tmp/$unpacked" "$dir"
		rm -rf "$tmp"
	fi
	ln -sfn "$dir/zig" "$BIN_DIR/zig"
}

ensure_tool() {
	local name="$1" spec version url sha256 path_in_archive
	# zig is the one tool that is a tree rather than a lone binary, and it
	# reports its version through a subcommand instead of a flag.
	if [ "$name" = zig ]; then
		if ! has_version zig "$ZIG_VERSION" version; then
			install_zig
		fi
		return 0
	fi
	# Assigned on its own line: tool_spec runs in a subshell, so its exit status
	# for an unknown name only propagates through the assignment. Inlined into
	# the here-string below it would be lost, and the install would run with an
	# empty URL.
	spec="$(tool_spec "$name")"
	IFS='|' read -r version url sha256 path_in_archive <<<"$spec"
	if has_version "$name" "$version" --version; then
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
