use std::{fs, path::Path, process::Command};
use sulafat_core::ssh_config::SshConfig;

fn effective(path: &Path, alias: &str) -> String {
    // Fixtures contain no Match exec, proxy commands or hostname canonicalization.
    // -G evaluates these temporary files without opening a connection.
    let result = Command::new("ssh")
        .args(["-G", "-F"])
        .arg(path)
        .arg(alias)
        .output()
        .expect("OpenSSH client is required for the precedence regression tests");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

fn exercise(body: &str, edit: impl FnOnce(&mut sulafat_core::ssh_config::SshHost), expected: &str) {
    let dir = tempfile::tempdir().unwrap();
    let included = dir.path().join("included.conf");
    fs::write(
        &included,
        "User actual-user\nPort 2200\nIdentityFile /tmp/first-key\n",
    )
    .unwrap();
    let source = body.replace("@INCLUDE@", included.to_str().unwrap());
    let expected = expected.replace("@INCLUDE@", included.to_str().unwrap());
    let path = dir.path().join("config");
    fs::write(&path, &source).unwrap();
    let before = effective(&path, "demo");
    let mut config = SshConfig::load_from(&path).unwrap();
    let mut host = config
        .list_hosts()
        .into_iter()
        .find(|h| h.alias == "demo")
        .unwrap();
    edit(&mut host);
    config.upsert_host(host);
    config.save().unwrap();
    let after = effective(&path, "demo");
    assert_eq!(
        after, before,
        "effective configuration changed unexpectedly"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    // A second edit through the same model must preserve classification and formatting too.
    let host = config
        .list_hosts()
        .into_iter()
        .find(|h| h.alias == "demo")
        .unwrap();
    config.upsert_host(host);
    config.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    assert_eq!(
        fs::read_to_string(included).unwrap(),
        "User actual-user\nPort 2200\nIdentityFile /tmp/first-key\n"
    );
}

#[test]
fn include_before_fields_keeps_effective_user_and_identity_order() {
    let text = "Host demo\n  Include @INCLUDE@\n  # keep here\n  User fallback-user\n  IdentityFile /tmp/second-key\n  Port 22\n  User third-user\n";
    exercise(text, |_| {}, text);
}

#[test]
fn mixed_terminators_tabs_repeated_fields_and_no_final_newline_are_unchanged() {
    let text = "# top\r\nhOsT demo\r\n\tInclude @INCLUDE@\r\n\tUsEr\t fallback-user  \r\n  # between\n\tPort 0022\r\n\tIdentityFile /tmp/second-key\r\n\tUser third-user";
    exercise(text, |_| {}, text);
}

#[test]
fn include_after_first_user_and_before_repeated_user_stays_in_place() {
    let text = "Host demo\n  User explicit-user\n  Include @INCLUDE@\n  User fallback-user\n";
    exercise(text, |_| {}, text);
}

#[test]
fn changing_a_fallback_field_does_not_move_include_or_match() {
    let text = "Host demo\n  Include @INCLUDE@\n  User fallback-user\nMatch host demo\n  Compression yes\nHost other\n  User other-user\n";
    exercise(
        text,
        |host| host.user = Some("new-fallback".into()),
        &text.replace("User fallback-user", "User new-fallback"),
    );
}

#[test]
fn changing_an_advanced_comment_does_not_move_include() {
    let text = "Host demo\n  # old note\n  Include @INCLUDE@\n  User fallback-user\n  # trailing\n";
    exercise(
        text,
        |host| host.extra = host.extra.replace("old note", "new note"),
        &text.replace("old note", "new note"),
    );
}

#[test]
fn independent_advanced_insertions_and_deletions_preserve_include_anchors() {
    let text = "Host demo\n  # remove\n  Include @INCLUDE@\n  User fallback-user\n  # middle\n  IdentityFile /tmp/second-key\n  # last\n";
    let expected = text
        .replace("  # remove\n", "")
        .replace("  # middle", "  # inserted\n  # middle")
        .replace("  # last", "  # last\n  # appended");
    exercise(
        text,
        |host| {
            host.extra = host
                .extra
                .replace("  # remove\n", "")
                .replace("  # middle", "  # inserted\n  # middle")
                .replace("  # last", "  # last\n  # appended")
        },
        &expected,
    );
}

#[test]
fn adding_a_field_to_an_include_only_block_keeps_include_first() {
    let text = "Host demo\r\n  Include @INCLUDE@";
    exercise(
        text,
        |host| host.user = Some("fallback-user".into()),
        "Host demo\r\n  Include @INCLUDE@\r\n    User fallback-user\r\n",
    );
}

#[test]
fn replacing_advanced_text_keeps_mixed_terminators_and_unterminated_last_line() {
    let text = "Host demo\r\n  Include @INCLUDE@\r\n  User fallback-user\n  # old";
    exercise(
        text,
        |host| host.extra = host.extra.replace("# old", "# new"),
        &text.replace("# old", "# new"),
    );
}

#[test]
fn deleting_advanced_comments_preserves_repeated_directives_in_their_slots() {
    let text = "Host demo\n  # remove\n  Include @INCLUDE@\n  User fallback-user\n  Include @INCLUDE@\n  IdentityFile /tmp/second-key\n  # keep\n";
    exercise(
        text,
        |host| host.extra = host.extra.replace("  # remove\n", ""),
        &text.replace("  # remove\n", ""),
    );
}

#[test]
fn explicit_edits_and_removal_keep_later_fields_and_model_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    let source = "Host demo\r\n  # before\r\n  User first-user\r\n  # middle\n  Port 22\r\n  User second-user\r\n  Port 2200";
    fs::write(&path, source).unwrap();
    let mut config = SshConfig::load_from(&path).unwrap();
    let mut host = config.list_hosts().remove(0);
    host.user = None;
    host.port = Some(2222);
    config.upsert_host(host);
    config.save().unwrap();
    let expected = source
        .replace("  User first-user\r\n", "")
        .replace("  Port 22\r\n", "  Port 2222\r\n");
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    let actual = effective(&path, "demo");
    assert!(actual.lines().any(|line| line == "user second-user"));
    assert!(actual.lines().any(|line| line == "port 2222"));
    let mut host = config.list_hosts().remove(0);
    assert_eq!(host.user.as_deref(), Some("second-user"));
    host.user = Some("third-user".into());
    config.upsert_host(host);
    config.save().unwrap();
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        expected.replace("second-user", "third-user")
    );
}

#[test]
fn advanced_known_directive_is_visible_on_subsequent_edit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    fs::write(&path, "Host demo\n  # before\n  Port 22\n").unwrap();
    let mut config = SshConfig::load_from(&path).unwrap();
    let mut host = config.list_hosts().remove(0);
    host.extra = "  User new-user".into();
    config.upsert_host(host);
    config.save().unwrap();
    let mut host = config.list_hosts().remove(0);
    assert_eq!(host.user.as_deref(), Some("new-user"));
    host.user = Some("edited-user".into());
    config.upsert_host(host);
    config.save().unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "Host demo\n  User edited-user\n  Port 22\n"
    );
    assert!(effective(&path, "demo")
        .lines()
        .any(|line| line == "user edited-user"));
}
