use rusqlite::Connection;

use crate::storage::migration::run_migrations;

#[test]
fn migration_installs_grouped_global_quick_commands() {
    let connection = Connection::open_in_memory().expect("open memory sqlite");

    run_migrations(&connection).expect("run migrations");

    assert_eq!(30, count_commands(&connection));
    assert_eq!(0, count_unscoped_or_ungrouped_commands(&connection));
    assert_eq!(
        vec!["Docker", "Files", "Git", "Network", "System"],
        group_names(&connection)
    );
    for command in ["pwd", "df -h", "ip addr", "git status", "docker ps"] {
        assert_eq!(1, count_command(&connection, command), "missing {command}");
    }
}

#[test]
fn migration_is_idempotent() {
    let connection = Connection::open_in_memory().expect("open memory sqlite");

    run_migrations(&connection).expect("run initial migrations");
    run_migrations(&connection).expect("run migrations again");

    assert_eq!(30, count_commands(&connection));
    assert_eq!(30, count_distinct_commands(&connection));
}

#[test]
fn migration_preserves_an_existing_global_command() {
    let connection = Connection::open_in_memory().expect("open memory sqlite");
    run_migrations(&connection).expect("run initial migrations");
    connection
        .execute(
            "UPDATE quick_commands SET name = 'My PWD', command = ' PWD ' WHERE command = 'pwd'",
            [],
        )
        .expect("customize existing command");
    connection
        .execute(
            "DELETE FROM _migrations WHERE version = '20260720000001'",
            [],
        )
        .expect("make default migration runnable again");

    run_migrations(&connection).expect("rerun default migration");

    assert_eq!(30, count_commands(&connection));
    let name: String = connection
        .query_row(
            "SELECT name FROM quick_commands WHERE LOWER(TRIM(command)) = 'pwd'",
            [],
            |row| row.get(0),
        )
        .expect("read customized command");
    assert_eq!("My PWD", name);
}

fn count_commands(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM quick_commands", [], |row| row.get(0))
        .expect("count quick commands")
}

fn count_distinct_commands(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(DISTINCT command) FROM quick_commands",
            [],
            |row| row.get(0),
        )
        .expect("count distinct quick commands")
}

fn count_unscoped_or_ungrouped_commands(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM quick_commands
             WHERE connection_id IS NOT NULL OR TRIM(COALESCE(group_name, '')) = ''",
            [],
            |row| row.get(0),
        )
        .expect("count invalid default commands")
}

fn group_names(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT DISTINCT group_name FROM quick_commands ORDER BY group_name")
        .expect("prepare group query");
    statement
        .query_map([], |row| row.get(0))
        .expect("query groups")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect groups")
}

fn count_command(connection: &Connection, command: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM quick_commands WHERE command = ?1",
            [command],
            |row| row.get(0),
        )
        .expect("count command")
}
