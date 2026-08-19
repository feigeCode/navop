#pragma once

#include "host_internal.h"

struct NativeRdpDispatchTarget {
    IUnknown* object;
    const wchar_t* property_name;
    const char* trace_stage;
};

NavopRdpResult get_required_dispatch_object(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    IUnknown** out_object) noexcept;

// connection_policy_internal.h is included before the generated mstscax.tlh in
// every policy translation unit. Keep only a forward declaration here; the
// full type is available where get_advanced_settings8 is defined/used.
struct IMsRdpClientAdvancedSettings8;

NavopRdpResult get_advanced_settings8(
    NativeRdpHost* owner,
    IUnknown* client,
    IMsRdpClientAdvancedSettings8** out_settings) noexcept;

NavopRdpResult configure_redirect_bool(
    NativeRdpHost* owner,
    IUnknown* advanced,
    const wchar_t* property_name,
    const char* trace_stage,
    bool enabled) noexcept;

NavopRdpResult set_required_dispatch_bool(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    bool value) noexcept;

NavopRdpResult set_required_dispatch_long(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    LONG value) noexcept;

NavopRdpResult set_required_dispatch_utf16(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    NavopRdpBorrowedUtf16 value) noexcept;

NavopRdpResult set_optional_dispatch_bool_if_supported(
    NativeRdpHost* owner,
    const NativeRdpDispatchTarget& target,
    bool value) noexcept;

NavopRdpResult configure_display_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept;

NavopRdpResult configure_resource_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept;

NavopRdpResult configure_input_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept;

NavopRdpResult configure_performance_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept;

NavopRdpResult configure_security_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept;

NavopRdpResult configure_gateway_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept;

NavopRdpResult configure_reconnect_policy(
    const NativeRdpConnectionPolicyContext& context,
    const NavopRdpConnectionOptions& options) noexcept;
