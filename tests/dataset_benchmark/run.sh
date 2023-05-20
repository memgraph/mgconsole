#!/bin/bash -e

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

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$DIR"

MGCONSOLE_BINARY="${MGCONSOLE_BINARY:-$DIR/../../build/src/mgconsole}"
MGCONSOLE_SETUP="${MGCONSOLE_SETUP:-STORAGE MODE IN_MEMORY_ANALYTICAL;}"
MGCONSOLE_BATCH_SIZE="${MGCONSOLE_BATCH_SIZE:-1000}"
MGCONSOLE_WORKERS="${MGCONSOLE_WORKERS:-32}"

TIMEFORMAT=%R
DATASETS=(
  "https://download.memgraph.com/datasets/cora-scientific-publications/cora-scientific-publications.cypherl.gz 2708 5278"
  "https://download.memgraph.com/datasets/marvel-cinematic-universe/marvel-cinematic-universe.cypherl.gz 21732 682943"
)

function check_dataset {
  expected_nodes=$1
  expected_edges=$2
  actual_nodes=$(echo "MATCH (n) RETURN count(n);" | $MGCONSOLE_BINARY --output-format=csv | tail -n 1 | tr -d '"')
  actual_edges=$(echo "MATCH (n)-[r]->(m) RETURN count(r);" | $MGCONSOLE_BINARY --output-format=csv | tail -n 1 | tr -d '"')
  if [[ $expected_nodes != $actual_nodes ]]; then
    echo "The number of nodes is wrong, expected: $expected_nodes actual: $actual_nodes"
    exit 1
  fi
  if [[ $expected_edges != $actual_edges ]]; then
    echo "The number of edges is wrong, expected: $expected_edges actual: $actual_edges"
    exit 1
  fi
}

measure_serial_import() {
  dataset_cypherl=$1
  nodes=$1
  edges=$2
  echo "MATCH (n) DETACH DELETE n;" | $MGCONSOLE_BINARY
  import_time=$( { time cat $dataset_cypherl | $MGCONSOLE_BINARY --import-mode="serial"; } 2>&1 )
  echo "$import_time"
}

measure_batched_parallel_import() {
  dataset_cypherl=$1
  nodes=$1
  edges=$2
  echo "MATCH (n) DETACH DELETE n;" | $MGCONSOLE_BINARY
  import_time=$( { time cat $dataset_cypherl | $MGCONSOLE_BINARY --import-mode="batched-parallel" --batch-size=$MGCONSOLE_BATCH_SIZE --workers-number=$MGCONSOLE_WORKERS; } 2>&1 )
  echo "$import_time"
}

echo "$MGCONSOLE_SETUP" | $MGCONSOLE_BINARY
for dataset in "${DATASETS[@]}"; do
  set -- $dataset; dataset_url=$1; nodes=$2; edges=$3
  dataset_gz="$(basename $dataset_url)"
  dataset_cypherl="$(basename $dataset_gz .gz)"
  if [[ ! -f $dataset_cypherl ]]; then
    wget $dataset_url -O $dataset_gz
    gzip -df $dataset_gz
  fi

  echo "$dataset_cypherl serial import..."
  serial_import_time=$(measure_serial_import $dataset_cypherl $nodes $edges)
  check_dataset $nodes $edges
  serial_tx=$(echo "($nodes + $edges)/$serial_import_time" | bc -l)

  echo "$dataset_cypherl parallel import..."
  parallel_import_time=$(measure_batched_parallel_import $dataset_cypherl $nodes $edges)
  check_dataset $nodes $edges
  parallel_tx=$(echo "($nodes + $edges)/$parallel_import_time" | bc -l)

  echo "dataset | nodes | edges | serial (nodes+edges)/s | parallel (nodes+edges)/s | batch size | workers number"
  echo "$dataset_cypherl | $nodes | $edges | $serial_tx | $parallel_tx | $MGCONSOLE_BATCH_SIZE | $MGCONSOLE_WORKERS"
done
