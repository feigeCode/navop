#include "connection_policy_internal.h"

#include <windows.h>

#include <atlbase.h>

#pragma warning(push)
#pragma warning(disable : 4471)
#include "mstscax.tlh"
#pragma warning(pop)

namespace {

// IMsRdpClientTransportSettings::GatewayUsageMethod values:
// 1 = always use the RD Gateway, 2 = use it only when a direct connection
// fails (this is how the "bypass for local addresses" behavior is expressed).
constexpr LONG kGatewayUsageAlways = 1;
constexpr LONG kGatewayUsageAutoDetect = 2;
// IMsRdpClientTransportSettings2::GatewayProfileUsageMethod = 1 selects the
// explicit settings the caller provided.
constexpr LONG kGatewayProfileUsageExplicit = 1;
// GatewayCredSharing = 0 keeps the gateway credentials private to the gateway
// connection instead of sharing them with the session.
constexpr LONG kGatewayCredSharingDisabled = 0;

NavopRdpResult get_transport_settings2(
    NativeRdpHost* owner,
    IUnknown* client,
    IMsRdpClientTransportSettings2** out_transport) noexcept {
    CComQIPtr<IMsRdpClient7> client7(client);
    if (client7 == nullptr) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    CComPtr<IMsRdpClientTransportSettings2> transport;
    trace_native_stage("connect.gateway.get_transport_settings2.before");
    const HRESULT result = client7->get_TransportSettings2(&transport);
    trace_native_hresult(
        "connect.gateway.get_transport_settings2.after",
        static_cast<int32_t>(result));
    if (FAILED(result) || transport == nullptr) {
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
    *out_transport = transport.Detach();
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult require_gateway_supported(
    NativeRdpHost* owner,
    IUnknown* transport) noexcept {
    bool supported = false;
    const HRESULT result = get_dispatch_bool(
        transport,
        L"GatewayIsSupported",
        &supported);
    trace_native_hresult(
        "connect.gateway.is_supported",
        static_cast<int32_t>(result));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    if (!supported) {
        trace_native_stage("connect.gateway.unsupported");
        return record_last_error(owner, NAVOP_RDP_RESULT_UNAVAILABLE);
    }
    return NAVOP_RDP_RESULT_OK;
}

}  // namespace

NavopRdpResult configure_gateway_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept {
    if (options.gateway_mode == NAVOP_RDP_GATEWAY_MODE_NONE) {
        trace_native_stage("connect.gateway.disabled");
        return NAVOP_RDP_RESULT_OK;
    }

    CComPtr<IMsRdpClientTransportSettings2> transport;
    NavopRdpResult result = get_transport_settings2(
        context.owner,
        context.client,
        &transport);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }
    result = require_gateway_supported(context.owner, transport);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const NativeRdpDispatchTarget profile_usage{
        transport,
        L"GatewayProfileUsageMethod",
        "connect.gateway.profile_usage",
    };
    result = set_required_dispatch_long(
        context.owner,
        profile_usage,
        kGatewayProfileUsageExplicit);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const LONG usage_method =
        options.gateway_mode == NAVOP_RDP_GATEWAY_MODE_EXPLICIT
        ? kGatewayUsageAlways
        : kGatewayUsageAutoDetect;
    const NativeRdpDispatchTarget usage{
        transport,
        L"GatewayUsageMethod",
        "connect.gateway.usage_method",
    };
    result = set_required_dispatch_long(context.owner, usage, usage_method);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    // Only the hostname length is logged; the hostname itself never crosses
    // the diagnostic surface.
    const NativeRdpDispatchTarget hostname{
        transport,
        L"GatewayHostname",
        "connect.gateway.hostname",
    };
    result = set_required_dispatch_utf16(
        context.owner,
        hostname,
        options.gateway_hostname);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const NativeRdpDispatchTarget credentials{
        transport,
        L"GatewayCredsSource",
        "connect.gateway.credentials",
    };
    result = set_required_dispatch_long(
        context.owner,
        credentials,
        static_cast<LONG>(options.gateway_credential_source));
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    const NativeRdpDispatchTarget cred_sharing{
        transport,
        L"GatewayCredSharing",
        "connect.gateway.cred_sharing",
    };
    result = set_required_dispatch_long(
        context.owner,
        cred_sharing,
        kGatewayCredSharingDisabled);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    // BYPASS_LOCAL has no reliable standalone ActiveX property. Do not
    // silently approximate it with GatewayUsageMethod: callers must either
    // disable the flag or receive an explicit unavailable result.
    if ((options.gateway_flags & NAVOP_RDP_GATEWAY_FLAG_BYPASS_LOCAL) != 0) {
        trace_native_stage("connect.gateway.bypass_local.unavailable");
        return record_last_error(
            context.owner,
            NAVOP_RDP_RESULT_UNAVAILABLE);
    }
    return NAVOP_RDP_RESULT_OK;
}
