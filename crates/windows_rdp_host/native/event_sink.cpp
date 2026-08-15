#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>
#include <ocidl.h>

#pragma warning(push)
#pragma warning(disable : 4471)
#include "mstscax.tlh"
#pragma warning(pop)

#include <array>
#include <memory>
#include <new>

namespace {

// DISPIDs and signatures are defined by the IMsTscAxEvents type library:
// https://learn.microsoft.com/windows/win32/termserv/imstscaxevents-interface
constexpr DISPID kOnConnecting = 1;
constexpr DISPID kOnConnected = 2;
constexpr DISPID kOnLoginComplete = 3;
constexpr DISPID kOnDisconnected = 4;
constexpr DISPID kOnEnterFullScreenMode = 5;
constexpr DISPID kOnLeaveFullScreenMode = 6;
constexpr DISPID kOnFatalError = 10;
constexpr DISPID kOnWarning = 11;
constexpr DISPID kOnRemoteDesktopSizeChange = 12;
constexpr DISPID kOnConfirmClose = 15;
constexpr DISPID kOnAutoReconnecting = 17;
constexpr DISPID kOnAuthenticationWarningDisplayed = 18;
constexpr DISPID kOnAuthenticationWarningDismissed = 19;
constexpr DISPID kOnLogonError = 22;
constexpr DISPID kOnFocusReleased = 23;
constexpr DISPID kOnNetworkStatusChanged = 32;
constexpr DISPID kOnAutoReconnected = 33;
constexpr DISPID kOnAutoReconnecting2 = 34;

constexpr LONG kAutoReconnectContinueAutomatic = 0;
constexpr uint32_t kMaxTestArguments = 8;

void encode_u32_le(uint32_t value, uint8_t* output) noexcept {
    output[0] = static_cast<uint8_t>(value & UINT32_C(0xff));
    output[1] = static_cast<uint8_t>((value >> 8U) & UINT32_C(0xff));
    output[2] = static_cast<uint8_t>((value >> 16U) & UINT32_C(0xff));
    output[3] = static_cast<uint8_t>((value >> 24U) & UINT32_C(0xff));
}

bool has_exact_arguments(
    const DISPPARAMS* parameters,
    UINT argument_count) noexcept {
    return parameters != nullptr &&
        parameters->cArgs == argument_count &&
        parameters->cNamedArgs == 0 &&
        (argument_count == 0 || parameters->rgvarg != nullptr);
}

bool read_i32(const VARIANTARG& argument, LONG* value) noexcept {
    if (argument.vt != VT_I4 || value == nullptr) {
        return false;
    }
    *value = argument.lVal;
    return true;
}

bool read_u32(const VARIANTARG& argument, ULONG* value) noexcept {
    if (argument.vt != VT_UI4 || value == nullptr) {
        return false;
    }
    *value = argument.ulVal;
    return true;
}

bool read_bool(const VARIANTARG& argument) noexcept {
    return argument.vt == VT_BOOL;
}

bool set_i32_by_ref(VARIANTARG& argument, LONG value) noexcept {
    if (argument.vt != (VT_I4 | VT_BYREF) || argument.plVal == nullptr) {
        return false;
    }
    *argument.plVal = value;
    return true;
}

bool set_bool_by_ref(VARIANTARG& argument, VARIANT_BOOL value) noexcept {
    if (argument.vt != (VT_BOOL | VT_BYREF) ||
        argument.pboolVal == nullptr) {
        return false;
    }
    *argument.pboolVal = value;
    return true;
}

void dispatch(
    NativeRdpHost* host,
    uint32_t kind,
    int32_t code,
    const uint8_t* payload,
    uint32_t payload_length) noexcept {
    const uint64_t generation = host->generation;
    const NavopRdpEvent event{
        static_cast<uint32_t>(sizeof(NavopRdpEvent)),
        NAVOP_RDP_ABI_VERSION,
        kind,
        UINT32_C(0),
        static_cast<uint32_t>(generation),
        static_cast<uint32_t>(generation >> 32U),
        code,
        payload_length};
    const NavopRdpResult result = dispatch_event(host, &event, payload);
    if (result != NAVOP_RDP_RESULT_OK) {
        trace_native_result("event_sink.dispatch_failure", result);
    }
}

void dispatch_disconnected(
    NativeRdpHost* host,
    int32_t disconnect_code,
    const int32_t* extended_code) noexcept {
    if (extended_code == nullptr) {
        dispatch(
            host,
            NAVOP_RDP_EVENT_DISCONNECTED,
            disconnect_code,
            nullptr,
            0);
        return;
    }

    std::array<uint8_t, 4> payload{};
    encode_u32_le(
        static_cast<uint32_t>(*extended_code),
        payload.data());
    dispatch(
        host,
        NAVOP_RDP_EVENT_DISCONNECTED,
        disconnect_code,
        payload.data(),
        static_cast<uint32_t>(payload.size()));
}

class RdpEventSink final : public IDispatch {
public:
    explicit RdpEventSink(NativeRdpHost* host) noexcept
        : ref_count_(1), host_(host) {}

    void detach() noexcept {
        host_ = nullptr;
    }

    HRESULT STDMETHODCALLTYPE QueryInterface(
        REFIID interface_id,
        void** object) noexcept override {
        if (object == nullptr) {
            return E_POINTER;
        }
        *object = nullptr;
        if (interface_id == IID_IUnknown ||
            interface_id == IID_IDispatch ||
            interface_id == __uuidof(IMsTscAxEvents)) {
            *object = static_cast<IDispatch*>(this);
            AddRef();
            return S_OK;
        }
        return E_NOINTERFACE;
    }

    ULONG STDMETHODCALLTYPE AddRef() noexcept override {
        return static_cast<ULONG>(InterlockedIncrement(&ref_count_));
    }

    ULONG STDMETHODCALLTYPE Release() noexcept override {
        const LONG references = InterlockedDecrement(&ref_count_);
        if (references == 0) {
            delete this;
            return 0;
        }
        return static_cast<ULONG>(references);
    }

    HRESULT STDMETHODCALLTYPE GetTypeInfoCount(
        UINT* count) noexcept override {
        if (count == nullptr) {
            return E_POINTER;
        }
        *count = 0;
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE GetTypeInfo(
        UINT,
        LCID,
        ITypeInfo**) noexcept override {
        return E_NOTIMPL;
    }

    HRESULT STDMETHODCALLTYPE GetIDsOfNames(
        REFIID,
        LPOLESTR*,
        UINT,
        LCID,
        DISPID*) noexcept override {
        return DISP_E_UNKNOWNNAME;
    }

    HRESULT STDMETHODCALLTYPE Invoke(
        DISPID dispatch_id,
        REFIID,
        LCID,
        WORD,
        DISPPARAMS* parameters,
        VARIANT*,
        EXCEPINFO*,
        UINT*) noexcept override {
        try {
            NativeRdpHost* host = host_;
            if (host == nullptr) {
                trace_native_stage("event_sink.detached_callback");
                return S_OK;
            }

            std::array<uint8_t, 8> payload{};
            switch (dispatch_id) {
            case kOnConnecting:
                if (!dispatch_no_arguments(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_CONNECTING)) {
                    trace_native_stage("event_sink.connecting.malformed_arguments");
                }
                break;
            case kOnConnected:
                if (!dispatch_no_arguments(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_CONNECTED)) {
                    trace_native_stage("event_sink.connected.malformed_arguments");
                }
                break;
            case kOnLoginComplete:
                if (!dispatch_no_arguments(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_LOGIN_COMPLETE)) {
                    trace_native_stage(
                        "event_sink.login_complete.malformed_arguments");
                }
                break;
            case kOnDisconnected:
                if (!dispatch_disconnected_from_parameters(host, parameters)) {
                    trace_native_stage(
                        "event_sink.disconnected.malformed_arguments");
                }
                break;
            case kOnEnterFullScreenMode:
                if (!dispatch_no_arguments(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_ENTER_FULLSCREEN)) {
                    trace_native_stage(
                        "event_sink.enter_fullscreen.malformed_arguments");
                }
                break;
            case kOnLeaveFullScreenMode:
                if (!dispatch_no_arguments(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_LEAVE_FULLSCREEN)) {
                    trace_native_stage(
                        "event_sink.leave_fullscreen.malformed_arguments");
                }
                break;
            case kOnFatalError:
                if (!dispatch_code(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_FATAL_ERROR)) {
                    trace_native_stage("event_sink.fatal_error.malformed_arguments");
                }
                break;
            case kOnWarning:
                if (!dispatch_code(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_WARNING)) {
                    trace_native_stage("event_sink.warning.malformed_arguments");
                }
                break;
            case kOnRemoteDesktopSizeChange:
                if (has_exact_arguments(parameters, 2)) {
                    LONG width = 0;
                    LONG height = 0;
                    if (read_i32(parameters->rgvarg[1], &width) &&
                        read_i32(parameters->rgvarg[0], &height)) {
                        encode_u32_le(
                            static_cast<uint32_t>(width),
                            payload.data());
                        encode_u32_le(
                            static_cast<uint32_t>(height),
                            payload.data() + 4);
                        dispatch(
                            host,
                            NAVOP_RDP_EVENT_REMOTE_DESKTOP_SIZE_CHANGED,
                            0,
                            payload.data(),
                            8);
                    } else {
                        trace_native_stage(
                            "event_sink.size_change.invalid_parameter_types");
                    }
                } else {
                    trace_native_stage(
                        "event_sink.size_change.invalid_parameter_count");
                }
                break;
            case kOnConfirmClose:
                if (has_exact_arguments(parameters, 1) &&
                    set_bool_by_ref(
                        parameters->rgvarg[0],
                        VARIANT_TRUE)) {
                    dispatch(
                        host,
                        NAVOP_RDP_EVENT_CLOSE_CONFIRMED,
                        0,
                        nullptr,
                        0);
                } else {
                    trace_native_stage(
                        "event_sink.confirm_close.malformed_arguments");
                }
                break;
            case kOnAutoReconnecting:
                if (has_exact_arguments(parameters, 3)) {
                    LONG disconnect_reason = 0;
                    LONG attempt_count = 0;
                    if (set_i32_by_ref(
                            parameters->rgvarg[0],
                            kAutoReconnectContinueAutomatic) &&
                        read_i32(
                            parameters->rgvarg[1],
                            &attempt_count) &&
                        read_i32(
                            parameters->rgvarg[2],
                            &disconnect_reason)) {
                        trace_native_win32(
                            "event_sink.auto_reconnecting.disconnect_reason",
                            static_cast<uint32_t>(disconnect_reason));
                        encode_u32_le(
                            static_cast<uint32_t>(attempt_count),
                            payload.data());
                        dispatch(
                            host,
                            NAVOP_RDP_EVENT_RECONNECTING,
                            0,
                            payload.data(),
                            4);
                    } else {
                        trace_native_stage(
                            "event_sink.auto_reconnecting.malformed_arguments");
                    }
                } else {
                    trace_native_stage(
                        "event_sink.auto_reconnecting.invalid_parameter_count");
                }
                break;
            case kOnAuthenticationWarningDisplayed:
                if (!dispatch_no_arguments(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISPLAYED)) {
                    trace_native_stage(
                        "event_sink.authentication_displayed.malformed_arguments");
                }
                break;
            case kOnAuthenticationWarningDismissed:
                if (!dispatch_no_arguments(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISMISSED)) {
                    trace_native_stage(
                        "event_sink.authentication_dismissed.malformed_arguments");
                }
                break;
            case kOnLogonError:
                if (!dispatch_code(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_LOGON_ERROR)) {
                    trace_native_stage("event_sink.logon_error.malformed_arguments");
                }
                break;
            case kOnFocusReleased:
                if (has_exact_arguments(parameters, 1)) {
                    LONG direction = 0;
                    if (read_i32(
                            parameters->rgvarg[0],
                            &direction)) {
                        trace_native_win32(
                            "event_sink.focus_released.direction",
                            static_cast<uint32_t>(direction));
                        dispatch(
                            host,
                            NAVOP_RDP_EVENT_FOCUS_RELEASED,
                            0,
                            nullptr,
                            0);
                    } else {
                        trace_native_stage(
                            "event_sink.focus_released.invalid_parameter_types");
                    }
                } else {
                    trace_native_stage(
                        "event_sink.focus_released.invalid_parameter_count");
                }
                break;
            case kOnNetworkStatusChanged:
                if (has_exact_arguments(parameters, 3)) {
                    ULONG quality_level = 0;
                    LONG bandwidth = 0;
                    LONG round_trip_time = 0;
                    if (read_i32(
                            parameters->rgvarg[0],
                            &round_trip_time) &&
                        read_i32(
                            parameters->rgvarg[1],
                            &bandwidth) &&
                        read_u32(
                            parameters->rgvarg[2],
                            &quality_level)) {
                        trace_native_win32(
                            "event_sink.network_status.bandwidth",
                            static_cast<uint32_t>(bandwidth));
                        trace_native_win32(
                            "event_sink.network_status.round_trip_time",
                            static_cast<uint32_t>(round_trip_time));
                        encode_u32_le(
                            static_cast<uint32_t>(quality_level),
                            payload.data());
                        dispatch(
                            host,
                            NAVOP_RDP_EVENT_NETWORK_STATUS_CHANGED,
                            0,
                            payload.data(),
                            4);
                    } else {
                        trace_native_stage(
                            "event_sink.network_status.invalid_parameter_types");
                    }
                } else {
                    trace_native_stage(
                        "event_sink.network_status.invalid_parameter_count");
                }
                break;
            case kOnAutoReconnected:
                if (!dispatch_no_arguments(
                        host,
                        parameters,
                        NAVOP_RDP_EVENT_RECONNECTED)) {
                    trace_native_stage(
                        "event_sink.auto_reconnected.malformed_arguments");
                }
                break;
            case kOnAutoReconnecting2:
                if (has_exact_arguments(parameters, 4)) {
                    LONG disconnect_reason = 0;
                    LONG attempt_count = 0;
                    LONG max_attempt_count = 0;
                    if (read_i32(
                            parameters->rgvarg[0],
                            &max_attempt_count) &&
                        read_i32(
                            parameters->rgvarg[1],
                            &attempt_count) &&
                        read_bool(parameters->rgvarg[2]) &&
                        read_i32(
                            parameters->rgvarg[3],
                            &disconnect_reason)) {
                        trace_native_win32(
                            "event_sink.auto_reconnecting2.disconnect_reason",
                            static_cast<uint32_t>(disconnect_reason));
                        encode_u32_le(
                            static_cast<uint32_t>(attempt_count),
                            payload.data());
                        encode_u32_le(
                            static_cast<uint32_t>(max_attempt_count),
                            payload.data() + 4);
                        dispatch(
                            host,
                            NAVOP_RDP_EVENT_RECONNECTING,
                            0,
                            payload.data(),
                            8);
                    } else {
                        trace_native_stage(
                            "event_sink.auto_reconnecting2.malformed_arguments");
                    }
                } else {
                    trace_native_stage(
                        "event_sink.auto_reconnecting2.invalid_parameter_count");
                }
                break;
            default:
                // Future type-library events are intentionally ignored until
                // Navop assigns them an immutable Rust event kind/schema.
                trace_native_win32(
                    "event_sink.unknown_dispatch_id",
                    static_cast<uint32_t>(dispatch_id));
                break;
            }
            return S_OK;
        } catch (...) {
            trace_native_stage("event_sink.exception");
            return S_OK;
        }
    }

private:
    ~RdpEventSink() noexcept = default;

    static bool dispatch_no_arguments(
        NativeRdpHost* host,
        const DISPPARAMS* parameters,
        uint32_t kind) noexcept {
        if (!has_exact_arguments(parameters, 0)) {
            return false;
        }
        dispatch(host, kind, 0, nullptr, 0);
        return true;
    }

    static bool dispatch_code(
        NativeRdpHost* host,
        const DISPPARAMS* parameters,
        uint32_t kind) noexcept {
        if (!has_exact_arguments(parameters, 1)) {
            return false;
        }
        LONG code = 0;
        if (!read_i32(parameters->rgvarg[0], &code)) {
            return false;
        }
        dispatch(host, kind, static_cast<int32_t>(code), nullptr, 0);
        return true;
    }

    static bool dispatch_disconnected_from_parameters(
        NativeRdpHost* host,
        const DISPPARAMS* parameters) noexcept {
        if (!has_exact_arguments(parameters, 1)) {
            return false;
        }

        LONG disconnect_code = 0;
        if (!read_i32(parameters->rgvarg[0], &disconnect_code)) {
            return false;
        }

        int32_t extended_code = 0;
        const NavopRdpResult extended_result =
            get_active_x_extended_disconnect_reason(
                host->active_x_resources,
                &extended_code);
        trace_active_x_disconnect_description(
            host->active_x_resources,
            static_cast<int32_t>(disconnect_code),
            extended_result == NAVOP_RDP_RESULT_OK
                ? extended_code
                : 0);
        dispatch_disconnected(
            host,
            static_cast<int32_t>(disconnect_code),
            extended_result == NAVOP_RDP_RESULT_OK
                ? &extended_code
                : nullptr);
        return true;
    }

    volatile LONG ref_count_;
    NativeRdpHost* host_;
};

}  // namespace

struct NativeRdpEventSubscription {
    CComPtr<IConnectionPoint> connection_point;
    RdpEventSink* sink = nullptr;
    DWORD advise_cookie = 0;
};

NavopRdpResult create_event_subscription(
    NativeRdpHost* host,
    IUnknown* control,
    NativeRdpEventSubscription** out_subscription) noexcept {
    try {
        if (host == nullptr ||
            control == nullptr ||
            out_subscription == nullptr) {
            return host == nullptr
                ? NAVOP_RDP_RESULT_INVALID_ARGUMENT
                : record_last_error(
                      host,
                      NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        *out_subscription = nullptr;

        CComPtr<IConnectionPointContainer> connection_point_container;
        trace_native_stage("event_subscription.query_container.before");
        HRESULT result = control->QueryInterface(
            IID_PPV_ARGS(&connection_point_container));
        trace_native_hresult(
            "event_subscription.query_container.after",
            static_cast<int32_t>(result));
        if (FAILED(result) || connection_point_container == nullptr) {
            if (FAILED(result)) {
                return record_last_hresult(
                    host,
                    NAVOP_RDP_RESULT_UNAVAILABLE,
                    static_cast<int32_t>(result));
            }
            return record_last_error(host, NAVOP_RDP_RESULT_UNAVAILABLE);
        }

        CComPtr<IConnectionPoint> connection_point;
        trace_native_stage("event_subscription.find_connection_point.before");
        result = connection_point_container->FindConnectionPoint(
            __uuidof(IMsTscAxEvents),
            &connection_point);
        trace_native_hresult(
            "event_subscription.find_connection_point.after",
            static_cast<int32_t>(result));
        if (FAILED(result) || connection_point == nullptr) {
            if (FAILED(result)) {
                return record_last_hresult(
                    host,
                    NAVOP_RDP_RESULT_UNAVAILABLE,
                    static_cast<int32_t>(result));
            }
            return record_last_error(host, NAVOP_RDP_RESULT_UNAVAILABLE);
        }

        trace_native_stage("event_subscription.allocate.before");
        auto subscription = std::unique_ptr<NativeRdpEventSubscription>(
            new (std::nothrow) NativeRdpEventSubscription());
        if (!subscription) {
            trace_native_stage("event_subscription.allocate.failed");
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_ALLOCATION_FAILED);
        }
        RdpEventSink* sink = new (std::nothrow) RdpEventSink(host);
        if (sink == nullptr) {
            trace_native_stage("event_subscription.sink_allocate.failed");
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_ALLOCATION_FAILED);
        }
        trace_native_stage("event_subscription.allocate.after");

        DWORD advise_cookie = 0;
        trace_native_stage("event_subscription.advise.before");
        result = connection_point->Advise(
            static_cast<IDispatch*>(sink),
            &advise_cookie);
        trace_native_hresult(
            "event_subscription.advise.after",
            static_cast<int32_t>(result));
        if (FAILED(result) || advise_cookie == 0) {
            sink->Release();
            if (FAILED(result)) {
                return record_last_hresult(
                    host,
                    NAVOP_RDP_RESULT_UNAVAILABLE,
                    static_cast<int32_t>(result));
            }
            return record_last_error(host, NAVOP_RDP_RESULT_UNAVAILABLE);
        }

        subscription->connection_point = connection_point;
        subscription->sink = sink;
        subscription->advise_cookie = advise_cookie;
        *out_subscription = subscription.release();
        trace_native_stage("event_subscription.complete");
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}

void destroy_event_subscription(
    NativeRdpEventSubscription* subscription) noexcept {
    if (subscription == nullptr) {
        return;
    }
    trace_native_stage("event_subscription.destroy.begin");

    RdpEventSink* sink = subscription->sink;
    if (sink != nullptr) {
        trace_native_stage("event_subscription.destroy.detach.before");
        sink->detach();
        trace_native_stage("event_subscription.destroy.detach.after");
    }
    if (subscription->connection_point != nullptr &&
        subscription->advise_cookie != 0) {
        trace_native_stage("event_subscription.destroy.unadvise.before");
        const HRESULT unadvise_result =
            subscription->connection_point->Unadvise(
                subscription->advise_cookie);
        trace_native_hresult(
            "event_subscription.destroy.unadvise.after",
            static_cast<int32_t>(unadvise_result));
    }
    subscription->advise_cookie = 0;
    trace_native_stage("event_subscription.destroy.release_connection_point.before");
    subscription->connection_point.Release();
    trace_native_stage("event_subscription.destroy.release_connection_point.after");
    if (sink != nullptr) {
        trace_native_stage("event_subscription.destroy.release_sink.before");
        sink->Release();
        trace_native_stage("event_subscription.destroy.release_sink.after");
    }
    subscription->sink = nullptr;
    delete subscription;
    trace_native_stage("event_subscription.destroy.complete");
}

extern "C" NavopRdpResult navop_rdp_test_invoke_active_x_event(
    NativeRdpHost* host,
    int32_t dispatch_id,
    int32_t* arguments,
    const uint16_t* variant_types,
    uint32_t argument_count) noexcept {
    try {
        if (host == nullptr ||
            argument_count > kMaxTestArguments ||
            (argument_count != 0 &&
             (arguments == nullptr || variant_types == nullptr))) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        std::array<VARIANTARG, kMaxTestArguments> variants{};
        std::array<LONG, kMaxTestArguments> by_ref_i32{};
        std::array<VARIANT_BOOL, kMaxTestArguments> by_ref_bool{};
        for (uint32_t index = 0; index < argument_count; ++index) {
            VARIANTARG& variant = variants[index];
            VariantInit(&variant);
            variant.vt = variant_types[index];
            switch (variant.vt) {
            case VT_I4:
                variant.lVal = static_cast<LONG>(arguments[index]);
                break;
            case VT_UI4:
                variant.ulVal = static_cast<ULONG>(arguments[index]);
                break;
            case VT_BOOL:
                variant.boolVal = static_cast<VARIANT_BOOL>(arguments[index]);
                break;
            case VT_I4 | VT_BYREF:
                by_ref_i32[index] = static_cast<LONG>(arguments[index]);
                variant.plVal = &by_ref_i32[index];
                break;
            case VT_BOOL | VT_BYREF:
                by_ref_bool[index] =
                    static_cast<VARIANT_BOOL>(arguments[index]);
                variant.pboolVal = &by_ref_bool[index];
                break;
            default:
                variant.llVal = static_cast<LONGLONG>(arguments[index]);
                break;
            }
        }

        DISPPARAMS parameters{
            argument_count == 0 ? nullptr : variants.data(),
            nullptr,
            argument_count,
            0};
        RdpEventSink* sink = new (std::nothrow) RdpEventSink(host);
        if (sink == nullptr) {
            return NAVOP_RDP_RESULT_ALLOCATION_FAILED;
        }
        const HRESULT result = sink->Invoke(
            static_cast<DISPID>(dispatch_id),
            IID_NULL,
            LOCALE_USER_DEFAULT,
            DISPATCH_METHOD,
            &parameters,
            nullptr,
            nullptr,
            nullptr);
        sink->Release();

        for (uint32_t index = 0; index < argument_count; ++index) {
            if (variant_types[index] == (VT_I4 | VT_BYREF)) {
                arguments[index] = static_cast<int32_t>(by_ref_i32[index]);
            } else if (variant_types[index] == (VT_BOOL | VT_BYREF)) {
                arguments[index] =
                    static_cast<int32_t>(by_ref_bool[index]);
            }
        }
        return SUCCEEDED(result)
            ? NAVOP_RDP_RESULT_OK
            : NAVOP_RDP_RESULT_INTERNAL_ERROR;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_test_dispatch_disconnect_event(
    NativeRdpHost* host,
    int32_t disconnect_code,
    uint32_t has_extended_code,
    int32_t extended_code) noexcept {
    if (host == nullptr || has_extended_code > UINT32_C(1)) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    dispatch_disconnected(
        host,
        disconnect_code,
        has_extended_code == UINT32_C(1) ? &extended_code : nullptr);
    return NAVOP_RDP_RESULT_OK;
}
