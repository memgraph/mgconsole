#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$SCRIPT_DIR"

RED='\033[1;31m'
GREEN='\033[1;32m'
YELLOW='\033[1;33m'
RESET='\033[0m'

function cleanup() {
    status=$?
    if [ $status -ne 0 ]; then
        COLOUR=$RED
    else
        COLOUR=$YELLOW
    fi
    echo -e "${COLOUR}Cleaning up...${RESET}"
    docker stop builder || true
    exit $status
}

ARCH="$(arch)"
if [[ $ARCH == "x86_64" ]]; then
    DOCKER_IMAGE="memgraph/mgbuild:v7_centos-9"  # libc 2.34
elif [[ $ARCH == "aarch64" ]]; then
    DOCKER_IMAGE="memgraph/mgbuild:v7_debian-12-arm"  # libc 2.36
fi

trap cleanup EXIT ERR

echo -e "${GREEN}Starting build container...${RESET}"
docker run --rm -d --name builder $DOCKER_IMAGE

echo -e "${GREEN}Copying mgconsole source code to build container...${RESET}"
docker cp "$PROJECT_ROOT/." builder:/home/mg/mgconsole
docker exec -u root builder bash -c "chown -R mg:mg /home/mg/mgconsole"

echo -e "${GREEN}Building mgconsole...${RESET}"
docker exec -u mg builder bash -c "
    source /opt/toolchain-v7/activate && \
    cd /home/mg/mgconsole && \
    cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DMGCONSOLE_STATIC_SSL=ON -DCMAKE_INSTALL_PREFIX=/home/mg/mgconsole/build/install . && \
    cmake --build build && \
    cmake --install build"

echo -e "${GREEN}Saving build...${RESET}"
mkdir -p "$PROJECT_ROOT/build/generic"
docker cp builder:/home/mg/mgconsole/build/install/bin/mgconsole "$PROJECT_ROOT/build/generic/"

echo -e "${GREEN}Build complete!${RESET}"
