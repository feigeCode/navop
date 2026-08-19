#include "connection_policy_internal.h"

#include <windows.h>

#include <atlbase.h>

#pragma warning(push)
#pragma warning(disable : 4471)
#include "mstscax.tlh"
#pragma warning(pop)

namespace {

// MSTSC TSC_PERF reverse mapping for IMsRdpClientAdvancedSettings8: the navop
// feature bits are "enabled" flags while the MSTSC PerformanceFlags mask uses
// inverted DISABLE bits plus two positive ENABLE bits.
// 0x01 DISABLE_WALLPAPER, 0x02 DISABLE_FULLWINDOWDRAG, 0x04
// DISABLE_MENUANIMATIONS, 0x08 DISABLE_THEMING, 0x20 DISABLE_CURSORSHADOW,
// 0x40 DISABLE_CURSORSETTINGS, 0x80 ENABLE_FONTSMOOTHING,
// 0x100 ENABLE_DESKTOPCOMPOSITION.
constexpr LONG kPerfDisableWallpaper = 0x01;
constexpr LONG kPerfDisableFullWindowDrag = 0x02;
constexpr LONG kPerfDisableMenuAnimations = 0x04;
constexpr LONG kPerfDisableTheming = 0x08;
constexpr LONG kPerfDisableCursorShadow = 0x20;
constexpr LONG kPerfDisableCursorSettings = 0x40;
constexpr LONG kPerfEnableFontSmoothing = 0x80;
constexpr LONG kPerfEnableDesktopComposition = 0x100;

// MSTSC NetworkConnectionType values are one-based
// (CONNECTION_TYPE_MODEM=1 ... CONNECTION_TYPE_LAN=6), while the navop ABI is
// zero-based. Autodetect (6) is not a settable MSTSC value, so the property is
// left at the control default.
constexpr LONG kNetworkConnectionTypeOffset = 1;
constexpr uint32_t kNetworkConnectionTypeAutodetect =
    NAVOP_RDP_NETWORK_CONNECTION_AUTODETECT;

LONG to_mstsc_performance_flags(uint32_t flags) noexcept {
    LONG mstsc_flags = 0;
    if ((flags & NAVOP_RDP_PERFORMANCE_FLAG_WALLPAPER) == 0) {
        mstsc_flags |= kPerfDisableWallpaper;
    }
    if ((flags & NAVOP_RDP_PERFORMANCE_FLAG_FULL_WINDOW_DRAG) == 0) {
        mstsc_flags |= kPerfDisableFullWindowDrag;
    }
    if ((flags & NAVOP_RDP_PERFORMANCE_FLAG_MENU_ANIMATIONS) == 0) {
        mstsc_flags |= kPerfDisableMenuAnimations;
    }
    if ((flags & NAVOP_RDP_PERFORMANCE_FLAG_THEMES) == 0) {
        mstsc_flags |= kPerfDisableTheming;
    }
    if ((flags & NAVOP_RDP_PERFORMANCE_FLAG_CURSOR_SHADOW) == 0) {
        mstsc_flags |= kPerfDisableCursorShadow;
    }
    if ((flags & NAVOP_RDP_PERFORMANCE_FLAG_CURSOR_SETTINGS) == 0) {
        mstsc_flags |= kPerfDisableCursorSettings;
    }
    if ((flags & NAVOP_RDP_PERFORMANCE_FLAG_FONT_SMOOTHING) != 0) {
        mstsc_flags |= kPerfEnableFontSmoothing;
    }
    if ((flags & NAVOP_RDP_PERFORMANCE_FLAG_DESKTOP_COMPOSITION) != 0) {
        mstsc_flags |= kPerfEnableDesktopComposition;
    }
    return mstsc_flags;
}

}  // namespace

NavopRdpResult configure_resource_policy(
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

    CComQIPtr<IMsRdpClientNonScriptable3> non_scriptable3(
        context.non_scriptable3);
    if (non_scriptable3 == nullptr) {
        return record_last_error(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    const struct {
        const wchar_t* property;
        const char* stage;
        uint32_t flag;
    } redirects[] = {
        {L"RedirectClipboard", "connect.resource.redirect_clipboard",
         NAVOP_RDP_RESOURCE_FLAG_CLIPBOARD},
        {L"RedirectDrives", "connect.resource.redirect_drives",
         NAVOP_RDP_RESOURCE_FLAG_DRIVES},
        {L"RedirectPrinters", "connect.resource.redirect_printers",
         NAVOP_RDP_RESOURCE_FLAG_PRINTERS},
        {L"RedirectSmartCards", "connect.resource.redirect_smart_cards",
         NAVOP_RDP_RESOURCE_FLAG_SMART_CARDS},
        {L"RedirectPOSDevices", "connect.resource.redirect_pos_devices",
         NAVOP_RDP_RESOURCE_FLAG_POS_DEVICES},
    };
    for (const auto& redirect : redirects) {
        result = configure_redirect_bool(
            context.owner,
            advanced,
            redirect.property,
            redirect.stage,
            (options.resource_flags & redirect.flag) != 0);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
    }

    trace_native_stage("connect.resource.redirect_dynamic_drives.before");
    HRESULT hresult = non_scriptable3->put_RedirectDynamicDrives(
        (options.resource_flags & NAVOP_RDP_RESOURCE_FLAG_DYNAMIC_DRIVES) != 0
            ? VARIANT_TRUE
            : VARIANT_FALSE);
    trace_native_hresult(
        "connect.resource.redirect_dynamic_drives.after",
        static_cast<int32_t>(hresult));
    if (FAILED(hresult)) {
        return record_last_hresult(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(hresult));
    }

    trace_native_stage("connect.resource.redirect_dynamic_devices.before");
    hresult = non_scriptable3->put_RedirectDynamicDevices(
        (options.resource_flags & NAVOP_RDP_RESOURCE_FLAG_DYNAMIC_DEVICES) != 0
            ? VARIANT_TRUE
            : VARIANT_FALSE);
    trace_native_hresult(
        "connect.resource.redirect_dynamic_devices.after",
        static_cast<int32_t>(hresult));
    if (FAILED(hresult)) {
        return record_last_hresult(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(hresult));
    }

    trace_native_stage("connect.resource.redirect_serial_ports.before");
    hresult = advanced->put_RedirectPorts(
        (options.resource_flags & NAVOP_RDP_RESOURCE_FLAG_SERIAL_PORTS) != 0
            ? VARIANT_TRUE
            : VARIANT_FALSE);
    trace_native_hresult(
        "connect.resource.redirect_serial_ports.after",
        static_cast<int32_t>(hresult));
    if (FAILED(hresult)) {
        return record_last_hresult(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(hresult));
    }

    // Cameras need the IMsRdpClientNonScriptable camera collection, which the
    // current ABI does not transport; the flag is consumed as unsupported.
    if ((options.resource_flags & NAVOP_RDP_RESOURCE_FLAG_CAMERAS) != 0) {
        trace_native_stage("connect.resource.cameras.unsupported");
    }
    // Microphones are consumed by the audio policy through
    // AudioCaptureRedirectionMode (see configure_audio_redirection).
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult configure_input_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept {
    CComQIPtr<IMsRdpClient7> client7(context.client);
    if (client7 == nullptr) {
        return record_last_error(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    // KeyboardHookMode belongs to IMsRdpClientSecuredSettings. Setting it
    // through AdvancedSettings9 via IDispatch fails with DISP_E_UNKNOWNNAME on
    // current MSTSC controls.
    CComPtr<IMsRdpClientSecuredSettings2> secured_settings3;
    trace_native_stage("connect.input.get_secured_settings3.before");
    HRESULT hresult = client7->get_SecuredSettings3(&secured_settings3);
    trace_native_hresult(
        "connect.input.get_secured_settings3.after",
        static_cast<int32_t>(hresult));
    if (FAILED(hresult) || secured_settings3 == nullptr) {
        if (FAILED(hresult)) {
            return record_last_hresult(
                context.owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(hresult));
        }
        return record_last_error(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    trace_native_stage("connect.input.keyboard_hook_mode.before");
    hresult = secured_settings3->put_KeyboardHookMode(
        static_cast<LONG>(options.keyboard_hook_mode));
    trace_native_hresult(
        "connect.input.keyboard_hook_mode.after",
        static_cast<int32_t>(hresult));
    // KeyboardHookMode is not implemented by every mstscax.dll build even
    // though the interface is present in the type library. The default
    // control behavior is still usable, so treat an unknown property as an
    // optional capability instead of aborting the entire connection.
    if (hresult == DISP_E_UNKNOWNNAME) {
        trace_native_stage("connect.input.keyboard_hook_mode.unsupported");
    } else if (FAILED(hresult)) {
        return record_last_hresult(
            context.owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(hresult));
    }

    CComPtr<IMsRdpClientAdvancedSettings8> advanced;
    NavopRdpResult result = get_advanced_settings8(
        context.owner,
        context.client,
        &advanced);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    result = configure_redirect_bool(
        context.owner,
        advanced,
        L"EnableWindowsKey",
        "connect.input.enable_windows_key",
        (options.input_flags & NAVOP_RDP_INPUT_FLAG_ENABLE_WINDOWS_KEY) != 0);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    return configure_redirect_bool(
        context.owner,
        advanced,
        L"GrabFocusOnConnect",
        "connect.input.grab_focus_on_connect",
        (options.input_flags & NAVOP_RDP_INPUT_FLAG_GRAB_FOCUS_ON_CONNECT) !=
            0);
}

NavopRdpResult configure_performance_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept {
    trace_native_win32(
        "connect.performance.preset",
        options.performance_preset);
    CComPtr<IMsRdpClientAdvancedSettings8> advanced;
    NavopRdpResult result = get_advanced_settings8(
        context.owner,
        context.client,
        &advanced);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const NativeRdpDispatchTarget performance_flags{
        advanced,
        L"PerformanceFlags",
        "connect.performance.flags",
    };
    result = set_required_dispatch_long(
        context.owner,
        performance_flags,
        to_mstsc_performance_flags(options.performance_flags));
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const bool bitmap_cache =
        (options.performance_flags & NAVOP_RDP_PERFORMANCE_FLAG_BITMAP_CACHE) !=
        0;
    result = configure_redirect_bool(
        context.owner,
        advanced,
        L"BitmapPersistence",
        "connect.performance.bitmap_persistence",
        bitmap_cache);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    const NativeRdpDispatchTarget cache_persistence{
        advanced,
        L"CachePersistenceActive",
        "connect.performance.cache_persistence",
    };
    result = set_required_dispatch_long(
        context.owner,
        cache_persistence,
        bitmap_cache ? 1 : 0);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    if (options.network_connection_type == kNetworkConnectionTypeAutodetect) {
        trace_native_stage(
            "connect.performance.network_connection_type.autodetect");
        return NAVOP_RDP_RESULT_OK;
    }
    const NativeRdpDispatchTarget network_type{
        advanced,
        L"NetworkConnectionType",
        "connect.performance.network_connection_type",
    };
    return set_required_dispatch_long(
        context.owner,
        network_type,
        static_cast<LONG>(options.network_connection_type) +
            kNetworkConnectionTypeOffset);
}
