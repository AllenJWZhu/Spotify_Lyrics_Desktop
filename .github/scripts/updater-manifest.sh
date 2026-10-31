#!/usr/bin/env bash
# Writes the latest.json the in-app updater reads (Tauri's static format).
#
#   updater-manifest.sh VERSION PLATFORM URL SIGNATURE_FILE NOTES_FILE
#
# PLATFORM is the updater target, e.g. windows-x86_64.
set -euo pipefail
version=$1 platform=$2 url=$3 sig_file=$4 notes_file=$5

jq -n \
  --arg version "$version" \
  --arg platform "$platform" \
  --arg url "$url" \
  --rawfile signature "$sig_file" \
  --rawfile notes "$notes_file" \
  --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{
    version: $version,
    notes: $notes,
    pub_date: $pub_date,
    platforms: { ($platform): { signature: ($signature | rtrimstr("\n")), url: $url } }
  }'
