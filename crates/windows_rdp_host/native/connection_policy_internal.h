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

void set_best_effort_dispatch_bool(
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
