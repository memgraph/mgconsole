#!/usr/bin/env bash
# Submits a .zip, .pkg or .dmg to Apple's notary service, waits for the
# verdict, and fails with Apple's log if it is not accepted. Run by
# action.yml, which passes its inputs as:
#   FILE               path of the file to notarize
#   APPLE_ID           the Apple ID that notarizes
#   APPLE_ID_PASSWORD  an app-specific password for APPLE_ID
#   APPLE_TEAM_ID      the team id
set -uo pipefail
if [ -z "$APPLE_ID" ] || [ -z "$APPLE_ID_PASSWORD" ] || [ -z "$APPLE_TEAM_ID" ]; then
  echo "APPLE_ID, APPLE_ID_PASSWORD and APPLE_TEAM_ID must be set as repository secrets."
  exit 1
fi
result="$RUNNER_TEMP/notary-$(basename "$FILE").json"
submit_exit=0
xcrun notarytool submit "$FILE" --wait --timeout 30m \
  --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_ID_PASSWORD" \
  --output-format json > "$result" || submit_exit=$?
cat "$result"
# notarytool may fail before writing any JSON, or write an error
# message instead, so a missing status or id is not an error here.
status="$(jq -r '.status // empty' "$result" 2>/dev/null || true)"
if [ "$submit_exit" -ne 0 ] || [ "$status" != "Accepted" ]; then
  echo "Notarization of $FILE failed (notarytool exit $submit_exit, status ${status:-unknown})."
  id="$(jq -r '.id // empty' "$result" 2>/dev/null || true)"
  if [ -n "$id" ]; then
    xcrun notarytool log "$id" \
      --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_ID_PASSWORD"
  fi
  exit 1
fi
