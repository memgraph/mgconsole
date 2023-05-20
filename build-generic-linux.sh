#!/bin/bash

set -Eeuo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$SCRIPT_DIR"

docker_name="mgconsole_build_generic_linux"
if [ ! "$(docker ps -q -f name=$docker_name)" ]; then
    if [ "$(docker ps -aq -f status=exited -f name=$docker_name)" ]; then
        echo "Cleanup of the old exited mgconsole build container..."
        docker rm $docker_name
    fi
    docker run -d --name "$docker_name" centos:7 sleep infinity
fi
echo "The mgconsole build container is active!"

docker_exec () {
  cmd="$1"
  docker exec "$docker_name" bash -c "$cmd"
}

docker_exec "mkdir -p /mgconsole"
docker cp "$PROJECT_ROOT/." "$docker_name:/mgconsole/"
docker_exec "ls /mgconsole"
