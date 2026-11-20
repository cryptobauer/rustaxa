set(HEADERS
    include/app/app.hpp
)

set(SOURCES
    src/app.cpp
)

add_library(app ${SOURCES} ${HEADERS})
if(RUSTAXA_ENABLE)
    target_sources(app PRIVATE
        src/production_consensus_application.cpp
        include/app/production_consensus_application.hpp
        ${PROJECT_SOURCE_DIR}/libraries/core_libs/consensus/src/application/consensus_host_ports.cpp
    )
    target_link_libraries(app PRIVATE Rustaxa::application-host-bridge)
    target_include_directories(app PRIVATE ${PROJECT_SOURCE_DIR}/rust/crates/rustaxa-native-config/include)
endif()
target_include_directories(app PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)
target_link_libraries(app PUBLIC
    common
    core_libs
    rpc_plugin
    light_plugin
)

install(TARGETS app
    ARCHIVE  DESTINATION ${CMAKE_INSTALL_LIBDIR}
    LIBRARY  DESTINATION ${CMAKE_INSTALL_LIBDIR}
)
