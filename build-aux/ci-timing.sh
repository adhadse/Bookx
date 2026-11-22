#!/usr/bin/env bash
# Prints what the CI machine is (cores, memory, CPU, build parallelism) at the start of a heavy job, so the numbers
# of a slow run can be read against the hardware. The time of each step is in the job's step list (the PR report's
# "Where the time goes" table reads it from the GitHub API); nothing here measures by itself.
echo "cpus:   $(nproc) (cargo builds with $(nproc) jobs unless CARGO_BUILD_JOBS says otherwise: '${CARGO_BUILD_JOBS:-unset}')"
free -m | awk 'NR==2 {printf "memory: %s MB total, %s MB available\n", $2, $7}'
lscpu 2>/dev/null | sed -n 's/^Model name:\s*/cpu:    /p' | head -1
echo "linker: ${RUSTFLAGS:-default}"
echo "debug info: dev=${CARGO_PROFILE_DEV_DEBUG:-default} test=${CARGO_PROFILE_TEST_DEBUG:-default}"
df -h . 2>/dev/null | awk 'NR==2 {print "disk:   " $4 " free of " $2}'
