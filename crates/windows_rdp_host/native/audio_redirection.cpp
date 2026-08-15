#include "connection_policy_internal.h"

#include <windows.h>

#include <atlbase.h>

#pragma warning(push)
#pragma warning(disable : 4471)
#include "mstscax.tlh"
#pragma warning(pop)

namespace {

constexpr LONG kAudioRedirectionRedirectToLocal = 0;
constexpr LONG kAudioRedirectionPlayOnRemote = 1;
constexpr LONG kAudioRedirectionDisabled = 2;

LONG audio_redirection_mode(uint32_t mode) noexcept {
    switch (mode) {
        case NAVOP_RDP_AUDIO_MODE_LOCAL:
            return kAudioRedirectionRedirectToLocal;
        case NAVOP_RDP_AUDIO_MODE_REMOTE:
            return kAudioRedirectionPlayOnRemote;
        default:
            return kAudioRedirectionDisabled;
    }
}

NavopRdpResult configure_audio_quality_and_capture(
    NativeRdpHost* owner,
    IUnknown* advanced_settings9,
    const NavopRdpConnectionOptions& options) noexcept {
    if (options.audio_mode == NAVOP_RDP_AUDIO_MODE_LOCAL) {
        const NativeRdpDispatchTarget quality{
            advanced_settings9,
            L"AudioQualityMode",
            "connect.set_audio_quality",
        };
        const NavopRdpResult result = set_required_dispatch_long(
            owner,
            quality,
            static_cast<LONG>(options.audio_quality));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
    }

    const bool capture =
        (options.audio_flags & NAVOP_RDP_AUDIO_FLAG_CAPTURE) != 0 ||
        (options.resource_flags & NAVOP_RDP_RESOURCE_FLAG_MICROPHONES) != 0;
    const NativeRdpDispatchTarget capture_target{
        advanced_settings9,
        L"AudioCaptureRedirectionMode",
        "connect.set_audio_capture",
    };
    return set_required_dispatch_bool(owner, capture_target, capture);
}

}  // namespace

NavopRdpResult configure_audio_redirection(
    NativeRdpHost* owner,
    IUnknown* client,
    const NavopRdpConnectionOptions& options) noexcept {
    CComQIPtr<IMsRdpClient7> client7(client);
    if (client7 == nullptr) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    // The SecuredSettings3 property returns IMsRdpClientSecuredSettings2.
    CComPtr<IMsRdpClientSecuredSettings2> secured_settings3;
    trace_native_stage("connect.get_secured_settings3.before");
    HRESULT result = client7->get_SecuredSettings3(&secured_settings3);
    trace_native_hresult(
        "connect.get_secured_settings3.after",
        static_cast<int32_t>(result));
    if (FAILED(result) || secured_settings3 == nullptr) {
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    trace_native_stage("connect.set_audio_redirection_mode.before");
    result = secured_settings3->put_AudioRedirectionMode(
        audio_redirection_mode(options.audio_mode));
    trace_native_hresult(
        "connect.set_audio_redirection_mode.after",
        static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }

    CComQIPtr<IMsRdpClient9> client9(client);
    if (client9 == nullptr) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
    CComPtr<IMsRdpClientAdvancedSettings8> advanced_settings9;
    trace_native_stage("connect.get_audio_advanced_settings9.before");
    result = client9->get_AdvancedSettings9(&advanced_settings9);
    trace_native_hresult(
        "connect.get_audio_advanced_settings9.after",
        static_cast<int32_t>(result));
    if (FAILED(result) || advanced_settings9 == nullptr) {
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
    return configure_audio_quality_and_capture(
        owner,
        advanced_settings9,
        options);
}
