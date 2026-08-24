include(GoogleTest)

add_executable(rust_consensus_tests
    test_final_chain.cpp
    test_network_api.cpp
    test_pillar_vote_bundle.cpp
)

target_link_libraries(rust_consensus_tests
    PRIVATE
    Rustaxa::bridge
    core_libs
    vote
    GTest::gtest_main
    gmp::gmp
    mpfr::mpfr
)

if(UNIX AND NOT APPLE)
    target_link_libraries(rust_consensus_tests PRIVATE pthread dl)
elseif(APPLE)
    target_link_libraries(rust_consensus_tests PRIVATE pthread)
endif()

if(TARAXA_ENABLE_SANITIZERS)
    add_sanitizers(rust_consensus_tests)
endif()

gtest_discover_tests(rust_consensus_tests)
