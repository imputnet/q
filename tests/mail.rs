mod common;
use common::*;

#[test]
fn mail_writes_basic_mbox() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();

    q(dir.path())
        .env("LC_ALL", "C.UTF-8")
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--prefix",
            "test",
            "--sender",
            "dev@example.com",
            "--subject",
            "Patch series",
            "-m",
            "Intro",
        ])
        .assert()
        .success();
    let mbox = fs::read_to_string(dir.path().join("mbox")).unwrap();
    assert!(mbox.contains("Message-ID: <"));
    assert!(mbox.contains("User-Agent: quilt/"));
    assert!(mbox.contains("Date: "));
    assert!(mbox.contains("MIME-Version: 1.0"));
    assert!(mbox.contains("Content-Type: text/plain; charset=UTF-8"));
    assert!(mbox.contains("Subject: [test 0/1] Patch series"));
    assert!(mbox.contains("Subject: [test 1/1] Fix thing"));
}

#[cfg(unix)]
#[test]
fn mail_send_invokes_configured_sendmail() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();
    let sendmail = dir.path().join("sendmail");
    let log = dir.path().join("sendmail.log");
    let body = dir.path().join("sendmail.body");
    fs::write(
        &sendmail,
        format!(
            "#!/bin/sh\nprintf 'args:%s\\n' \"$*\" >> '{}'\nprintf '%s\\n' '---message---' >> '{}'\ncat >> '{}'\n",
            log.display(),
            body.display(),
            body.display()
        ),
    )
    .unwrap();
    let mut perms = fs::metadata(&sendmail).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&sendmail, perms).unwrap();

    q(dir.path())
        .env("QUILT_SENDMAIL", sendmail.to_str().unwrap())
        .args([
            "mail",
            "--send",
            "--sender",
            "alice@example.com",
            "--to",
            "dev@example.com",
            "--cc",
            "review@example.com",
            "--bcc",
            "hidden@example.com",
            "--subject",
            "Patch series",
            "-m",
            "Intro",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "-f alice@example.com dev@example.com review@example.com hidden@example.com",
        ));

    let log = fs::read_to_string(log).unwrap();
    assert!(log.contains(
        "args:-f alice@example.com dev@example.com review@example.com hidden@example.com"
    ));
    let body = fs::read_to_string(body).unwrap();
    assert!(body.contains("Subject: [patch 0/1] Patch series"));
    assert!(body.contains("Subject: [patch 1/1] Fix thing"));
    assert!(!body.contains("Bcc:"));
}

#[test]
fn mail_reply_to_adds_thread_headers() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("reply.eml"),
        "Message-ID: <base@example.com>\nReferences: <root@example.com>\nSubject: Re: Original topic\n\nBody\n",
    )
    .unwrap();

    q(dir.path())
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--sender",
            "alice@example.com",
            "--to",
            "dev@example.com",
            "--reply-to",
            "reply.eml",
            "-m",
            "Intro",
        ])
        .assert()
        .success();

    let mbox = fs::read_to_string(dir.path().join("mbox")).unwrap();
    assert!(mbox.contains("In-Reply-To: <base@example.com>\n"));
    assert!(mbox.contains("References: <root@example.com>\n <base@example.com>\n"));
    assert!(mbox.contains("Subject: [patch 0/1] Re: Original topic"));
}

#[test]
fn mail_charset_sets_patch_content_type() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();

    q(dir.path())
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--sender",
            "alice@example.com",
            "--to",
            "dev@example.com",
            "--subject",
            "Patch series",
            "--charset",
            "ISO-8859-1",
            "-m",
            "Intro",
        ])
        .assert()
        .success();

    let mbox = fs::read_to_string(dir.path().join("mbox")).unwrap();
    assert!(mbox.contains("Content-Type: text/plain; charset=ISO-8859-1"));
}

#[test]
fn mail_honors_patch_range_arguments() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(
        dir.path().join("patches/series"),
        "one.diff\ntwo.diff\nthree.diff\n",
    )
    .unwrap();
    for name in ["one", "two", "three"] {
        fs::write(
            dir.path().join(format!("patches/{name}.diff")),
            format!("Subject: {name}\n\n--- a/{name}\n+++ b/{name}\n"),
        )
        .unwrap();
    }

    q(dir.path())
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--sender",
            "alice@example.com",
            "--to",
            "dev@example.com",
            "--subject",
            "Range",
            "-m",
            "Intro",
            "two.diff",
            "three.diff",
        ])
        .assert()
        .success();

    let mbox = fs::read_to_string(dir.path().join("mbox")).unwrap();
    assert!(mbox.contains("Subject: [patch 0/2] Range"));
    assert!(!mbox.contains("Subject: [patch 1/2] one"));
    assert!(mbox.contains("Subject: [patch 1/2] two"));
    assert!(mbox.contains("Subject: [patch 2/2] three"));
}

#[test]
fn mail_uses_home_signature_by_default() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("home")).unwrap();
    fs::write(dir.path().join("home/.signature"), "Default sig\n").unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();

    q(dir.path())
        .env("HOME", dir.path().join("home"))
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--sender",
            "alice@example.com",
            "--to",
            "dev@example.com",
            "--subject",
            "Patch series",
            "-m",
            "Intro",
        ])
        .assert()
        .success();

    let mbox = fs::read_to_string(dir.path().join("mbox")).unwrap();
    assert!(mbox.contains("-- \nDefault sig\n"));
}

#[test]
fn mail_signature_dash_suppresses_signature() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("home")).unwrap();
    fs::write(dir.path().join("home/.signature"), "Default sig\n").unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();

    q(dir.path())
        .env("HOME", dir.path().join("home"))
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--sender",
            "alice@example.com",
            "--to",
            "dev@example.com",
            "--subject",
            "Patch series",
            "--signature",
            "-",
            "-m",
            "Intro",
        ])
        .assert()
        .success();

    let mbox = fs::read_to_string(dir.path().join("mbox")).unwrap();
    assert!(!mbox.contains("Default sig"));
    assert!(!mbox.contains("-- \n"));
}

#[test]
fn mail_rejects_duplicate_patch_subjects() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\ntwo.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix same thing\n\n--- a/one\n+++ b/one\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("patches/two.diff"),
        "Subject: [PATCH] Fix same thing\n\n--- a/two\n+++ b/two\n",
    )
    .unwrap();

    q(dir.path())
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--sender",
            "alice@example.com",
            "--subject",
            "Series",
            "-m",
            "Intro",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Patches one.diff, two.diff have duplicate subject headers.",
        ));
}

#[test]
fn mail_reply_to_without_subject_requires_intro_subject() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("reply.eml"),
        "Message-ID: <base@example.com>\n\nBody\n",
    )
    .unwrap();

    q(dir.path())
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--sender",
            "alice@example.com",
            "--reply-to",
            "reply.eml",
            "-m",
            "Intro without subject",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Introduction has no subject header",
        ));
}

#[test]
fn mail_args_from_environment_affect_mbox_headers() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();
    fs::write(dir.path().join("sig"), "Regards\n").unwrap();

    q(dir.path())
        .env(
            "QUILT_MAIL_ARGS",
            "--sender 'Alice <alice@example.com>' --from 'Bob <bob@example.com>' --to dev@example.com --cc review@example.com --signature sig",
        )
        .args(["mail", "--mbox", "mbox", "--subject", "Patch series", "-m", "Intro"])
        .assert()
        .success();

    let mbox = fs::read_to_string(dir.path().join("mbox")).unwrap();
    assert!(mbox.contains("From alice@example.com "));
    assert!(mbox.contains("From: Bob <bob@example.com>"));
    assert!(mbox.contains("To: dev@example.com"));
    assert!(mbox.contains("Cc: review@example.com"));
    assert!(mbox.contains("-- \nRegards\n"));
}

#[test]
fn mail_rejects_multiple_intro_sources() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();
    fs::write(dir.path().join("intro"), "Subject: Intro\n").unwrap();

    q(dir.path())
        .args([
            "mail",
            "--mbox",
            "mbox",
            "-m",
            "Subject: Intro",
            "-M",
            "intro",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Introduction message already specified",
        ));
}

#[test]
fn mail_uses_locale_default_charset_and_recipient_tags() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "one.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/one.diff"),
        "Subject: Fix thing\nreviewed-by: Reviewer <review@example.com>\nTested-by: Tester <test@example.com>\n\n--- a/file\n+++ b/file\n",
    )
    .unwrap();

    q(dir.path())
        .env("LC_ALL", "C")
        .env_remove("LANG")
        .args([
            "mail",
            "--mbox",
            "mbox",
            "--sender",
            "dev@example.com",
            "--subject",
            "Patch series",
            "-m",
            "Intro",
        ])
        .assert()
        .success();

    let mbox = fs::read_to_string(dir.path().join("mbox")).unwrap();
    assert!(mbox.contains("Content-Type: text/plain; charset=ISO-8859-15"));
    assert!(mbox.contains("Cc: Reviewer <review@example.com>,\n Tester <test@example.com>"));
}
