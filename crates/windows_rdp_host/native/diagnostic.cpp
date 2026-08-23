#include "host_internal.h"

#include <cinttypes>
#include <cstdio>
#include <cstdlib>

namespace {

constexpr const char* kTracePrefix = "RDP_NATIVE_TRACE";
constexpr char kReplacementCharacter[] = "\xEF\xBF\xBD";

// Native stage traces are diagnostics for white-screen / z-order issues and
// run on every presentation-state poll. Keep them off by default so normal
// operation does not flood stderr; set `NAVOP_REMOTE_DESKTOP_DIAGNOSTICS`
// (the same switch as the Rust-side remote desktop diagnostics) to re-enable.
bool native_trace_enabled() noexcept {
    static const bool enabled = []() noexcept {
        char* value = nullptr;
        const errno_t error = _dupenv_s(
            &value,
            nullptr,
            "NAVOP_REMOTE_DESKTOP_DIAGNOSTICS");
        if (error != 0 || value == nullptr) {
            return false;
        }
        std::free(value);
        return true;
    }();
    return enabled;
}

void flush_trace() noexcept {
    std::fflush(stderr);
}

void write_replacement_character() noexcept {
    static_cast<void>(std::fwrite(
        kReplacementCharacter,
        1,
        sizeof(kReplacementCharacter) - 1,
        stderr));
}

void write_byte(uint8_t byte) noexcept {
    static_cast<void>(std::fputc(static_cast<int>(byte), stderr));
}

void write_utf8_code_point(uint32_t code_point) noexcept {
    if (code_point <= UINT32_C(0x7f)) {
        write_byte(static_cast<uint8_t>(code_point));
        return;
    }

    if (code_point <= UINT32_C(0x7ff)) {
        write_byte(static_cast<uint8_t>(
            UINT32_C(0xc0) | (code_point >> 6U)));
        write_byte(static_cast<uint8_t>(
            UINT32_C(0x80) | (code_point & UINT32_C(0x3f))));
        return;
    }

    if (code_point <= UINT32_C(0xffff)) {
        write_byte(static_cast<uint8_t>(
            UINT32_C(0xe0) | (code_point >> 12U)));
        write_byte(static_cast<uint8_t>(
            UINT32_C(0x80) |
            ((code_point >> 6U) & UINT32_C(0x3f))));
        write_byte(static_cast<uint8_t>(
            UINT32_C(0x80) | (code_point & UINT32_C(0x3f))));
        return;
    }

    write_byte(static_cast<uint8_t>(
        UINT32_C(0xf0) | (code_point >> 18U)));
    write_byte(static_cast<uint8_t>(
        UINT32_C(0x80) |
        ((code_point >> 12U) & UINT32_C(0x3f))));
    write_byte(static_cast<uint8_t>(
        UINT32_C(0x80) |
        ((code_point >> 6U) & UINT32_C(0x3f))));
    write_byte(static_cast<uint8_t>(
        UINT32_C(0x80) | (code_point & UINT32_C(0x3f))));
}

void write_utf16(const uint16_t* text, uint32_t text_len) noexcept {
    uint32_t index = 0;
    while (index < text_len) {
        const uint16_t first = text[index++];
        switch (first) {
        case u'\\':
            static_cast<void>(std::fputs("\\\\", stderr));
            continue;
        case u'"':
            static_cast<void>(std::fputs("\\\"", stderr));
            continue;
        case u'\n':
            static_cast<void>(std::fputs("\\n", stderr));
            continue;
        case u'\r':
            static_cast<void>(std::fputs("\\r", stderr));
            continue;
        case u'\t':
            static_cast<void>(std::fputs("\\t", stderr));
            continue;
        default:
            break;
        }
        if (first < UINT16_C(0x20) || first == UINT16_C(0x7f)) {
            static_cast<void>(std::fprintf(
                stderr,
                "\\u%04" PRIX16,
                static_cast<unsigned int>(first)));
            continue;
        }
        if (first >= UINT16_C(0xd800) &&
            first <= UINT16_C(0xdbff)) {
            if (index >= text_len) {
                write_replacement_character();
                break;
            }
            const uint16_t second = text[index];
            if (second < UINT16_C(0xdc00) ||
                second > UINT16_C(0xdfff)) {
                write_replacement_character();
                continue;
            }
            ++index;
            const uint32_t code_point =
                UINT32_C(0x10000) +
                ((static_cast<uint32_t>(first) -
                  UINT32_C(0xd800))
                 << 10U) +
                (static_cast<uint32_t>(second) -
                 UINT32_C(0xdc00));
            write_utf8_code_point(code_point);
            continue;
        }
        if (first >= UINT16_C(0xdc00) &&
            first <= UINT16_C(0xdfff)) {
            write_replacement_character();
            continue;
        }
        write_utf8_code_point(static_cast<uint32_t>(first));
    }
}

}  // namespace

void trace_native_stage(const char* stage) noexcept {
    if (!native_trace_enabled()) {
        return;
    }
    std::fprintf(
        stderr,
        "%s stage=%s\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage);
    flush_trace();
}

void trace_native_hresult(
    const char* stage,
    int32_t hresult) noexcept {
    if (!native_trace_enabled()) {
        return;
    }
    std::fprintf(
        stderr,
        "%s stage=%s hresult=0x%08" PRIX32 "\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        static_cast<uint32_t>(hresult));
    flush_trace();
}

void trace_native_result(
    const char* stage,
    NavopRdpResult result) noexcept {
    if (!native_trace_enabled()) {
        return;
    }
    std::fprintf(
        stderr,
        "%s stage=%s result=%" PRId32 "\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        result);
    flush_trace();
}

void trace_native_win32(
    const char* stage,
    uint32_t win32_code) noexcept {
    if (!native_trace_enabled()) {
        return;
    }
    std::fprintf(
        stderr,
        "%s stage=%s win32=0x%08" PRIX32 "\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        win32_code);
    flush_trace();
}

void trace_native_pointer(
    const char* stage,
    uintptr_t pointer) noexcept {
    if (!native_trace_enabled()) {
        return;
    }
    std::fprintf(
        stderr,
        "%s stage=%s pointer=0x%" PRIXPTR "\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        pointer);
    flush_trace();
}

void trace_native_rect(
    const char* stage,
    int32_t left,
    int32_t top,
    int32_t right,
    int32_t bottom) noexcept {
    if (!native_trace_enabled()) {
        return;
    }
    std::fprintf(
        stderr,
        "%s stage=%s rect={left=%" PRId32 ",top=%" PRId32
        ",right=%" PRId32 ",bottom=%" PRId32 "}\n",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        left,
        top,
        right,
        bottom);
    flush_trace();
}

void trace_native_window(
    const char* stage,
    uint32_t index,
    uintptr_t window,
    uintptr_t parent,
    uint32_t visible,
    uintptr_t style,
    uintptr_t ex_style,
    int32_t left,
    int32_t top,
    int32_t right,
    int32_t bottom,
    const uint16_t* class_name,
    uint32_t class_name_len) noexcept {
    if (!native_trace_enabled()) {
        return;
    }
    std::fprintf(
        stderr,
        "%s stage=%s index=%" PRIu32 " pointer=0x%" PRIXPTR
        " parent=0x%" PRIXPTR " visible=%" PRIu32
        " style=0x%" PRIXPTR " ex_style=0x%" PRIXPTR
        " rect={left=%" PRId32 ",top=%" PRId32
        ",right=%" PRId32 ",bottom=%" PRId32 "} class=\"",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        index,
        window,
        parent,
        visible,
        style,
        ex_style,
        left,
        top,
        right,
        bottom);
    if (class_name != nullptr && class_name_len != UINT32_C(0)) {
        write_utf16(class_name, class_name_len);
    }
    static_cast<void>(std::fputs("\"\n", stderr));
    flush_trace();
}

void trace_native_utf16(
    const char* stage,
    int32_t hresult,
    const uint16_t* text,
    uint32_t text_len) noexcept {
    if (!native_trace_enabled()) {
        return;
    }
    std::fprintf(
        stderr,
        "%s stage=%s hresult=0x%08" PRIX32 " text=\"",
        kTracePrefix,
        stage == nullptr ? "<null>" : stage,
        static_cast<uint32_t>(hresult));
    if (text != nullptr && text_len != UINT32_C(0)) {
        write_utf16(text, text_len);
    }
    static_cast<void>(std::fputs("\"\n", stderr));
    flush_trace();
}
