#pragma once

#include "windows_rdp_host.h"

enum class CallbackState : uint32_t {
    Open,
    Closing,
    Closed,
};

struct NativeRdpHost {
    uint64_t generation;
    NavopRdpEventCallback callback;
    void* callback_context;
    CallbackState callback_state;
};
