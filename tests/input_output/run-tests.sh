#!/bin/bash

# mgconsole - console client for Memgraph database
# Copyright (C) 2016-2023 Memgraph Ltd. [https://memgraph.com]
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

## Helper functions

function wait_for_server {
    port=$1
    while ! nc -z -w 1 127.0.0.1 $port; do
        sleep 0.1
    done
    sleep 1
}

function echo_info { printf "\033[1;36m~~ $1 ~~\033[0m\n"; }
function echo_success { printf "\033[1;32m~~ $1 ~~\033[0m\n\n"; }
function echo_failure { printf "\033[1;31m~~ $1 ~~\033[0m\n\n"; }

use_ssl=false
use_docker=false
memgraph_image="memgraph/memgraph"
cert_dir=""
container_name=""

# Parse flags
while [[ $# -gt 0 ]]; do
    case $1 in
        --use-ssl)
            use_ssl=true
            shift
            ;;
        --docker)
            use_docker=true
            shift
            ;;
        *)
            break
            ;;
    esac
done

if [ ! $# -eq 2 ]; then
    echo "Usage: $0 [--use-ssl] [--docker] [path to memgraph binary or docker image] [path to client binary]"
    exit 1
fi

if [ "$use_docker" = true ]; then
    memgraph_image=$1
    # Check if docker is available
    if ! command -v docker &> /dev/null; then
        echo_failure "docker command not found"
        exit 1
    fi
else
    # Find memgraph binaries.
    if [ ! -x $1 ]; then
        echo_failure "memgraph executable not found"
        exit 1
    fi
    memgraph_binary=$(realpath $1)
fi

# Find mgconsole binaries.
if [ ! -x $2 ]; then
    echo_failure "mgconsole executable not found"
    exit 1
fi

client_binary=$(realpath $2)

## Environment setup
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$DIR"

# Create a temporary directory for output files
tmpdir=/tmp/mgconsole/output
if [ -d $tmpdir ]; then
    rm -rf $tmpdir
fi
mkdir -p $tmpdir
cd $tmpdir

# Check and generate SSL certificates
key_file=""
cert_file=""
if $use_ssl; then
    key_file=".key.pem"
    cert_file=".cert.pem"
    openssl req -new -newkey rsa:4096 -days 365 -nodes -x509 \
        -subj "/C=GB/ST=London/L=London/O=Memgraph/CN=db.memgraph.com" \
        -keyout $key_file -out $cert_file || exit 1
fi

## Startup

# Start the memgraph process and wait for it to start.
echo_info "Starting memgraph"
if [ "$use_docker" = true ]; then
    container_name="mgconsole_test_$$"
    # Remove container if it already exists (from a previous failed run)
    docker rm -f $container_name > /dev/null 2>&1
    # Also check for any containers using port 7687
    existing_container=$(docker ps -a --filter "publish=7687" --format "{{.Names}}" | grep -v "^$" | head -1)
    if [ -n "$existing_container" ]; then
        echo_info "Error: Existing container using port 7687: $existing_container"
        exit 1
    fi
    
    docker_args=" -p 7687:7687 -p 7444:7444"    
    if $use_ssl; then
        docker create --name $container_name $docker_args \
            $memgraph_image \
            --bolt-port 7687 \
            --bolt-cert-file=/etc/memgraph/ssl/cert.pem \
            --bolt-key-file=/etc/memgraph/ssl/key.pem \
            --storage-properties-on-edges=true \
            --storage-snapshot-interval-sec=0 \
            --storage-wal-enabled=false \
            --data-recovery-on-startup=false \
            --storage-snapshot-on-exit=false \
            --telemetry-enabled=false \
            --timezone=UTC \
            --log-file='' || exit 1
        echo_info "Copying certificates to container"
        # Create the SSL directory and copy certificates before starting
        docker cp $cert_file $container_name:/tmp/cert.pem
        docker cp $key_file $container_name:/tmp/key.pem
        docker start $container_name
        # Wait a moment for container to be ready
        sleep 1
        # Create directory and copy certificates as root, then fix ownership
        docker exec -u root $container_name sh -c "mkdir -p /etc/memgraph/ssl/ && cp /tmp/cert.pem /etc/memgraph/ssl/cert.pem && cp /tmp/key.pem /etc/memgraph/ssl/key.pem && chmod 644 /etc/memgraph/ssl/cert.pem && chmod 600 /etc/memgraph/ssl/key.pem && chown -R memgraph:memgraph /etc/memgraph/ssl/ || true"
        docker exec $container_name ls -la /etc/memgraph/ssl/ 2>&1 || true
        # Restart container to pick up the certificates
        docker restart $container_name
        sleep 2
    else
        docker run -d --name $container_name $docker_args \
            $memgraph_image \
            --bolt-port 7687 \
            --storage-properties-on-edges=true \
            --storage-snapshot-interval-sec=0 \
            --storage-wal-enabled=false \
            --data-recovery-on-startup=false \
            --storage-snapshot-on-exit=false \
            --telemetry-enabled=false \
            --timezone=UTC \
            --log-file='' || exit 1
    fi
   
    wait_for_server 7687
    echo_success "Started memgraph in docker container"
else
    $memgraph_binary --bolt-port 7687 \
            --bolt-cert-file=$cert_file \
            --bolt-key-file=$key_file \
            --data-directory=$tmpdir \
            --storage-properties-on-edges=true \
            --storage-snapshot-interval-sec=0 \
            --storage-wal-enabled=false \
            --data-recovery-on-startup=false \
            --storage-snapshot-on-exit=false \
            --telemetry-enabled=false \
            --timezone=UTC \
            --log-file='' &

    pid=$!
    wait_for_server 7687
    echo_success "Started memgraph"
fi


## Tests

client_flags="--use-ssl=$use_ssl"

echo_info "Prepare database"
echo  # Blank line

$client_binary $client_flags < ${DIR}/prepare.cypher > $tmpdir/prepare.log
echo_info "Running tests"
echo  # Blank line

client_flags="--use-ssl=$use_ssl"
test_code=0
for output_dir in ${DIR}/output_*; do
    for filename in ${DIR}/input/*; do
        test_name=$(basename $filename)
        test_name=${test_name%.*}
        output_name="$test_name.txt"

        output_format=$(basename $output_dir)
        output_format=${output_format#*_}
        run_flags="$client_flags --output-format=$output_format"

        echo_info "Running test '$test_name' with $output_format output"
        $client_binary $run_flags < $filename > $tmpdir/$test_name 2>&1
        test_exit_code=$?
        if [ $test_exit_code -ne 0 ]; then
            echo_failure "Test '$test_name' with $output_format output failed with exit code $test_exit_code"
            echo "Output:"
            cat $tmpdir/$test_name
        fi
        diff -b $tmpdir/$test_name $output_dir/$output_name
        test_code=$?
        if [ $test_code -ne 0 ]; then
            echo_failure "Test '$test_name' with $output_format output failed"
            break
        else
            echo_success "Test '$test_name' with $output_format output passed"
        fi

        # Clear database for each test.
        $client_binary $client_flags <<< "MATCH (n) DETACH DELETE n;" \
                                     &> /dev/null || exit 1
    done
    if [ $test_code -ne 0 ]; then
        break
    fi
done


## Cleanup
echo_info "Starting test cleanup"

# Shutdown the memgraph process or container.
if [ "$use_docker" = true ]; then
    docker stop $container_name > /dev/null 2>&1
    code_mg=$?
    docker rm $container_name > /dev/null 2>&1
    if [ -n "$cert_dir" ]; then
        rm -rf $cert_dir
    fi
else
    kill $pid
    wait -n
    code_mg=$?
fi

# Remove temporary directory
rm -rf $tmpdir

# Check memgraph exit code.
if [ $code_mg -ne 0 ]; then
    if [ "$use_docker" = true ]; then
        echo_failure "The memgraph container didn't terminate properly!"
    else
        echo_failure "The memgraph process didn't terminate properly!"
    fi
    exit $code_mg
fi
echo_success "Test cleanup done"

exit $test_code
