#!/usr/bin/env bash
set -e

trap "exit" INT TERM
trap "kill 0" EXIT
ECHOSERVER=echo-server-tasks
cargo +stable build --release --example ${ECHOSERVER}
(target/release/examples/${ECHOSERVER} &> autobahn/log.txt) &
sleep 1
podman run -it --rm -v "${PWD}/autobahn/fuzzingclient.json:/fuzzingclient.json:z" -v "${PWD}/autobahn:/reports:z" --network host crossbario/autobahn-testsuite wstest --mode fuzzingclient
cat autobahn/log.txt