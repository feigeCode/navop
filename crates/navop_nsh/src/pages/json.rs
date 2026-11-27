use nsh_base::{input, theme};
use nsh_core::prelude::*;

use crate::ui;

pub(crate) fn json() -> impl IntoElement {
    let input_text = signal(String::from(
        "{\n  \"app\": \"Navop\",\n  \"ui\": \"nsh\",\n  \"ready\": true\n}",
    ));
    let parsed = computed(move || format_json(&input_text.get()));
    let output = move || match parsed.get() {
        Ok(value) => value,
        Err(error) => error,
    };
    let copy_text = move || match parsed.get() {
        Ok(value) => value,
        Err(error) => error,
    };

    view()
        .size_full()
        .child(ui::page_header(
            "JSON",
            "Format, validate and copy JSON with nsh's native multiline input",
        ))
        .child(
            view()
                .flex_1()
                .min_h_0()
                .gap_3()
                .p_6()
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(
                            ui::secondary_button("Clear")
                                .on_press(move || input_text.set(String::new())),
                        )
                        .child(
                            ui::primary_button("Copy formatted")
                                .on_press(move || set_clipboard(copy_text())),
                        ),
                )
                .child(
                    view()
                        .flex_1()
                        .min_h_0()
                        .flex_row()
                        .gap_3()
                        .child(
                            view()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .gap_2()
                                .child(section_label("Input"))
                                .child(
                                    input()
                                        .multiline(true)
                                        .value(move || input_text.get())
                                        .on_change(move |value: &str| {
                                            input_text.set(value.to_owned())
                                        })
                                        .placeholder("Paste JSON here")
                                        .flex_1()
                                        .min_h_0()
                                        .p_3()
                                        .rounded(move || theme().radius)
                                        .border_1()
                                        .border_color(move || theme().input)
                                        .bg(move || theme().card)
                                        .text_color(move || theme().foreground)
                                        .text_size(px(13.0)),
                                ),
                        )
                        .child(
                            view()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .gap_2()
                                .child(section_label("Formatted"))
                                .child(
                                    scroll()
                                        .flex_1()
                                        .min_h_0()
                                        .p_3()
                                        .rounded(move || theme().radius)
                                        .border_1()
                                        .border_color(move || theme().border)
                                        .bg(move || theme().muted.opacity(0.35))
                                        .child(
                                            text(output)
                                                .text_size(px(13.0))
                                                .text_color(move || theme().foreground),
                                        ),
                                ),
                        ),
                ),
        )
}

fn section_label(label: &'static str) -> impl IntoElement {
    text(label)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().muted_foreground)
}

fn format_json(raw: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|error| format!("Invalid JSON: {error}"))?;
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_valid_json_and_reports_invalid_json() {
        let formatted = format_json(r#"{"b":2,"a":1}"#).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&formatted).unwrap();
        assert_eq!(parsed["a"], 1);
        assert_eq!(parsed["b"], 2);
        assert!(formatted.contains('\n'));
        assert!(format_json("{").unwrap_err().starts_with("Invalid JSON:"));
    }
}
