# mgconsole - console client for Memgraph database
# Copyright (C) 2016-2021 Memgraph Ltd. [https://memgraph.com]
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

cmake_minimum_required(VERSION 3.4)
project(mgconsole VERSION 1.4)
include(CTest)

set(CMAKE_MODULE_PATH ${CMAKE_MODULE_PATH} ${PROJECT_SOURCE_DIR}/cmake)

set(CMAKE_C_STANDARD 11)
set(CMAKE_CXX_STANDARD 20)

# Set default build type to 'Release'
if (NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE "Release")
endif()
message(STATUS "CMake build type: ${CMAKE_BUILD_TYPE}")
# Config of some libraries (e.g. replxx) depends on the build type.
string(TOLOWER "${CMAKE_BUILD_TYPE}" CMAKE_BUILD_TYPE_LOWERCASE)
# Enable explicit static linking of SSL because of generic Linux binary.
set(MGCONSOLE_STATIC_SSL OFF CACHE STRING "Statically link SSL")

# Set default installation directory to '/usr'
if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
    # '/usr' is a special case, for more details see:
    # https://cmake.org/cmake/help/latest/module/GNUInstallDirs.html#special-cases
    set(CMAKE_INSTALL_PREFIX "/usr" CACHE PATH "install dir" FORCE)
endif()

add_subdirectory(src)
if(BUILD_TESTING)
    add_subdirectory(tests)
endif()
