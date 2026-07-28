use declarative_ui_demo::{MAX_SPACING_SCALE, TailwindModifier, parse_classes};

#[test]
fn spacing_scale_accepts_zero_and_the_documented_upper_bound() {
    let classes = ["gap-0", "p-96", "px-1", "py-2"].map(str::to_owned);
    let parsed = parse_classes(&classes);

    assert_eq!(96, MAX_SPACING_SCALE);
    assert_eq!(
        vec![
            TailwindModifier::Gap(0),
            TailwindModifier::Padding(96),
            TailwindModifier::PaddingX(1),
            TailwindModifier::PaddingY(2),
        ],
        parsed.modifiers
    );
    assert!(parsed.unsupported.is_empty());
}

#[test]
fn spacing_scale_rejects_unsafe_or_non_integer_values() {
    let unsupported = [
        "gap-97",
        "gap-65535",
        "gap-65536",
        "gap--1",
        "gap-1.5",
        "gap-NaN",
        "gap-inf",
    ];
    let classes = unsupported.map(str::to_owned);
    let parsed = parse_classes(&classes);

    assert!(parsed.modifiers.is_empty());
    assert_eq!(unsupported, parsed.unsupported.as_slice());
}

#[test]
fn modifiers_preserve_source_order_for_last_setter_wins_semantics() {
    let classes = ["p-2", "p-4", "flex-col", "flex-row"].map(str::to_owned);
    let parsed = parse_classes(&classes);

    assert_eq!(
        vec![
            TailwindModifier::Padding(2),
            TailwindModifier::Padding(4),
            TailwindModifier::FlexColumn,
            TailwindModifier::FlexRow,
        ],
        parsed.modifiers
    );
}

#[test]
fn vertical_scroll_overflow_is_allowlisted() {
    let parsed = parse_classes(&["overflow-y-scroll".to_owned()]);

    assert_eq!(vec![TailwindModifier::OverflowYScroll], parsed.modifiers);
    assert!(parsed.unsupported.is_empty());
}
