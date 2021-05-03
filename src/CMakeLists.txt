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

find_package(Threads REQUIRED)
find_package(OpenSSL REQUIRED)

# Create compile_commands.json
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

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

ExternalProject_Add(mgclient-proj
        PREFIX            gflags
        GIT_REPOSITORY    https://github.com/memgraph/mgclient.git
        GIT_TAG           master
        CMAKE_ARGS        "-DCMAKE_INSTALL_PREFIX=<INSTALL_DIR>"
                          "-DCMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}"
        INSTALL_DIR       "${PROJECT_BINARY_DIR}/mgclient")

ExternalProject_Get_Property(mgclient-proj install_dir)
set(MGCLIENT_ROOT ${install_dir})

set(MGCLIENT_INCLUDE_DIRS ${MGCLIENT_ROOT}/include
        CACHE INTERNAL "Path to mgclient include directory")

set(MGCLIENT_LIBRARY_PATH ${MGCLIENT_ROOT}/lib/${CMAKE_FIND_LIBRARY_PREFIXES}mgclient.a)
set(MGCLIENT_DEBUG_LIBRARY_PATH ${MGCLIENT_ROOT}/lib/${CMAKE_FIND_LIBRARY_PREFIXES}mgclient_debug.a)
set(MGCLIENT_LIBRARY mgclient)
add_library(${MGCLIENT_LIBRARY} STATIC IMPORTED)
set_target_properties(${MGCLIENT_LIBRARY} PROPERTIES
  IMPORTED_LOCATION ${MGCLIENT_LIBRARY_PATH}
  IMPORTED_LOCATION_DEBUG ${MGCLIENT_DEBUG_LIBRARY_PATH}
        INTERFACE_LINK_LIBRARIES ${CMAKE_THREAD_LIBS_INIT})
      add_dependencies(${MGCLIENT_LIBRARY} gflags-proj)

add_subdirectory(utils)

add_compile_options(-Wall -Wextra -pedantic -Werror)

add_executable(mgconsole main.cpp)
target_include_directories(mgconsole PRIVATE
        ${GFLAGS_INCLUDE_DIRS}
        ${MGCLIENT_INCLUDE_DIRS}
        ${REPLXX_INCLUDE_DIRS}
        ${CMAKE_CURRENT_BINARY_DIR})
target_link_libraries(mgconsole PRIVATE ${MGCLIENT_LIBRARY} ${GFLAGS_LIBRARY} ${OPENSSL_LIBRARIES} utils stdc++fs)

configure_file("${CMAKE_CURRENT_SOURCE_DIR}/version.hpp.in"
        "${CMAKE_CURRENT_BINARY_DIR}/version.hpp")

include(GNUInstallDirs)

install(TARGETS mgconsole
       RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
