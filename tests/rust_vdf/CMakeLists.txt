include(GoogleTest)

add_executable(rust_vdf_tests
    test_vdf.cpp
    test_vdf_integration.cpp
)

target_link_libraries(rust_vdf_tests
    PRIVATE
    Rustaxa::bridge
    GTest::gtest_main
)

if(UNIX AND NOT APPLE)
    target_link_libraries(rust_vdf_tests PRIVATE pthread dl)
elseif(APPLE)
    target_link_libraries(rust_vdf_tests PRIVATE pthread)
endif()

if(TARAXA_ENABLE_SANITIZERS)
    target_compile_options(rust_vdf_tests PRIVATE
        -fsanitize=address,undefined
        -fno-omit-frame-pointer
        -g
    )
    target_link_options(rust_vdf_tests PRIVATE
        -fsanitize=address,undefined
    )
endif()

gtest_discover_tests(rust_vdf_tests)
