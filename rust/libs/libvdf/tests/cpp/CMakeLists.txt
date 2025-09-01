add_executable(vdf_tests
    test_vdf.cpp
    test_vdf_integration.cpp
)

target_link_libraries(vdf_tests
    PRIVATE
    rustaxa-vdf
    GTest::gtest_main
)

target_include_directories(vdf_tests PRIVATE
    ${CMAKE_CURRENT_BINARY_DIR}/../../cargo_build/cxxbridge
)

if(UNIX AND NOT APPLE)
    target_link_libraries(vdf_tests PRIVATE pthread dl)
elseif(APPLE)
    target_link_libraries(vdf_tests PRIVATE pthread)
endif()

target_compile_features(vdf_tests PRIVATE cxx_std_17)

if(ENABLE_SANITIZER)
    target_compile_options(vdf_tests PRIVATE
        -fsanitize=address,undefined
        -fno-omit-frame-pointer
        -g
    )
    target_link_options(vdf_tests PRIVATE
        -fsanitize=address,undefined
    )
endif()

add_dependencies(vdf_tests rustaxa-vdf)

gtest_discover_tests(vdf_tests)
