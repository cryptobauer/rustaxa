add_subdirectory(consensus)
add_subdirectory(storage)

if(RUSTAXA_APP_OFFLINE_TEST)
    add_executable(rustaxa_offline_app_test production_owner_test.cpp)
    target_compile_definitions(rustaxa_offline_app_test PRIVATE RUSTAXA_ACTUAL_APP_TEST)
    target_link_libraries(rustaxa_offline_app_test PRIVATE app)
    target_include_directories(rustaxa_offline_app_test PRIVATE
        "${PROJECT_SOURCE_DIR}/rust/crates/rustaxa-native-config/include")
endif()
