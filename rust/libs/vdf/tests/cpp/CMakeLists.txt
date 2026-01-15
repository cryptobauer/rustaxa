find_package(GTest REQUIRED)
include(GoogleTest)

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
    ${RUSTAXA_VDF_CXXBRIDGE_DIR}
)

if(UNIX AND NOT APPLE)
    target_link_libraries(vdf_tests PRIVATE pthread dl)
elseif(APPLE)
    target_link_libraries(vdf_tests PRIVATE pthread)
endif()

target_compile_features(vdf_tests PRIVATE cxx_std_17)

if(TARAXA_ENABLE_SANITIZERS)
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
