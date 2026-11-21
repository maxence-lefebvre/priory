#!/bin/sh
# Runs in a scratch checkout of the result's commit, with PRIORY_SCORE_BASE, PRIORY_SCORE_HEAD
# and PRIORY_SCORE_RESULT set. Exit 0 passes, 1 fails, anything else is an error.
cargo test --quiet || exit 1
