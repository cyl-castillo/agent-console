# Portable CPU baseline for the native code we ship (whisper.cpp via
# whisper-rs). Passed to every cmake-built crate through the
# CMAKE_TOOLCHAIN_FILE env var in CI; it only pins ggml's instruction-set
# options, so other crates are unaffected.
#
# Without it ggml defaults to GGML_NATIVE=ON (-march=native): the release
# binary inherits the GitHub runner's AVX-512 and dies with SIGILL
# ("trap invalid opcode") the moment Whisper loads on any CPU without it —
# which is most desktops, including the maintainer's i5-7600T. Every Linux
# release since voice shipped (v0.29) had this.
#
# x86-64-v3 (AVX2 + FMA + F16C + BMI2, Haswell 2013 and newer) is the
# baseline every current OS requires or recommends; AVX-512 stays off. On
# arm64 (Apple Silicon) turning native off leaves the compiler's generic
# armv8 target, which every M-series chip runs.
set(GGML_NATIVE OFF CACHE BOOL "portable build: no -march=native" FORCE)
if(CMAKE_SYSTEM_PROCESSOR MATCHES "(x86_64|AMD64|amd64)")
  set(GGML_SSE42 ON CACHE BOOL "" FORCE)
  set(GGML_AVX ON CACHE BOOL "" FORCE)
  set(GGML_AVX2 ON CACHE BOOL "" FORCE)
  set(GGML_FMA ON CACHE BOOL "" FORCE)
  set(GGML_F16C ON CACHE BOOL "" FORCE)
  set(GGML_BMI2 ON CACHE BOOL "" FORCE)
  set(GGML_AVX512 OFF CACHE BOOL "" FORCE)
endif()
