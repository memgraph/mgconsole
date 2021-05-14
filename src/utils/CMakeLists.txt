ExternalProject_Add(replxx-proj
        PREFIX            replxx
        GIT_REPOSITORY    https://github.com/AmokHuginnsson/replxx.git
        GIT_TAG           release-0.0.3
        CMAKE_ARGS        "-DCMAKE_INSTALL_PREFIX=<INSTALL_DIR>"
                          "-DCMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}"
        INSTALL_DIR       "${PROJECT_BINARY_DIR}/replxx")

ExternalProject_Get_Property(replxx-proj install_dir)
set(REPLXX_ROOT ${install_dir})
set(REPLXX_INCLUDE_DIRS ${REPLXX_ROOT}/include)
set(REPLXX_LIBRARY_PATH ${REPLXX_ROOT}/lib/libreplxx.a)
set(REPLXX_LIBRARY replxx)

add_library(${REPLXX_LIBRARY} STATIC IMPORTED GLOBAL)
target_compile_definitions(${REPLXX_LIBRARY} INTERFACE REPLXX_STATIC)
set_target_properties(${REPLXX_LIBRARY} PROPERTIES
        IMPORTED_LOCATION ${REPLXX_LIBRARY_PATH})

add_dependencies(${REPLXX_LIBRARY} replxx-proj)
add_library(utils STATIC utils.cpp)
add_dependencies(utils replxx mgclient)
target_compile_definitions(utils PUBLIC MGCLIENT_STATIC_DEFINE)
target_include_directories(utils PUBLIC ${REPLXX_INCLUDE_DIRS} ${MGCLIENT_INCLUDE_DIRS})
target_link_libraries(utils ${REPLXX_LIBRARY})
