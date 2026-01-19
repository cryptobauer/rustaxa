include(GoogleTest)

add_executable(rust_storage_tests
    test_storage.cpp
)

target_link_libraries(rust_storage_tests
    PRIVATE
    Rustaxa::bridge
    GTest::gtest_main
)

if(UNIX AND NOT APPLE)
    target_link_libraries(rust_storage_tests PRIVATE pthread dl)
elseif(APPLE)
    target_link_libraries(rust_storage_tests PRIVATE pthread)
endif()

if(TARAXA_ENABLE_SANITIZERS)
    add_sanitizers(rust_storage_tests)
endif()

gtest_discover_tests(rust_storage_tests)
