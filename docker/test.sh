#!/usr/bin/env bash
# Build the Alfred test image and run a smoke test against a pristine container.
#
#   IMAGE=alfred-test ./docker/test.sh
set -euo pipefail

IMAGE="${IMAGE:-alfred-test}"
cd "$(dirname "$0")/.."

echo "==> Building ${IMAGE}"
docker build -t "${IMAGE}" .

echo "==> Health check"
docker run --rm "${IMAGE}" curl -s localhost:3000/health
echo

echo "==> Server info"
docker run --rm "${IMAGE}" curl -s localhost:3000/api/info
echo

echo "OK"
