ExternalProject_Add(replxx-proj
        PREFIX            replxx
        GIT_REPOSITORY    https://github.com/AmokHuginnsson/replxx.git
        GIT_TAG           release-0.0.3
        CMAKE_ARGS        "-DCMAKE_INSTALL_PREFIX=<INSTALL_DIR>"
                          "-DCMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}"
        INSTALL_DIR       "${PROJECT_BINARY_DIR}/replxx")

ExternalProject_Get_Property(replxx-proj install_dir)
set(REPLXX_ROOT ${install_dir})
set(REPLXX_INCLUDE_DIRS ${REPLXX_ROOT}/include
        CACHE INTERNAL "Path to replxx include directory")
set(REPLXX_LIBRARY_PATH ${REPLXX_ROOT}/lib/${CMAKE_FIND_LIBRARY_PREFIXES}replxx.a)
set(REPLXX_DEBUG_LIBRARY_PATH ${REPLXX_ROOT}/lib/${CMAKE_FIND_LIBRARY_PREFIXES}replxx_debug.a)
set(REPLXX_LIBRARY replxx)
add_library(${REPLXX_LIBRARY} STATIC IMPORTED GLOBAL)
set_target_properties(${REPLXX_LIBRARY} PROPERTIES
        IMPORTED_LOCATION ${REPLXX_LIBRARY_PATH}
        IMPORTED_LOCATION_DEBUG ${REPLXX_DEBUG_LIBRARY_PATH})
      add_dependencies(${REPLXX_LIBRARY} replxx-proj)

add_library(utils utils.cpp)
target_include_directories(utils PUBLIC ${REPLXX_INCLUDE_DIRS})
target_link_libraries(utils ${REPLXX_LIBRARY})
