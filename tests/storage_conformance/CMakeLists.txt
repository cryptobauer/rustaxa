add_executable(storage_conformance_runner
    storage_conformance_runner.cpp
)
if(RUSTAXA_ENABLE_STORAGE)
    target_link_libraries(storage_conformance_runner
        PRIVATE
        rustaxa-bridge
        gmp::gmp
        mpfr::mpfr
    )

    if(APPLE)
        target_link_libraries(storage_conformance_runner PRIVATE pthread dl)
    else()
        target_link_libraries(storage_conformance_runner PRIVATE pthread)
    endif()
else()
    target_link_libraries(storage_conformance_runner
        PRIVATE
        test_util
    )
endif()
add_test(storage_conformance_runner ${CMAKE_RUNTIME_OUTPUT_DIRECTORY}/storage_conformance_runner)
