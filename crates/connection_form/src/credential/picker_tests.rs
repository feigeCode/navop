use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, TestAppContext, Window, div,
};
use gpui_component::{Root, Theme};
use one_core::storage::{CredentialReference, CredentialSummary};

use super::{
    CredentialCapabilities, CredentialField, CredentialPickerConfig, CredentialReferencePicker,
    CredentialSelectValue, create_credential_picker, create_credential_picker_with_summaries,
};

struct PickerTestRoot {
    picker: Entity<CredentialReferencePicker>,
}

impl Render for PickerTestRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(self.picker.clone())
    }
}

fn summary() -> CredentialSummary {
    CredentialSummary {
        id: 42,
        name: "Production".to_string(),
        kind: "SSH".to_string(),
        username: Some("root".to_string()),
        has_password: true,
        has_private_key_path: true,
        has_private_key_content: false,
        has_passphrase: true,
        sync_enabled: false,
        cloud_id: None,
        last_synced_at: None,
        team_id: None,
        owner_id: None,
        created_at: None,
        updated_at: None,
    }
}

fn with_picker(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut Window, &mut Context<PickerTestRoot>) -> Entity<CredentialReferencePicker>
    + 'static,
) -> Entity<PickerTestRoot> {
    let mut form = None;
    cx.update(|cx| {
        cx.set_global(Theme::default());
        cx.open_window(Default::default(), |window, cx| {
            let entity = cx.new(|cx| PickerTestRoot {
                picker: build(window, cx),
            });
            form = Some(entity.clone());
            cx.new(|cx| Root::new(entity, window, cx))
        })
        .expect("test window opens");
    });
    form.expect("picker root created")
}

#[gpui::test]
fn picker_without_global_storage_is_safe(cx: &mut TestAppContext) {
    let form = with_picker(cx, |window, cx| {
        create_credential_picker(
            CredentialPickerConfig::new("credential-test", CredentialCapabilities::login()),
            window,
            cx,
        )
    });

    cx.update(|cx| {
        let picker = form.read(cx).picker.read(cx);
        assert_eq!(None, picker.selected_reference());
        assert_eq!(CredentialSelectValue::Manual, picker.selected_value());
    });
}

#[gpui::test]
fn picker_selection_and_fields_follow_the_reference_contract(cx: &mut TestAppContext) {
    let form = with_picker(cx, |window, cx| {
        create_credential_picker_with_summaries(
            CredentialPickerConfig::new("credential-test", CredentialCapabilities::all()),
            vec![summary()],
            window,
            cx,
        )
    });

    cx.update(|cx| {
        let picker = form.read(cx).picker.clone();
        picker.update(cx, |picker, cx| {
            picker.select_value(CredentialSelectValue::Credential(42), cx);
            picker.select_field(CredentialField::PrivateKey, true, cx);
        });
        let picker = form.read(cx).picker.read(cx);
        assert!(picker.field_referenced(CredentialField::PrivateKey));
        assert!(picker.field_referenced(CredentialField::Passphrase));
        assert!(!picker.field_referenced(CredentialField::Password));
    });
}

#[gpui::test]
fn manual_username_override_preserves_password_reference(cx: &mut TestAppContext) {
    let form = with_picker(cx, |window, cx| {
        create_credential_picker_with_summaries(
            CredentialPickerConfig::new("credential-test", CredentialCapabilities::login()),
            vec![summary()],
            window,
            cx,
        )
    });

    cx.update(|cx| {
        let picker = form.read(cx).picker.clone();
        picker.update(cx, |picker, cx| {
            picker.select_value(CredentialSelectValue::Credential(42), cx);
            picker.use_manual_field_without_window(CredentialField::Username, cx);
        });
        let picker = form.read(cx).picker.read(cx);
        assert!(!picker.field_referenced(CredentialField::Username));
        assert!(picker.field_referenced(CredentialField::Password));
    });
}

#[gpui::test]
fn manual_password_override_preserves_username_reference(cx: &mut TestAppContext) {
    let form = with_picker(cx, |window, cx| {
        create_credential_picker_with_summaries(
            CredentialPickerConfig::new("credential-test", CredentialCapabilities::login()),
            vec![summary()],
            window,
            cx,
        )
    });

    cx.update(|cx| {
        let picker = form.read(cx).picker.clone();
        picker.update(cx, |picker, cx| {
            picker.select_value(CredentialSelectValue::Credential(42), cx);
            picker.use_manual_field_without_window(CredentialField::Password, cx);
        });
        let picker = form.read(cx).picker.read(cx);
        assert!(picker.field_referenced(CredentialField::Username));
        assert!(!picker.field_referenced(CredentialField::Password));
    });
}

#[gpui::test]
fn capability_changes_normalize_existing_references(cx: &mut TestAppContext) {
    let reference = CredentialReference {
        credential_id: 42,
        credential_cloud_id: None,
        username: true,
        password: true,
        private_key: false,
        passphrase: false,
    };
    let form = with_picker(cx, move |window, cx| {
        create_credential_picker_with_summaries(
            CredentialPickerConfig::new("credential-test", CredentialCapabilities::all())
                .reference(Some(reference)),
            vec![summary()],
            window,
            cx,
        )
    });

    cx.update(|cx| {
        let picker = form.read(cx).picker.clone();
        picker.update(cx, |picker, cx| {
            picker.set_capabilities_without_window(CredentialCapabilities::private_key(), cx);
        });
        let picker = form.read(cx).picker.read(cx);
        assert!(!picker.field_referenced(CredentialField::Password));
        assert!(picker.field_referenced(CredentialField::PrivateKey));
    });
}

#[test]
fn picker_source_never_reads_plaintext_credentials() {
    let source = concat!(include_str!("picker.rs"), include_str!("repository.rs"),);

    assert!(source.contains("list_summaries"));
    assert!(!source.contains("get_plaintext"));
}
