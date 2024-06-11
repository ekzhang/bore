#!/usr/bin/env bash
# Script for building your rust projects.
set -e

source ci/common.bash

# $1 {path} = Path to cross/cargo executable
CROSS=$1
# $1 {string} = <Target Triple>
TARGET_TRIPLE=$2

required_arg $CROSS 'CROSS'
required_arg $TARGET_TRIPLE '<Target Triple>'

max_attempts=3
count=0

while [ $count -lt $max_attempts ]; do
    $CROSS test --target $TARGET_TRIPLE
    status=$?
    if [ $status -eq 0 ]; then
        echo "Test passed"
        break
    else
        echo "Test failed, attempt $(($count + 1))"
    fi
    count=$(($count + 1))
done

if [ $status -ne 0 ]; then
    echo "Test failed after $max_attempts attempts"
fi
