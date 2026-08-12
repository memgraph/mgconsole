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

TOOLCHAIN_ROOT="/opt/toolchain-v8"
ARCH="$(uname -m)"
if [[ $ARCH == "x86_64" ]]; then
    DOCKER_IMAGE="memgraph/mgbuild:v8_ubuntu-24.04"
elif [[ $ARCH == "aarch64" ]]; then
    DOCKER_IMAGE="memgraph/mgbuild:v8_ubuntu-24.04-arm"
else
    echo -e "${RED}Unsupported architecture: $ARCH${RESET}"
    exit 1
fi

trap cleanup EXIT ERR

echo -e "${GREEN}Starting build container...${RESET}"
docker run --rm -d --name builder $DOCKER_IMAGE

echo -e "${GREEN}Copying mgconsole source code to build container...${RESET}"
docker cp "$PROJECT_ROOT/." builder:/home/mg/mgconsole
docker exec -u root builder bash -c "chown -R mg:mg /home/mg/mgconsole"

# Build against the toolchain sysroot (glibc 2.31)
# --strip on install: the sysroot gcc build carries debug info (~21MB -> ~8MB).
echo -e "${GREEN}Building mgconsole...${RESET}"
docker exec -u mg builder bash -c "
    source $TOOLCHAIN_ROOT/activate && \
    export CC=$TOOLCHAIN_ROOT/bin/gcc CXX=$TOOLCHAIN_ROOT/bin/g++ && \
    export OPENSSL_ROOT_DIR=$TOOLCHAIN_ROOT/sysroot/usr && \
    cd /home/mg/mgconsole && \
    cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DMGCONSOLE_STATIC_SSL=ON -DCMAKE_INSTALL_PREFIX=/home/mg/mgconsole/build/install . && \
    cmake --build build && \
    cmake --install build --strip"

echo -e "${GREEN}Checking GLIBC requirement of the binary...${RESET}"
docker exec -u mg builder bash -c '
    source '"$TOOLCHAIN_ROOT"'/activate
    versions=$(objdump -T /home/mg/mgconsole/build/install/bin/mgconsole \
               | grep -oE "GLIBC_[0-9]+(\.[0-9]+)+" | sort -uV)
    echo "Referenced GLIBC versions: $(echo $versions)"
    echo "Maximum GLIBC version required: $(echo "$versions" | tail -n 1)"'

echo -e "${GREEN}Saving build...${RESET}"
mkdir -p "$PROJECT_ROOT/build/generic"
docker cp builder:/home/mg/mgconsole/build/install/bin/mgconsole "$PROJECT_ROOT/build/generic/"

echo -e "${GREEN}Build complete!${RESET}"
