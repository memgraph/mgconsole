#!/bin/bash

# mgconsole - console client for Memgraph database
# Copyright (C) 2016-2019 Memgraph Ltd. [https://memgraph.com]
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
if [ "$1" == "--use-ssl" ]; then
  use_ssl=true
  shift
fi

if [ ! $# -eq 2 ]; then
    echo "Usage: $0 [path to memgraph binary] [path to client binary]"
    exit 1
fi

# Find memgraph binaries.
if [ ! -x $1 ]; then
    echo_failure "memgraph executable not found"
    exit 1
fi

# Find mgconsole binaries.
if [ ! -x $2 ]; then
    echo_failure "mgconsole executable not found"
    exit 1
fi

memgraph_binary=$(realpath $1)
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
$memgraph_binary --port 7687 \
        --cert-file=$cert_file \
        --key-file=$key_file \
        --durability-directory=$tmpdir \
        --db-recover-on-startup=false \
        --durability-enabled=false \
        --properties-on-disk='' \
        --snapshot-on-exit=false \
        --telemetry-enabled=false \
        --log-file='' &

pid=$!
wait_for_server 7687
echo_success "Started memgraph"


## Tests

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
        $client_binary $run_flags < $filename > $tmpdir/$test_name
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

# Shutdown the memgraph process.
kill $pid
wait -n
code_mg=$?

# Remove temporary directory
rm -rf $tmpdir

# Check memgraph exit code.
if [ $code_mg -ne 0 ]; then
    echo_failure "The memgraph process didn't terminate properly!"
    exit $code_mg
fi
echo_success "Test cleanup done"

exit $test_code
