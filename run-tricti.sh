#!/usr/bin/env bash
export LLVM_SYS_181_PREFIX=/nix/store/0l2qyps0nlhdpl5hxzrxbr3lkq7irkmk-llvm-18.1.8-dev
export SKIP_STDLIB=1
./target/debug/tricti "$@"
