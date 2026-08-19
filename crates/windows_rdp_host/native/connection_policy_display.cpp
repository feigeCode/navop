#include "connection_policy_internal.h"

#include <windows.h>

#include <atlbase.h>

#pragma warning(push)
#pragma warning(disable : 4471)
#include "mstscax.tlh"
#pragma warning(pop)

namespace {

// Navop hosts the ActiveX child below a GPUI child overlay, so the container
// owns fullscreen transitions and the control must never enter its own
// fullscreen mode. The control-side flag is therefore always enabled.
constexpr bool kContainerHandledFullScreen = true;

NavopRdpResult optional_extended_property_result(
    NativeRdpHost* owner,
    uint32_t stage,
    HRESULT result) noexcept {
    // Extended display properties are optional and differ across Windows
    // mstscax.dll builds. Some builds expose IMsRdpExtendedSettings but
    // return E_FAIL rather than DISP_E_UNKNOWNNAME for an unsupported
    // property. Do not let that optional DPI enhancement block RDP itself.
    if (result == S_OK || result == DISP_E_UNKNOWNNAME ||
        result == E_NOTIMPL || result == E_NOINTERFACE || result == E_FAIL) {
        if (result != S_OK) {
            trace_native_stage("connect.display.extended_property.unsupported");
        }
        return NAVOP_RDP_RESULT_OK;
    }
    if (FAILED(result)) {
        return record_last_stage_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            stage,
            static_cast<int32_t>(result));
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult configure_extended_scale_factors(
    NativeRdpHost* owner,
    IUnknown* control,
    const NavopRdpConnectionOptions& options) noexcept {
    CComPtr<IMsRdpExtendedSettings> extended_settings;
    trace_native_stage("connect.display.query_extended_settings.before");
    HRESULT result = control == nullptr
        ? E_POINTER
        : control->QueryInterface(IID_PPV_ARGS(&extended_settings));
    trace_native_hresult(
        "connect.display.query_extended_settings.after",
        static_cast<int32_t>(result));
    if (FAILED(result) || extended_settings == nullptr) {
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    CComVariant desktop_scale_factor(
        static_cast<ULONG>(options.desktop_scale_factor));
    trace_native_stage("connect.display.desktop_scale_factor.before");
    result = extended_settings->put_Property(
        CComBSTR(L"DesktopScaleFactor"),
        &desktop_scale_factor);
    trace_native_hresult(
        "connect.display.desktop_scale_factor.after",
        static_cast<int32_t>(result));
    NavopRdpResult property_result = optional_extended_property_result(
        owner,
        NAVOP_RDP_STAGE_CONNECT_DISPLAY_DESKTOP_SCALE_FACTOR,
        result);
    if (property_result != NAVOP_RDP_RESULT_OK) {
        return property_result;
    }

    CComVariant device_scale_factor(
        static_cast<ULONG>(options.device_scale_factor));
    trace_native_stage("connect.display.device_scale_factor.before");
    result = extended_settings->put_Property(
        CComBSTR(L"DeviceScaleFactor"),
        &device_scale_factor);
    trace_native_hresult(
        "connect.display.device_scale_factor.after",
        static_cast<int32_t>(result));
    return optional_extended_property_result(
        owner,
        NAVOP_RDP_STAGE_CONNECT_DISPLAY_DEVICE_SCALE_FACTOR,
        result);
}

}  // namespace

NavopRdpResult configure_display_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept {
    CComPtr<IMsRdpClientAdvancedSettings8> advanced;
    NavopRdpResult result = get_advanced_settings8(
        context.owner,
        context.client,
        &advanced);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const NativeRdpDispatchTarget smart_sizing{
        advanced,
        L"SmartSizing",
        "connect.display.smart_sizing",
    };
    result = set_optional_dispatch_bool_if_supported(
        context.owner,
        smart_sizing,
        (options.display_flags & NAVOP_RDP_DISPLAY_FLAG_SMART_SIZING) != 0);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    CComQIPtr<IMsRdpClientNonScriptable5> non_scriptable5(
        context.non_scriptable5);
    if (non_scriptable5 == nullptr) {
        return record_last_error(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    trace_native_stage("connect.display.use_multimon.before");
    const HRESULT use_multimon_result = non_scriptable5->put_UseMultimon(
        (options.display_flags & NAVOP_RDP_DISPLAY_FLAG_USE_MULTIMON) != 0
            ? VARIANT_TRUE
            : VARIANT_FALSE);
    trace_native_hresult(
        "connect.display.use_multimon.after",
        static_cast<int32_t>(use_multimon_result));
    if (FAILED(use_multimon_result)) {
        return record_last_hresult(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(use_multimon_result));
    }

    const NativeRdpDispatchTarget container_full_screen{
        advanced,
        L"ContainerHandledFullScreen",
        "connect.display.container_handled_full_screen",
    };
    result = set_optional_dispatch_bool_if_supported(
        context.owner,
        container_full_screen,
        kContainerHandledFullScreen);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    // SPAN_MONITORS has no reliable per-monitor ActiveX mapping. UseMultimon
    // above already spans every monitor; the legacy span flag is recorded as
    // best-effort/unsupported instead of being silently dropped.
    if ((options.display_flags & NAVOP_RDP_DISPLAY_FLAG_SPAN_MONITORS) != 0) {
        trace_native_stage(
            "connect.display.span_monitors.best_effort_unsupported");
    }

    return configure_extended_scale_factors(
        context.owner,
        context.control,
        options);
}
