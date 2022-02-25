#!/usr/bin/env bash
set -e

trap "exit" INT TERM
trap "kill 0" EXIT
cargo +stable build --example echo-server-tasks
cargo +stable run --example echo-server-tasks &
sleep 1
podman run -it --rm -v "${PWD}/autobahn/fuzzingclient.json:/fuzzingclient.json:z" -v "${PWD}/autobahn:/reports:z" --network host crossbario/autobahn-testsuite wstest -d --mode fuzzingclient
