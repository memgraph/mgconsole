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

# Environment setup
DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$DIR"

mgconsole_binary="$DIR/../../build/src/mgconsole"
mgconsole_exec_before="STORAGE MODE IN_MEMORY_ANALYTICAL;"

DATASETS=(
  "https://download.memgraph.com/datasets/cora-scientific-publications/cora-scientific-publications.cypherl.gz"
  "https://download.memgraph.com/datasets/marvel-cinematic-universe/marvel-cinematic-universe.cypherl.gz"
)

function check_mg_status {
  echo "MATCH (n) RETURN count(n);" | $mgconsole_binary
  echo "MATCH (n)-[r]->(m) RETURN count(r);" | $mgconsole_binary
}

function measure_parsing {
  dataset_cypherl="$1"
  time cat $dataset_cypherl | $mgconsole_binary --import-mode="parser"
}

function measure_serial_import {
  echo "MATCH (n) DETACH DELETE n;" | $mgconsole_binary
  echo "$dataset_cypherl SERIAL TIME"
  time cat $dataset_cypherl | $mgconsole_binary --import-mode="serial"
  check_mg_status
}

function measure_batched_parallel_import {
  echo "MATCH (n) DETACH DELETE n;" | $mgconsole_binary
  echo "$dataset_cypherl BATCH-PARALLEL TIME"
  time cat $dataset_cypherl | $mgconsole_binary --import-mode="batched-parallel"
  check_mg_status
}

echo "$mgconsole_exec_before" | $mgconsole_binary
for dataset_url in "${DATASETS[@]}"; do
  dataset_gz="$(basename $dataset_url)"
  dataset_cypherl="$(basename $dataset_gz .gz)"
  wget $dataset_url -O $dataset_gz
  gzip -df $dataset_gz
  measure_parsing $dataset_cypherl
done
