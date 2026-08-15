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
        static_cast<long>(options.desktop_scale_factor));
    trace_native_stage("connect.display.desktop_scale_factor.before");
    result = extended_settings->put_Property(
        CComBSTR(L"DesktopScaleFactor"),
        &desktop_scale_factor);
    trace_native_hresult(
        "connect.display.desktop_scale_factor.after",
        static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }

    CComVariant device_scale_factor(
        static_cast<long>(options.device_scale_factor));
    trace_native_stage("connect.display.device_scale_factor.before");
    result = extended_settings->put_Property(
        CComBSTR(L"DeviceScaleFactor"),
        &device_scale_factor);
    trace_native_hresult(
        "connect.display.device_scale_factor.after",
        static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    return NAVOP_RDP_RESULT_OK;
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
    result = set_required_dispatch_bool(
        context.owner,
        smart_sizing,
        (options.display_flags & NAVOP_RDP_DISPLAY_FLAG_SMART_SIZING) != 0);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const NativeRdpDispatchTarget use_multimon{
        advanced,
        L"UseMultimon",
        "connect.display.use_multimon",
    };
    result = set_required_dispatch_bool(
        context.owner,
        use_multimon,
        (options.display_flags & NAVOP_RDP_DISPLAY_FLAG_USE_MULTIMON) != 0);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const NativeRdpDispatchTarget container_full_screen{
        advanced,
        L"ContainerHandledFullScreen",
        "connect.display.container_handled_full_screen",
    };
    result = set_required_dispatch_bool(
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
