#!/usr/bin/env sh
# Linux: pull the latest commit and run an optimized build.
# Usage (from the repo root): scripts/playtest.sh [game options]
set -e
git pull --ff-only
exec cargo run --profile playtest -- "$@"
