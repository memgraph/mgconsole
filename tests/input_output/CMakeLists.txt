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

set(MEMGRAPH_PATH "/usr/lib/memgraph/memgraph" CACHE FILEPATH
        "Path to Memgraph binary for client tests")

add_test(NAME mgconsole-test
        COMMAND ./run-tests.sh ${MEMGRAPH_PATH} ${PROJECT_BINARY_DIR}/src/mgconsole
        WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR})

add_test(NAME mgconsole-secure-test
        COMMAND ./run-tests.sh --use-ssl ${MEMGRAPH_PATH} ${PROJECT_BINARY_DIR}/src/mgconsole
        WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR})
