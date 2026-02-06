#!/bin/bash
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$SCRIPT_DIR/.."

set -euo pipefail

# Parse flags
use_ssl=false
use_docker=false
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

client_binary=${1:-"$PROJECT_ROOT/build/generic/mgconsole"}
memgraph_image=${2:-"memgraph/memgraph:latest"}

echo "Testing mgconsole static build"

script_args=()
if $use_ssl; then
    script_args+=("--use-ssl")
fi
if $use_docker; then
    script_args+=("--docker")
fi
script_args+=("$memgraph_image" "$client_binary")
echo "Running tests with arguments: ${script_args[@]}"

cd "$SCRIPT_DIR/input_output"
./run-tests.sh "${script_args[@]}"
echo "Tests passed"
