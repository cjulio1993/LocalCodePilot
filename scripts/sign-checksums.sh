#!/usr/bin/env bash
# Signs a checksums file with Ed25519 and checks the result against a public key.
#
#   UPDATE_SIGNING_KEY="<private key PEM>" scripts/sign-checksums.sh <SHA256SUMS.txt> <public.pem>
#
# Writes <SHA256SUMS.txt>.sig: the raw 64-byte signature the app verifies with the
# key embedded in it (desktop/src/updates.rs). Fails without the private key, or when
# it does not match the public key, so a release is never published unsigned or
# signed with the wrong key. The private key only touches a temporary file that is
# removed on exit.
set -euo pipefail

checksums="${1:?usage: sign-checksums.sh <SHA256SUMS.txt> <public.pem>}"
public_key="${2:?usage: sign-checksums.sh <SHA256SUMS.txt> <public.pem>}"

if [[ -z "${UPDATE_SIGNING_KEY:-}" ]]; then
  echo "::error::UPDATE_SIGNING_KEY is not set: the release cannot be signed." >&2
  exit 1
fi

umask 077
key_file="$(mktemp)"
trap 'rm -f "$key_file"' EXIT
printf '%s\n' "$UPDATE_SIGNING_KEY" > "$key_file"

openssl pkeyutl -sign -inkey "$key_file" -rawin -in "$checksums" -out "$checksums.sig"
if ! openssl pkeyutl -verify -pubin -inkey "$public_key" -rawin -in "$checksums" -sigfile "$checksums.sig" >/dev/null; then
  rm -f "$checksums.sig"
  echo "::error::The signature does not match $public_key: wrong UPDATE_SIGNING_KEY." >&2
  exit 1
fi
echo "Signed $checksums with the key of $public_key."
