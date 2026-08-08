#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>
#include <ocidl.h>

#pragma warning(push)
#pragma warning(disable : 4192)
#import "libid:8C11EFA1-92C3-11D1-BC1E-00C04FA31489" \
    raw_interfaces_only, named_guids, no_namespace, exclude("UINT_PTR")
#pragma warning(pop)
#include "mstscax.tlh"

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
    static_cast<void>(dispatch_event(host, &event, payload));
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
                return S_OK;
            }

            std::array<uint8_t, 8> payload{};
            switch (dispatch_id) {
            case kOnConnecting:
                dispatch_no_arguments(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_CONNECTING);
                break;
            case kOnConnected:
                dispatch_no_arguments(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_CONNECTED);
                break;
            case kOnLoginComplete:
                dispatch_no_arguments(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_LOGIN_COMPLETE);
                break;
            case kOnDisconnected:
                dispatch_disconnected_from_parameters(host, parameters);
                break;
            case kOnEnterFullScreenMode:
                dispatch_no_arguments(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_ENTER_FULLSCREEN);
                break;
            case kOnLeaveFullScreenMode:
                dispatch_no_arguments(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_LEAVE_FULLSCREEN);
                break;
            case kOnFatalError:
                dispatch_code(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_FATAL_ERROR);
                break;
            case kOnWarning:
                dispatch_code(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_WARNING);
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
                    }
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
                        static_cast<void>(disconnect_reason);
                        encode_u32_le(
                            static_cast<uint32_t>(attempt_count),
                            payload.data());
                        dispatch(
                            host,
                            NAVOP_RDP_EVENT_RECONNECTING,
                            0,
                            payload.data(),
                            4);
                    }
                }
                break;
            case kOnAuthenticationWarningDisplayed:
                dispatch_no_arguments(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISPLAYED);
                break;
            case kOnAuthenticationWarningDismissed:
                dispatch_no_arguments(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISMISSED);
                break;
            case kOnLogonError:
                dispatch_code(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_LOGON_ERROR);
                break;
            case kOnFocusReleased:
                if (has_exact_arguments(parameters, 1)) {
                    LONG direction = 0;
                    if (read_i32(
                            parameters->rgvarg[0],
                            &direction)) {
                        static_cast<void>(direction);
                        dispatch(
                            host,
                            NAVOP_RDP_EVENT_FOCUS_RELEASED,
                            0,
                            nullptr,
                            0);
                    }
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
                        static_cast<void>(bandwidth);
                        static_cast<void>(round_trip_time);
                        encode_u32_le(
                            static_cast<uint32_t>(quality_level),
                            payload.data());
                        dispatch(
                            host,
                            NAVOP_RDP_EVENT_NETWORK_STATUS_CHANGED,
                            0,
                            payload.data(),
                            4);
                    }
                }
                break;
            case kOnAutoReconnected:
                dispatch_no_arguments(
                    host,
                    parameters,
                    NAVOP_RDP_EVENT_RECONNECTED);
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
                        static_cast<void>(disconnect_reason);
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
                    }
                }
                break;
            default:
                // Future type-library events are intentionally ignored until
                // Navop assigns them an immutable Rust event kind/schema.
                break;
            }
            return S_OK;
        } catch (...) {
            return S_OK;
        }
    }

private:
    ~RdpEventSink() noexcept = default;

    static void dispatch_no_arguments(
        NativeRdpHost* host,
        const DISPPARAMS* parameters,
        uint32_t kind) noexcept {
        if (has_exact_arguments(parameters, 0)) {
            dispatch(host, kind, 0, nullptr, 0);
        }
    }

    static void dispatch_code(
        NativeRdpHost* host,
        const DISPPARAMS* parameters,
        uint32_t kind) noexcept {
        if (!has_exact_arguments(parameters, 1)) {
            return;
        }
        LONG code = 0;
        if (read_i32(parameters->rgvarg[0], &code)) {
            dispatch(host, kind, static_cast<int32_t>(code), nullptr, 0);
        }
    }

    static void dispatch_disconnected_from_parameters(
        NativeRdpHost* host,
        const DISPPARAMS* parameters) noexcept {
        if (!has_exact_arguments(parameters, 1)) {
            return;
        }

        LONG disconnect_code = 0;
        if (!read_i32(parameters->rgvarg[0], &disconnect_code)) {
            return;
        }

        int32_t extended_code = 0;
        const NavopRdpResult extended_result =
            get_active_x_extended_disconnect_reason(
                host->active_x_resources,
                &extended_code);
        dispatch_disconnected(
            host,
            static_cast<int32_t>(disconnect_code),
            extended_result == NAVOP_RDP_RESULT_OK
                ? &extended_code
                : nullptr);
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
        HRESULT result = control->QueryInterface(
            IID_PPV_ARGS(&connection_point_container));
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
        result = connection_point_container->FindConnectionPoint(
            __uuidof(IMsTscAxEvents),
            &connection_point);
        if (FAILED(result) || connection_point == nullptr) {
            if (FAILED(result)) {
                return record_last_hresult(
                    host,
                    NAVOP_RDP_RESULT_UNAVAILABLE,
                    static_cast<int32_t>(result));
            }
            return record_last_error(host, NAVOP_RDP_RESULT_UNAVAILABLE);
        }

        auto subscription = std::unique_ptr<NativeRdpEventSubscription>(
            new (std::nothrow) NativeRdpEventSubscription());
        if (!subscription) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_ALLOCATION_FAILED);
        }
        RdpEventSink* sink = new (std::nothrow) RdpEventSink(host);
        if (sink == nullptr) {
            return record_last_error(
                host,
                NAVOP_RDP_RESULT_ALLOCATION_FAILED);
        }

        DWORD advise_cookie = 0;
        result = connection_point->Advise(
            static_cast<IDispatch*>(sink),
            &advise_cookie);
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

    RdpEventSink* sink = subscription->sink;
    if (sink != nullptr) {
        sink->detach();
    }
    if (subscription->connection_point != nullptr &&
        subscription->advise_cookie != 0) {
        static_cast<void>(subscription->connection_point->Unadvise(
            subscription->advise_cookie));
    }
    subscription->advise_cookie = 0;
    subscription->connection_point.Release();
    if (sink != nullptr) {
        sink->Release();
    }
    subscription->sink = nullptr;
    delete subscription;
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
