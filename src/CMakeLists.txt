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
include(GNUInstallDirs)

# Create compile_commands.json
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

if(WIN32)
  set(MGCONSOLE_ON_WINDOWS TRUE)
elseif(APPLE)
  set(MGCONSOLE_ON_OSX TRUE)
elseif(UNIX)
  set(MGCONSOLE_ON_LINUX TRUE)
endif()

find_package(Threads REQUIRED)
set(CMAKE_THREAD_PREFER_PTHREAD TRUE)
set(THREADS_PREFER_PTHREAD_FLAG TRUE)

if(NOT MGCONSOLE_ON_LINUX OR MGCONSOLE_STATIC_SSL)
  set(OPENSSL_USE_STATIC_LIBS TRUE)
endif()
find_package(OpenSSL REQUIRED)
if(MGCONSOLE_ON_OSX)
  set(MACOSX_OPENSSL_ROOTDIR_FLAG "-DOPENSSL_ROOT_DIR=${OPENSSL_ROOT_DIR}")
endif()

# Handle platforms
if(MGCONSOLE_ON_WINDOWS)
  set(GFLAGS_WIN_LIB_SUFFIX "_static")
  set(CMAKE_CXX_STANDARD_LIBRARIES "-static-libgcc -static-libstdc++ -lws2_32")
  set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} -Wl,-Bstatic,--whole-archive -lwinpthread -Wl,--no-whole-archive")
elseif(MGCONSOLE_ON_LINUX)
  set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} -static-libgcc -static-libstdc++")
endif()

if(MGCONSOLE_ON_LINUX)
  # On Debian, the libdir has multiarch path which we don't want while searching for
  # dependancy libs
  string(REGEX MATCH "lib[^/]*" MG_INSTALL_LIB_DIR ${CMAKE_INSTALL_LIBDIR})
else()
  set(MG_INSTALL_LIB_DIR "lib")
endif()

# Setup GFlags
ExternalProject_Add(gflags-proj
  PREFIX gflags
  GIT_REPOSITORY https://github.com/gflags/gflags.git
  GIT_TAG v2.2.2
  CMAKE_ARGS "-DCMAKE_INSTALL_PREFIX=<INSTALL_DIR>"
  "-DCMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}"
  "-DCMAKE_CXX_COMPILER=${CMAKE_CXX_COMPILER}"
  "-DCMAKE_C_COMPILER=${CMAKE_C_COMPILER}"
  INSTALL_DIR "${PROJECT_BINARY_DIR}/gflags")

ExternalProject_Get_Property(gflags-proj install_dir)
set(GFLAGS_ROOT ${install_dir})
set(GFLAGS_INCLUDE_DIRS ${GFLAGS_ROOT}/include)
set(GFLAGS_LIBRARY_PATH ${GFLAGS_ROOT}/lib/libgflags${GFLAGS_WIN_LIB_SUFFIX}.a)
set(GFLAGS_DEBUG_LIBRARY_PATH ${GFLAGS_ROOT}/lib/libgflags${GFLAGS_WIN_LIB_SUFFIX}_debug.a)
set(GFLAGS_LIBRARY gflags)

add_library(${GFLAGS_LIBRARY} STATIC IMPORTED)
target_compile_definitions(${GFLAGS_LIBRARY} INTERFACE GFLAGS_IS_A_DLL=0)
set_target_properties(${GFLAGS_LIBRARY} PROPERTIES
  IMPORTED_LOCATION ${GFLAGS_LIBRARY_PATH}
  IMPORTED_LOCATION_DEBUG ${GFLAGS_DEBUG_LIBRARY_PATH}
  INTERFACE_LINK_LIBRARIES Threads::Threads)
add_dependencies(${GFLAGS_LIBRARY} gflags-proj)

ExternalProject_Add(mgclient-proj
  PREFIX mgclient
  GIT_REPOSITORY https://github.com/memgraph/mgclient.git
  GIT_TAG v1.4.1
  CMAKE_ARGS "-DCMAKE_INSTALL_PREFIX=<INSTALL_DIR>"
  "-DCMAKE_CXX_COMPILER=${CMAKE_CXX_COMPILER}"
  "-DCMAKE_C_COMPILER=${CMAKE_C_COMPILER}"
  "-DCMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}"
  ${MACOSX_OPENSSL_ROOTDIR_FLAG}
  INSTALL_DIR "${PROJECT_BINARY_DIR}/mgclient")

ExternalProject_Get_Property(mgclient-proj install_dir)
set(MGCLIENT_ROOT ${install_dir})
set(MGCLIENT_INCLUDE_DIRS ${MGCLIENT_ROOT}/include)
set(MGCLIENT_LIBRARY_PATH ${MGCLIENT_ROOT}/${MG_INSTALL_LIB_DIR}/libmgclient.a)
set(MGCLIENT_LIBRARY mgclient)

add_library(${MGCLIENT_LIBRARY} STATIC IMPORTED)
set_target_properties(${MGCLIENT_LIBRARY} PROPERTIES
  IMPORTED_LOCATION ${MGCLIENT_LIBRARY_PATH}
  INTERFACE_LINK_LIBRARIES Threads::Threads)
add_dependencies(${MGCLIENT_LIBRARY} mgclient-proj)

add_subdirectory(utils)

add_compile_options(-Wall -Wextra -pedantic -Werror)

# replxx performs narrowing conversion on Windows
if(MGCONSOLE_ON_WINDOWS)
  add_compile_options(-Wno-narrowing)
endif()

add_executable(mgconsole main.cpp interactive.cpp serial_import.cpp batch_import.cpp parsing.cpp)
target_compile_definitions(mgconsole PRIVATE MGCLIENT_STATIC_DEFINE)
target_include_directories(mgconsole
  PRIVATE
  ${GFLAGS_INCLUDE_DIRS}
  ${MGCLIENT_INCLUDE_DIRS}
  ${REPLXX_INCLUDE_DIRS}
  ${CMAKE_CURRENT_BINARY_DIR})

target_link_libraries(mgconsole
  PRIVATE
  ${GFLAGS_LIBRARY}
  utils
  ${MGCLIENT_LIBRARY}
  ${OPENSSL_LIBRARIES})

if(MGCONSOLE_ON_WINDOWS)
  target_link_libraries(mgconsole PRIVATE shlwapi)
endif()

configure_file("${CMAKE_CURRENT_SOURCE_DIR}/version.hpp.in"
  "${CMAKE_CURRENT_BINARY_DIR}/version.hpp")

install(TARGETS mgconsole
  RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
