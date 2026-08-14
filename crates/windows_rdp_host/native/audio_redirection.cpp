#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>

#pragma warning(push)
#pragma warning(disable : 4471)
#include "mstscax.tlh"
#pragma warning(pop)

namespace {

constexpr LONG kAudioRedirectionRedirectToLocal = 0;
constexpr LONG kAudioRedirectionDisabled = 2;

LONG audio_redirection_mode(uint32_t flags) noexcept {
    return (flags & NAVOP_RDP_CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED) != 0
        ? kAudioRedirectionDisabled
        : kAudioRedirectionRedirectToLocal;
}

}  // namespace

NavopRdpResult configure_audio_redirection(
    NativeRdpHost* owner,
    IUnknown* client,
    uint32_t flags) noexcept {
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
        audio_redirection_mode(flags));
    trace_native_hresult(
        "connect.set_audio_redirection_mode.after",
        static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    return NAVOP_RDP_RESULT_OK;
}
