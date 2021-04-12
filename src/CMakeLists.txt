# mgconsole - console client for Memgraph database
# Copyright (C) 2016-2020 Memgraph Ltd. [https://memgraph.com]
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

include(ExternalProject)

if(APPLE)
  set(CMAKE_THREAD_LIBS_INIT "-lpthread")
  set(CMAKE_HAVE_THREADS_LIBRARY 1)
  set(CMAKE_USE_WIN32_THREADS_INIT 0)
  set(CMAKE_USE_PTHREADS_INIT 1)
  set(THREADS_PREFER_PTHREAD_FLAG ON)
endif()
find_package(Threads REQUIRED)
find_package(OpenSSL REQUIRED)
find_package(Readline REQUIRED)

# Setup GFlags
ExternalProject_Add(gflags-proj
        PREFIX            gflags
        GIT_REPOSITORY    https://github.com/gflags/gflags.git
        GIT_TAG           v2.2.2
        CMAKE_ARGS        "-DCMAKE_INSTALL_PREFIX=<INSTALL_DIR>"
                          "-DCMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}"
        INSTALL_DIR       "${PROJECT_BINARY_DIR}/gflags")

ExternalProject_Get_Property(gflags-proj install_dir)
set(GFLAGS_ROOT ${install_dir})

set(GFLAGS_INCLUDE_DIRS ${GFLAGS_ROOT}/include
        CACHE INTERNAL "Path to gflags include directory")

set(GFLAGS_LIBRARY_PATH ${GFLAGS_ROOT}/lib/${CMAKE_FIND_LIBRARY_PREFIXES}gflags.a)
set(GFLAGS_DEBUG_LIBRARY_PATH ${GFLAGS_ROOT}/lib/${CMAKE_FIND_LIBRARY_PREFIXES}gflags_debug.a)
set(GFLAGS_LIBRARY gflags)
add_library(${GFLAGS_LIBRARY} STATIC IMPORTED)
set_target_properties(${GFLAGS_LIBRARY} PROPERTIES
        IMPORTED_LOCATION ${GFLAGS_LIBRARY_PATH}
        IMPORTED_LOCATION_DEBUG ${GFLAGS_DEBUG_LIBRARY_PATH}
        INTERFACE_LINK_LIBRARIES ${CMAKE_THREAD_LIBS_INIT})
add_dependencies(${GFLAGS_LIBRARY} gflags-proj)

# Setup mgclient library
find_library(MGCLIENT_LIBRARY mgclient)
find_path(MGCLIENT_INCLUDE_DIRS mgclient.h)

if (MGCLIENT_LIBRARY AND MGCLIENT_INCLUDE_DIRS)
  message(STATUS "Found mgclient: " ${MGCLIENT_LIBRARY} " "
          ${MGCLIENT_INCLUDE_DIRS})
else()
  message(FATAL_ERROR "Could not find mgclient")
endif()

add_compile_options(-Wall -Wextra -pedantic -Werror)

add_executable(mgconsole main.cpp)
target_include_directories(mgconsole PRIVATE
        ${GFLAGS_INCLUDE_DIRS}
        ${MGCLIENT_INCLUDE_DIRS}
        ${CMAKE_CURRENT_BINARY_DIR})
target_link_libraries(mgconsole ${MGCLIENT_LIBRARY} ${GFLAGS_LIBRARY}
        ${READLINE_LIBRARY} stdc++fs)

configure_file("${CMAKE_CURRENT_SOURCE_DIR}/version.hpp.in"
        "${CMAKE_CURRENT_BINARY_DIR}/version.hpp")

include(GNUInstallDirs)

install(TARGETS mgconsole
       RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
