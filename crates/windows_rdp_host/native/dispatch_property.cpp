#include "host_internal.h"

#include <oaidl.h>
#include <oleauto.h>

#include <atlbase.h>

namespace {

HRESULT resolve_property(
    IDispatch* dispatch,
    const wchar_t* property_name,
    DISPID* out_dispid) noexcept {
    if (dispatch == nullptr ||
        property_name == nullptr ||
        out_dispid == nullptr) {
        return E_POINTER;
    }
    LPOLESTR names[] = {const_cast<LPOLESTR>(property_name)};
    return dispatch->GetIDsOfNames(
        IID_NULL,
        names,
        1,
        LOCALE_USER_DEFAULT,
        out_dispid);
}

HRESULT invoke_property_put(
    IUnknown* object,
    const wchar_t* property_name,
    VARIANTARG* value) noexcept {
    if (object == nullptr || property_name == nullptr || value == nullptr) {
        return E_POINTER;
    }
    CComPtr<IDispatch> dispatch;
    HRESULT result = object->QueryInterface(IID_PPV_ARGS(&dispatch));
    if (FAILED(result) || dispatch == nullptr) {
        return FAILED(result) ? result : E_NOINTERFACE;
    }

    DISPID property_id = DISPID_UNKNOWN;
    result = resolve_property(dispatch, property_name, &property_id);
    if (FAILED(result)) {
        return result;
    }

    DISPID named_argument = DISPID_PROPERTYPUT;
    DISPPARAMS parameters{
        value,
        &named_argument,
        1,
        1,
    };
    return dispatch->Invoke(
        property_id,
        IID_NULL,
        LOCALE_USER_DEFAULT,
        DISPATCH_PROPERTYPUT,
        &parameters,
        nullptr,
        nullptr,
        nullptr);
}

HRESULT invoke_property_get(
    IUnknown* object,
    const wchar_t* property_name,
    VARIANT* out_value) noexcept {
    if (object == nullptr ||
        property_name == nullptr ||
        out_value == nullptr) {
        return E_POINTER;
    }
    CComPtr<IDispatch> dispatch;
    HRESULT result = object->QueryInterface(IID_PPV_ARGS(&dispatch));
    if (FAILED(result) || dispatch == nullptr) {
        return FAILED(result) ? result : E_NOINTERFACE;
    }

    DISPID property_id = DISPID_UNKNOWN;
    result = resolve_property(dispatch, property_name, &property_id);
    if (FAILED(result)) {
        return result;
    }

    DISPPARAMS parameters{};
    VariantInit(out_value);
    result = dispatch->Invoke(
        property_id,
        IID_NULL,
        LOCALE_USER_DEFAULT,
        DISPATCH_PROPERTYGET,
        &parameters,
        out_value,
        nullptr,
        nullptr);
    if (FAILED(result)) {
        // A failed property-get may still have written a partial VARIANT;
        // release it so callers never observe leaked object/string contents.
        VariantClear(out_value);
    }
    return result;
}

}  // namespace

HRESULT get_dispatch_object(
    IUnknown* object,
    const wchar_t* property_name,
    IUnknown** out_object) noexcept {
    if (out_object == nullptr) {
        return E_POINTER;
    }
    *out_object = nullptr;

    VARIANT value{};
    const HRESULT result = invoke_property_get(
        object,
        property_name,
        &value);
    if (FAILED(result)) {
        VariantClear(&value);
        return result;
    }

    // The resolved interface pointer is AddRef'd once on behalf of the caller:
    // CComPtr callers must use Attach, never assignment, or the object leaks an
    // extra reference.
    IUnknown* resolved = nullptr;
    if (value.vt == VT_DISPATCH) {
        resolved = value.pdispVal;
    } else if (value.vt == VT_UNKNOWN) {
        resolved = value.punkVal;
    }
    if (resolved != nullptr) {
        resolved->AddRef();
        *out_object = resolved;
    }
    VariantClear(&value);
    return resolved == nullptr ? E_NOINTERFACE : S_OK;
}

HRESULT get_dispatch_bool(
    IUnknown* object,
    const wchar_t* property_name,
    bool* out_value) noexcept {
    if (out_value == nullptr) {
        return E_POINTER;
    }

    VARIANT value{};
    HRESULT result = invoke_property_get(object, property_name, &value);
    if (FAILED(result)) {
        VariantClear(&value);
        return result;
    }

    // Convert into an independent VARIANT: VariantChangeType must never mutate
    // the value in place, or the original BSTR/IUnknown contents are leaked.
    VARIANT converted{};
    result = VariantChangeType(&converted, &value, 0, VT_BOOL);
    if (SUCCEEDED(result)) {
        *out_value = converted.boolVal != VARIANT_FALSE;
    }
    VariantClear(&converted);
    VariantClear(&value);
    return result;
}

HRESULT set_dispatch_bool(
    IUnknown* object,
    const wchar_t* property_name,
    bool value) noexcept {
    VARIANTARG argument{};
    argument.vt = VT_BOOL;
    argument.boolVal = value ? VARIANT_TRUE : VARIANT_FALSE;
    return invoke_property_put(object, property_name, &argument);
}

HRESULT set_dispatch_long(
    IUnknown* object,
    const wchar_t* property_name,
    LONG value) noexcept {
    VARIANTARG argument{};
    argument.vt = VT_I4;
    argument.lVal = value;
    return invoke_property_put(object, property_name, &argument);
}

HRESULT set_dispatch_utf16(
    IUnknown* object,
    const wchar_t* property_name,
    NavopRdpBorrowedUtf16 value) noexcept {
    // An empty string is a valid, allocatable empty BSTR: len == 0 with a null
    // data pointer must succeed, while a non-zero length requires a non-null
    // pointer.
    if (value.len != UINT32_C(0) && value.data == nullptr) {
        return E_POINTER;
    }
    static_assert(
        sizeof(value.len) <= sizeof(UINT),
        "UTF-16 length must fit in UINT");
    const UINT length = static_cast<UINT>(value.len);
    BSTR string = SysAllocStringLen(
        reinterpret_cast<const OLECHAR*>(value.data),
        length);
    if (string == nullptr) {
        return E_OUTOFMEMORY;
    }

    VARIANTARG argument{};
    argument.vt = VT_BSTR;
    argument.bstrVal = string;
    const HRESULT result =
        invoke_property_put(object, property_name, &argument);
    VariantClear(&argument);
    return result;
}
