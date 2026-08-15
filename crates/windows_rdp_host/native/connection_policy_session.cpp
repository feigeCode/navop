#include "connection_policy_internal.h"

#include <windows.h>

#include <atlbase.h>

#pragma warning(push)
#pragma warning(disable : 4471)
#include "mstscax.tlh"
#pragma warning(pop)

namespace {

NavopRdpResult set_connection_flags_policy(
    NativeRdpHost* owner,
    IUnknown* advanced,
    const NavopRdpConnectionOptions& options) noexcept {
    const bool auto_reconnect =
        (options.connection_flags &
         NAVOP_RDP_CONNECTION_POLICY_FLAG_AUTO_RECONNECT) != 0;
    NavopRdpResult result = configure_redirect_bool(
        owner,
        advanced,
        L"EnableAutoReconnect",
        "connect.reconnect.auto_reconnect",
        auto_reconnect);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    const bool admin_session =
        (options.connection_flags &
         NAVOP_RDP_CONNECTION_POLICY_FLAG_ADMIN_SESSION) != 0;
    return configure_redirect_bool(
        owner,
        advanced,
        L"ConnectToAdministerServer",
        "connect.reconnect.admin_session",
        admin_session);
}

}  // namespace

NavopRdpResult configure_security_policy(
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

    result = configure_redirect_bool(
        context.owner,
        advanced,
        L"EnableCredSspSupport",
        "connect.security.enable_credssp",
        (options.security_flags & NAVOP_RDP_SECURITY_FLAG_ENABLE_CREDSSP) != 0);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    result = configure_redirect_bool(
        context.owner,
        advanced,
        L"PublicMode",
        "connect.security.public_mode",
        (options.security_flags & NAVOP_RDP_SECURITY_FLAG_PUBLIC_MODE) != 0);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    const NativeRdpDispatchTarget encryption{
        advanced,
        L"EncryptionEnabled",
        "connect.security.encryption",
    };
    result = set_required_dispatch_long(
        context.owner,
        encryption,
        (options.security_flags &
         NAVOP_RDP_SECURITY_FLAG_ENCRYPTION_ENABLED) != 0
            ? 1
            : 0);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    const NativeRdpDispatchTarget authentication{
        advanced,
        L"AuthenticationLevel",
        "connect.security.authentication_level",
    };
    return set_required_dispatch_long(
        context.owner,
        authentication,
        static_cast<LONG>(options.authentication_level));
}

NavopRdpResult configure_reconnect_policy(
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

    // keep_alive_seconds is validated to survive the LONG conversion after
    // the seconds → milliseconds scale.
    const NativeRdpDispatchTarget keep_alive{
        advanced,
        L"KeepAliveInterval",
        "connect.reconnect.keep_alive",
    };
    result = set_required_dispatch_long(
        context.owner,
        keep_alive,
        static_cast<LONG>(options.keep_alive_seconds) * 1000);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    const NativeRdpDispatchTarget timeout{
        advanced,
        L"OverallConnectionTimeout",
        "connect.reconnect.timeout",
    };
    result = set_required_dispatch_long(
        context.owner,
        timeout,
        static_cast<LONG>(options.timeout_seconds));
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    const NativeRdpDispatchTarget max_attempts{
        advanced,
        L"MaxReconnectAttempts",
        "connect.reconnect.max_attempts",
    };
    result = set_required_dispatch_long(
        context.owner,
        max_attempts,
        static_cast<LONG>(options.max_reconnect_attempts));
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    return set_connection_flags_policy(context.owner, advanced, options);
}
