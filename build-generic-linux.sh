#!/bin/bash
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$SCRIPT_DIR"

docker_name="mgconsole_build_generic_linux"
toolchain_url="https://s3-eu-west-1.amazonaws.com/deps.memgraph.io/toolchain-v4/toolchain-v4-binaries-centos-7-x86_64.tar.gz"
toolchain_tar_gz="$(basename $toolchain_url)"
memgraph_repo="https://github.com/memgraph/memgraph.git"
setup_toolchain_cmd="cd /memgraph/environment/os && \
  ./centos-7.sh check TOOLCHAIN_RUN_DEPS || \
  ./centos-7.sh install TOOLCHAIN_RUN_DEPS"
setup_memgraph_cmd="cd /memgraph/environment/os && \
  ./centos-7.sh check MEMGRAPH_BUILD_DEPS || \
  ./centos-7.sh install MEMGRAPH_BUILD_DEPS"
mgconsole_build_cmd="source /opt/toolchain-v4/activate && \
  mkdir -p /mgconsole/build && cd /mgconsole/build && \
  cmake -DCMAKE_BUILD_TYPE=Release .. && make -j"

if [ ! "$(docker info)" ]; then
  echo "ERROR: Docker is required"
  exit 1
fi

if [ ! "$(docker ps -q -f name=$docker_name)" ]; then
    if [ "$(docker ps -aq -f status=exited -f name=$docker_name)" ]; then
        echo "Cleanup of the old exited mgconsole build container..."
        docker rm $docker_name
    fi
    docker run -d --network host --name "$docker_name" centos:7 sleep infinity
fi
echo "The mgconsole build container is active!"

docker_exec () {
  cmd="$1"
  docker exec -it "$docker_name" bash -c "$cmd"
}

docker_exec "mkdir -p /mgconsole"
docker cp -q "$PROJECT_ROOT/." "$docker_name:/mgconsole/"
docker_exec "rm -rf /mgconsole/build/*"
docker_exec "yum install -y wget git"
docker_exec "[ ! -f /$toolchain_tar_gz ] && wget -O /$toolchain_tar_gz $toolchain_url"
docker_exec "[ ! -d /opt/toolchain-v4/ ] && tar -xzf /$toolchain_tar_gz -C /opt"
docker_exec "[ ! -d /memgraph/ ] && git clone $memgraph_repo"
docker_exec "$setup_toolchain_cmd"
docker_exec "$setup_memgraph_cmd"
docker_exec "$mgconsole_build_cmd"
mkdir -p "$PROJECT_ROOT/build/generic"
docker cp -q "$docker_name:/mgconsole/build/src/mgconsole" "$PROJECT_ROOT/build/generic/"
