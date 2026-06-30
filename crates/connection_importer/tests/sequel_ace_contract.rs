use connection_importer::{
    CredentialQuery, CredentialStore, ImportOptions, ImportSourceKind, PasswordImportStatus,
    parse_sequel_ace_favorites_plist_with_credentials,
    preview_connections_from_path_with_credentials,
};
use one_core::storage::DatabaseType;
use std::collections::HashMap;
use std::fs;

const SEQUEL_ACE_SAMPLE: &str = r#"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
 "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Favorites Root</key>
  <dict>
    <key>Name</key>
    <string>Favorites</string>
    <key>Children</key>
    <array>
      <dict>
        <key>id</key>
        <integer>101</integer>
        <key>name</key>
        <string>Sequel Prod</string>
        <key>type</key>
        <integer>0</integer>
        <key>host</key>
        <string>mysql.example.com</string>
        <key>port</key>
        <string>3307</string>
        <key>user</key>
        <string>app</string>
        <key>database</key>
        <string>shop</string>
      </dict>
      <dict>
        <key>Name</key>
        <string>Nested</string>
        <key>Children</key>
        <array>
          <dict>
            <key>id</key>
            <integer>102</integer>
            <key>name</key>
            <string>Socket Local</string>
            <key>type</key>
            <integer>1</integer>
            <key>socket</key>
            <string>/tmp/mysql.sock</string>
            <key>user</key>
            <string>root</string>
          </dict>
        </array>
      </dict>
    </array>
  </dict>
</dict>
</plist>
"#;

const SEQUEL_ACE_EXPORT_SAMPLE: &str = r#"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
 "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>SPConnectionFavorites</key>
  <array>
    <dict>
      <key>id</key>
      <string>201</string>
      <key>name</key>
      <string>Exported Favorite</string>
      <key>type</key>
      <integer>0</integer>
      <key>host</key>
      <string>export.example.com</string>
      <key>user</key>
      <string>export_user</string>
    </dict>
  </array>
</dict>
</plist>
"#;

#[derive(Default)]
struct FakeCredentialStore {
    passwords: HashMap<(String, String), String>,
}

impl FakeCredentialStore {
    fn with_password(mut self, service: &str, account: &str, password: &str) -> Self {
        self.passwords.insert(
            (service.to_string(), account.to_string()),
            password.to_string(),
        );
        self
    }
}

impl CredentialStore for FakeCredentialStore {
    fn get_password(&self, query: &CredentialQuery) -> Option<String> {
        self.passwords
            .get(&(query.service.clone(), query.account.clone()))
            .cloned()
    }
}

#[test]
fn parser_reads_favorites_plist_and_nested_groups() {
    let imported = parse_sequel_ace_favorites_plist_with_credentials(
        SEQUEL_ACE_SAMPLE.as_bytes(),
        ImportOptions {
            include_passwords: false,
        },
        &FakeCredentialStore::default(),
    )
    .expect("Sequel Ace sample should parse");

    assert_eq!(2, imported.len());
    let tcp = &imported[0];
    assert_eq!(ImportSourceKind::SequelAce, tcp.source);
    assert_eq!("101", tcp.source_id);
    assert_eq!("Sequel Prod", tcp.name);
    assert_eq!(DatabaseType::MySQL, tcp.database_type);
    assert_eq!("mysql.example.com", tcp.host);
    assert_eq!(Some(3307), tcp.port);
    assert_eq!("app", tcp.username);
    assert_eq!(Some("shop".to_string()), tcp.database);
    assert_eq!(PasswordImportStatus::Unsupported, tcp.password_status);

    let socket = &imported[1];
    assert_eq!("Socket Local", socket.name);
    assert_eq!("localhost", socket.host);
    assert_eq!("root", socket.username);
    assert_eq!(Some(3306), socket.port);
}

#[test]
fn parser_reads_exported_favorites_plist() {
    let imported = parse_sequel_ace_favorites_plist_with_credentials(
        SEQUEL_ACE_EXPORT_SAMPLE.as_bytes(),
        ImportOptions {
            include_passwords: false,
        },
        &FakeCredentialStore::default(),
    )
    .expect("Sequel Ace export sample should parse");

    assert_eq!(1, imported.len());
    assert_eq!("201", imported[0].source_id);
    assert_eq!("Exported Favorite", imported[0].name);
    assert_eq!("export.example.com", imported[0].host);
    assert_eq!("export_user", imported[0].username);
}

#[test]
fn parser_imports_password_from_keychain_when_requested() {
    let credentials = FakeCredentialStore::default().with_password(
        "Sequel Ace : Sequel Prod (101)",
        "app@mysql.example.com/shop",
        "secret",
    );

    let imported = parse_sequel_ace_favorites_plist_with_credentials(
        SEQUEL_ACE_SAMPLE.as_bytes(),
        ImportOptions {
            include_passwords: true,
        },
        &credentials,
    )
    .expect("Sequel Ace sample should parse");

    assert_eq!(Some("secret".to_string()), imported[0].password);
    assert_eq!(PasswordImportStatus::Included, imported[0].password_status);
    assert_eq!(None, imported[1].password);
    assert_eq!(PasswordImportStatus::Missing, imported[1].password_status);
}

#[test]
fn preview_connections_reads_file_from_path() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let favorites = temp_dir.path().join("Favorites.plist");
    fs::write(&favorites, SEQUEL_ACE_SAMPLE).expect("write favorites");

    let imported = preview_connections_from_path_with_credentials(
        ImportSourceKind::SequelAce,
        &favorites,
        ImportOptions {
            include_passwords: false,
        },
        &FakeCredentialStore::default(),
    )
    .expect("Sequel Ace file should preview");

    assert_eq!(2, imported.len());
    assert_eq!("Sequel Prod", imported[0].name);
    assert_eq!("Socket Local", imported[1].name);
}
