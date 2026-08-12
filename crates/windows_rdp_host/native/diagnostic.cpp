#include "host_internal.h"

#include <cinttypes>
#include <cstdio>

namespace {

constexpr const char* kTracePrefix = "RDP_NATIVE_TRACE";

void flush_trace() noexcept {
    std::fflush(stderr);
}

}  // namespace

void trace_native_stage(const char* stage) noexcept {
    std::fprintf(
        stderr,
        "%s stage=%s\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage);
    flush_trace();
}

void trace_native_hresult(
    const char* stage,
    int32_t hresult) noexcept {
    std::fprintf(
        stderr,
        "%s stage=%s hresult=0x%08" PRIX32 "\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        static_cast<uint32_t>(hresult));
    flush_trace();
}

void trace_native_result(
    const char* stage,
    NavopRdpResult result) noexcept {
    std::fprintf(
        stderr,
        "%s stage=%s result=%" PRId32 "\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        result);
    flush_trace();
}

void trace_native_win32(
    const char* stage,
    uint32_t win32_code) noexcept {
    std::fprintf(
        stderr,
        "%s stage=%s win32=0x%08" PRIX32 "\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        win32_code);
    flush_trace();
}

void trace_native_pointer(
    const char* stage,
    uintptr_t pointer) noexcept {
    std::fprintf(
        stderr,
        "%s stage=%s pointer=0x%" PRIXPTR "\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        pointer);
    flush_trace();
}
