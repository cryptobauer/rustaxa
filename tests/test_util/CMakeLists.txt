set(HEADERS
  include/test_util/node_dag_creation_fixture.hpp
  include/test_util/samples.hpp
  include/test_util/test_util.hpp
)

set(SOURCES
    src/samples.cpp
    src/test_util.cpp
)

# The manager-shaped DAG construction fixture is pure-C++ reference coverage.
# Rust-mode fixtures exercise the native root through tests/rust/consensus.
if(NOT RUSTAXA_ENABLE)
  list(APPEND SOURCES src/node_dag_creation_fixture.cpp)
endif()

# Configure file with version
add_library(test_util ${SOURCES} ${HEADERS})
target_include_directories(test_util PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)
target_link_libraries(test_util PUBLIC app core_libs gtest::gtest)

install(TARGETS common
    ARCHIVE  DESTINATION ${CMAKE_INSTALL_LIBDIR}
    LIBRARY  DESTINATION ${CMAKE_INSTALL_LIBDIR}
)
