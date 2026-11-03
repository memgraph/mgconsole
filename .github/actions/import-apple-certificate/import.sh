#!/usr/bin/env bash
# Imports a Developer ID certificate into a throwaway keychain and finds its
# signing identity. Run by action.yml, which passes its inputs as:
#   P12           the certificate and its private key, as a base64-encoded .p12
#   P12_PASSWORD  the .p12's password
#   KIND          "Developer ID Application" or "Developer ID Installer"
#   IDENTITY      optional name of the identity to require
# and reads the keychain and identity back from $GITHUB_OUTPUT.
set -euo pipefail

if [ -z "$P12" ] || [ -z "$P12_PASSWORD" ]; then
  echo "The $KIND certificate and its password must be set as repository secrets."
  exit 1
fi
keychain="$RUNNER_TEMP/apple-signing.keychain-db"
keychain_password="$(openssl rand -base64 24)"
printf %s "$P12" | base64 --decode > "$RUNNER_TEMP/cert.p12"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 3600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
import_exit=0
security import "$RUNNER_TEMP/cert.p12" -k "$keychain" -P "$P12_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/productbuild -T /usr/bin/productsign || import_exit=$?
rm -f "$RUNNER_TEMP/cert.p12"
if [ "$import_exit" -ne 0 ]; then exit "$import_exit"; fi
# Lets Apple's signing tools use the key without a permission prompt.
security set-key-partition-list -S apple-tool:,apple: -k "$keychain_password" "$keychain" > /dev/null
# Unquoted on purpose: each existing keychain must be its own argument.
# shellcheck disable=SC2046
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
echo "keychain=$keychain" >> "$GITHUB_OUTPUT"

# "basic" rather than "codesigning", which leaves out installer identities.
identities="$(security find-identity -v -p basic "$keychain")"
echo "$identities"
# The names of the identities of this kind, one per line.
names="$(printf '%s\n' "$identities" | sed -n "s/^.*\"\($KIND: .*\)\"\$/\1/p")"
if [ -n "$IDENTITY" ]; then
  case "$IDENTITY" in
    "$KIND: "*) wanted="$IDENTITY" ;;
    *) wanted="$KIND: $IDENTITY" ;;
  esac
  # A prefix match, as the identity may be given without its "(<team id>)".
  names="$(printf '%s\n' "$names" | awk -v w="$wanted" 'index($0, w) == 1')"
fi
count="$(printf '%s' "$names" | grep -c . || true)"
if [ "$count" -ne 1 ]; then
  echo "Expected one \"$KIND\" identity${IDENTITY:+ matching \"$IDENTITY\"} in the certificate, found $count; see the identities above."
  exit 1
fi
echo "Signing as \"$names\""
echo "identity=$names" >> "$GITHUB_OUTPUT"
