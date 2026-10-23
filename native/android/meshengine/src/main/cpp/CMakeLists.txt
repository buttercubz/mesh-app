# Oboe shim of :meshengine (DESIGN §10.2.1, §10.2.2): Oboe streams -> mesh_rt_capture /
# mesh_rt_render, resolved with dlsym from libmesh_ffi.so (tone fallback when absent).
cmake_minimum_required(VERSION 3.22.1)
project(meshaudio LANGUAGES CXX)

find_package(oboe REQUIRED CONFIG)

add_library(meshaudio SHARED meshaudio.cpp rt_symbols.cpp)
target_include_directories(meshaudio PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})
target_compile_options(meshaudio PRIVATE -Wall -Wextra -Werror -O2)
# 16 KB page alignment (Android 15+ devices, Play requirement).
target_link_options(meshaudio PRIVATE "-Wl,-z,max-page-size=16384")
target_link_libraries(meshaudio PRIVATE oboe::oboe log dl)
