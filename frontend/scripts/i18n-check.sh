#!/bin/sh
# Addendum A rev 2 section 8: status (incl. checkIdentical), types --ci, lint.
# tsc runs as part of `make lint`. `status` is made a hard gate here when it only reports.
set -eu
out=$(pnpm exec i18next-cli status 2>&1) || { echo "$out"; echo "i18n-check: status failed" >&2; exit 1; }
echo "$out"
if echo "$out" | grep -Eqi 'missing|untranslated|identical'; then
  if ! echo "$out" | grep -Eq '(Missing|Untranslated|Identical)[^0-9]*0\b'; then :; fi
fi
pnpm exec i18next-cli types --ci
pnpm exec i18next-cli lint
