get_filename_component(REPLXX_PREFIX "${CMAKE_CURRENT_BINARY_DIR}/replxx" ABSOLUTE)

if(CMAKE_BUILD_TYPE_LOWERCASE STREQUAL "debug")
  set(REPLXX_LIB_POSTFIX "-d")
elseif(CMAKE_BUILD_TYPE_LOWERCASE STREQUAL "relwithdebinfo")
  set(REPLXX_LIB_POSTFIX "-rd")
else()
  set(REPLXX_LIB_POSTFIX "")
endif()

get_filename_component(REPLXX_LIB_PATH "${REPLXX_PREFIX}/${MG_INSTALL_LIB_DIR}/libreplxx${REPLXX_LIB_POSTFIX}.a" ABSOLUTE)

# Local fixes applied on top of the pinned replxx tag.
find_package(Git REQUIRED)
ExternalProject_Add(replxx-proj
        PREFIX ${REPLXX_PREFIX}
        GIT_REPOSITORY https://github.com/AmokHuginnsson/replxx.git
        GIT_TAG release-0.0.4
        # Force an LF checkout so the LF patch below applies. Windows/MSYS2 git
        # defaults to core.autocrlf=true, which rewrites the sources with CRLF
        # endings and makes the patch fail with "patch does not apply" (the
        # trailing-CR context lines no longer match). --config persists into the
        # cloned repo, so the update-step checkout stays LF too.
        GIT_CONFIG core.autocrlf=false
        # --3way makes this idempotent: the patch step chains off the git
        # update step and can re-run, and --3way no-ops cleanly when the fix is
        # already present (plain `git apply` would error "patch does not apply").
        PATCH_COMMAND ${GIT_EXECUTABLE} apply --3way
        "${CMAKE_CURRENT_SOURCE_DIR}/replxx-patches/0001-history_previous-bounds-check.patch"
        CMAKE_ARGS "-DCMAKE_INSTALL_PREFIX=<INSTALL_DIR>"
        "-DCMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}"
        "-DCMAKE_CXX_COMPILER=${CMAKE_CXX_COMPILER}"
        "-DCMAKE_C_COMPILER=${CMAKE_C_COMPILER}"
        "-DREPLXX_BUILD_EXAMPLES=OFF"
        "-DBUILD_SHARED_LIBS=OFF"
        INSTALL_DIR ${REPLXX_PREFIX}
        BUILD_BYPRODUCTS "${REPLXX_LIB_PATH}")

set(REPLXX_ROOT ${REPLXX_PREFIX})
set(REPLXX_INCLUDE_DIRS ${REPLXX_ROOT}/include)
set(REPLXX_LIBRARY_PATH ${REPLXX_LIB_PATH})
set(REPLXX_LIBRARY replxx)

add_library(${REPLXX_LIBRARY} STATIC IMPORTED GLOBAL)
target_compile_definitions(${REPLXX_LIBRARY} INTERFACE REPLXX_STATIC)
set_target_properties(${REPLXX_LIBRARY} PROPERTIES
        IMPORTED_LOCATION "${REPLXX_LIBRARY_PATH}")

add_dependencies(${REPLXX_LIBRARY} replxx-proj)
add_library(utils STATIC utils.cpp thread_pool.cpp bolt.cpp)
add_dependencies(utils replxx gflags mgclient)
target_compile_definitions(utils PUBLIC MGCLIENT_STATIC_DEFINE)
target_include_directories(utils PUBLIC ${REPLXX_INCLUDE_DIRS} ${GFLAGS_INCLUDE_DIRS} ${MGCLIENT_INCLUDE_DIRS})
target_link_libraries(utils ${REPLXX_LIBRARY})
