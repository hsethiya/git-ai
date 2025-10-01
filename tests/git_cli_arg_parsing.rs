use git_ai::git::cli_parser::parse_git_cli_args;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn parses_simple_commit() {
    let args = s(&["-C", "..", "commit", "-m", "foo"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-C", ".."]));
    assert_eq!(got.command, Some("commit".into()));
    assert_eq!(got.command_args, s(&["-m", "foo"]));
}

#[test]
fn repeated_dash_c_and_dash_c_sticky() {
    let args = s(&[
        "-c",
        "user.name=alice",
        "-cuser.email=alice@example.com",
        "commit",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&["-c", "user.name=alice", "-cuser.email=alice@example.com"])
    );
    assert_eq!(got.command, Some("commit".into()));
    assert!(got.command_args.is_empty());
}

#[test]
fn long_eq_and_separate_forms() {
    let args = s(&["--git-dir=/x/repo.git", "--work-tree", "/x", "status"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&["--git-dir=/x/repo.git", "--work-tree", "/x"])
    );
    assert_eq!(got.command, Some("status".into()));
}

#[test]
fn meta_version_no_command() {
    let args = s(&["--version"]);
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--version"])); // per requirement
}

#[test]
fn meta_exec_path_with_value_no_command() {
    let args = s(&["--exec-path", "/usr/libexec/git-core"]);
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert_eq!(
        got.command_args,
        s(&["--exec-path", "/usr/libexec/git-core"])
    );
}

#[test]
fn end_of_options_forces_command_even_if_dashy() {
    let args = s(&["-C", ".", "--", "--weird"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-C", "."]));
    assert_eq!(got.command, Some("--weird".into()));
    assert!(got.command_args.is_empty());
}

#[test]
fn unknown_top_level_option_means_no_command() {
    let args = s(&["--totally-unknown", "rest"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--totally-unknown", "rest"]));
}

#[test]
fn multiple_dash_c_and_casing_mixture() {
    let args = s(&[
        "-c",
        "core.filemode=false",
        "-cuser.name=alice",
        "-c",
        "user.email=alice@example.com",
        "status",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&[
            "-c",
            "core.filemode=false",
            "-cuser.name=alice",
            "-c",
            "user.email=alice@example.com"
        ])
    );
    assert_eq!(got.command, Some("status".into()));
    assert!(got.command_args.is_empty());
}

#[test]
fn repeated_dash_c_retained_order() {
    let args = s(&["-c", "a=1", "-c", "a=2", "rev-parse"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-c", "a=1", "-c", "a=2"]));
    assert_eq!(got.command, Some("rev-parse".into()));
}

#[test]
fn mixed_equals_and_separate_for_long_globals() {
    let args = s(&[
        "--git-dir=/x/.git",
        "--work-tree",
        "/x",
        "--namespace=ns",
        "commit",
        "--amend",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&["--git-dir=/x/.git", "--work-tree", "/x", "--namespace=ns"])
    );
    assert_eq!(got.command, Some("commit".into()));
    assert_eq!(got.command_args, s(&["--amend"]));
}

#[test]
fn dash_c_space_and_sticky_variants() {
    let args = s(&["-c", "name=val", "-cname2=val2", "log"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-c", "name=val", "-cname2=val2"]));
    assert_eq!(got.command, Some("log".into()));
}

#[test]
fn config_env_equals_form() {
    let args = s(&[
        "--config-env",
        "http.proxy=HTTP_PROXY",
        "--config-env=core.askpass=GIT_ASKPASS",
        "fetch",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&[
            "--config-env",
            "http.proxy=HTTP_PROXY",
            "--config-env=core.askpass=GIT_ASKPASS"
        ])
    );
    assert_eq!(got.command, Some("fetch".into()));
}

#[test]
fn multiple_dash_c_with_command_args_present() {
    let args = s(&[
        "-c",
        "commit.gpgsign=true",
        "-c",
        "user.signingkey=ABC",
        "commit",
        "-m",
        "msg",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&["-c", "commit.gpgsign=true", "-c", "user.signingkey=ABC"])
    );
    assert_eq!(got.command, Some("commit".into()));
    assert_eq!(got.command_args, s(&["-m", "msg"]));
}

#[test]
fn pathspec_toggles_as_globals() {
    let args = s(&[
        "--literal-pathspecs",
        "--noglob-pathspecs",
        "--icase-pathspecs",
        "ls-files",
        "-z",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&[
            "--literal-pathspecs",
            "--noglob-pathspecs",
            "--icase-pathspecs"
        ])
    );
    assert_eq!(got.command, Some("ls-files".into()));
    assert_eq!(got.command_args, s(&["-z"]));
}

#[test]
fn paginate_and_no_pager_both_present_kept_as_globals() {
    let args = s(&["--paginate", "--no-pager", "log"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["--paginate", "--no-pager"]));
    assert_eq!(got.command, Some("log".into()));
}

#[test]
fn multiple_dash_c_directives_before_end_of_options() {
    let args = s(&["-c", "a=b", "--", "commit", "-m", "x"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-c", "a=b"]));
    assert_eq!(got.command, Some("commit".into()));
    assert_eq!(got.command_args, s(&["-m", "x"]));
}

#[test]
fn dash_dash_forces_command_even_if_dashy() {
    let args = s(&["--", "--help"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.command, Some("--help".into()));
    assert!(got.global_args.is_empty());
    assert!(got.command_args.is_empty());
}

#[test]
fn end_of_options_then_dashy_non_meta_command() {
    let args = s(&["--", "-notarealcmd", "--arg"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.command, Some("-notarealcmd".into()));
    assert_eq!(got.command_args, s(&["--arg"]));
}

#[test]
fn unknown_top_level_option_disables_command_and_passthrough() {
    let args = s(&["--unknown-top", "status", "-s"]);
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--unknown-top", "status", "-s"]));
}

#[test]
fn meta_version_no_command_even_with_extra_flags() {
    let args = s(&["--version", "-v"]);
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--version", "-v"]));
}

#[test]
fn meta_help_no_command() {
    let args = s(&["--help"]);
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--help"]));
}

#[test]
fn meta_exec_path_equals_form_no_command() {
    let args = s(&["--exec-path=/usr/libexec/git-core"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--exec-path=/usr/libexec/git-core"]));
}

#[test]
fn meta_discarded_if_real_command_appears_current_behavior() {
    // Current behavior in parser: pre-command meta is NOT injected when a command appears.
    // `git --help commit` internally acts like `git help commit`, but we do not rewrite.
    let args = s(&["--help", "commit", "-a"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, Vec::<String>::new());
    assert_eq!(got.command, Some("commit".into()));
    assert_eq!(got.command_args, s(&["-a"]));
}

#[test]
fn command_help_is_a_real_command() {
    let args = s(&["help", "-a"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.command, Some("help".into()));
    assert_eq!(got.command_args, s(&["-a"]));
}

#[test]
fn repeated_dash_c_and_multiple_dash_c_with_command_afterwards() {
    let args = s(&[
        "-c",
        "a=1",
        "-c",
        "b=2",
        "-c",
        "c=3",
        "rev-parse",
        "--is-inside-work-tree",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-c", "a=1", "-c", "b=2", "-c", "c=3"]));
    assert_eq!(got.command, Some("rev-parse".into()));
    assert_eq!(got.command_args, s(&["--is-inside-work-tree"]));
}

#[test]
fn multiple_dash_c_and_dash_c_sticky_then_end_of_options_and_weird_command() {
    let args = s(&["-cfoo=bar", "-c", "a=b", "--", "--oddcmd", "arg"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-cfoo=bar", "-c", "a=b"]));
    assert_eq!(got.command, Some("--oddcmd".into()));
    assert_eq!(got.command_args, s(&["arg"]));
}

#[test]
fn dash_c_and_namespace_and_gitdir_and_worktree() {
    let args = s(&[
        "-c",
        "a=b",
        "--namespace=ns",
        "--git-dir",
        "/g",
        "--work-tree=/w",
        "status",
        "--porcelain",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&[
            "-c",
            "a=b",
            "--namespace=ns",
            "--git-dir",
            "/g",
            "--work-tree=/w"
        ])
    );
    assert_eq!(got.command, Some("status".into()));
    assert_eq!(got.command_args, s(&["--porcelain"]));
}

#[test]
fn list_cmds_as_global_takes_value() {
    let args = s(&["--list-cmds=main,others", "status"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["--list-cmds=main,others"]));
    assert_eq!(got.command, Some("status".into()));
}

#[test]
fn super_prefix_and_attr_source_globals() {
    let args = s(&[
        "--super-prefix=foo/",
        "--attr-source",
        "path/to/file",
        "check-attr",
        "crlf",
        "README",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&["--super-prefix=foo/", "--attr-source", "path/to/file"])
    );
    assert_eq!(got.command, Some("check-attr".into()));
    assert_eq!(got.command_args, s(&["crlf", "README"]));
}

#[test]
fn multiple_dash_c_and_bare() {
    let args = s(&[
        "--bare",
        "-c",
        "init.defaultBranch=main",
        "rev-parse",
        "--git-dir",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&["--bare", "-c", "init.defaultBranch=main"])
    );
    assert_eq!(got.command, Some("rev-parse".into()));
    assert_eq!(got.command_args, s(&["--git-dir"]));
}

#[test]
fn sticky_dash_c_then_command() {
    let args = s(&["-cfoo.bar=baz", "status"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-cfoo.bar=baz"]));
    assert_eq!(got.command, Some("status".into()));
}

#[test]
fn sticky_dash_c_then_end_of_options_then_command() {
    let args = s(&["-cfoo.bar=baz", "--", "status", "-s"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-cfoo.bar=baz"]));
    assert_eq!(got.command, Some("status".into()));
    assert_eq!(got.command_args, s(&["-s"]));
}

#[test]
fn sticky_dash_c_and_sticky_dash_c_with_equals_in_value() {
    let args = s(&["-chttp.extraHeader=Authorization: Bearer=XYZ", "fetch"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&["-chttp.extraHeader=Authorization: Bearer=XYZ"])
    );
    assert_eq!(got.command, Some("fetch".into()));
}

#[test]
fn dash_c_then_missing_value_at_end_is_kept_and_no_crash() {
    let args = s(&["-c"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-c"])); // parser keeps it; validation is up to caller
    assert_eq!(got.command, None);
    assert!(got.command_args.is_empty());
}

#[test]
fn dash_c_value_but_no_command() {
    let args = s(&["-c", "a=b"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-c", "a=b"]));
    assert_eq!(got.command, None);
    assert!(got.command_args.is_empty());
}

#[test]
fn dash_c_and_cwd_changes_multiple_C_variants() {
    let args = s(&["-C", ".", "-C/tmp", "-C", "-", "status"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, s(&["-C", ".", "-C/tmp", "-C", "-"]));
    assert_eq!(got.command, Some("status".into()));
}

#[test]
fn meta_info_path_without_command() {
    let args = s(&["--info-path"]);
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--info-path"]));
}

#[test]
fn meta_html_path_then_real_command_meta_is_dropped_current_behavior() {
    let args = s(&["--html-path", "log", "-1"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.global_args, Vec::<String>::new());
    assert_eq!(got.command, Some("log".into()));
    assert_eq!(got.command_args, s(&["-1"]));
}

#[test]
fn no_args_at_all() {
    let args: Vec<String> = vec![];
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert!(got.command_args.is_empty());
}

#[test]
fn command_with_hyphen_in_name() {
    let args = s(&["ls-files", "--stage"]);
    let got = parse_git_cli_args(&args);
    assert_eq!(got.command, Some("ls-files".into()));
    assert_eq!(got.command_args, s(&["--stage"]));
}

#[test]
fn unknown_then_everything_passthrough_even_if_command_like_token_exists() {
    let args = s(&["--mystery", "commit", "-m", "x"]);
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--mystery", "commit", "-m", "x"]));
}

#[test]
fn exec_path_without_value_no_command() {
    let args = s(&["--exec-path"]);
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command, None);
    assert_eq!(got.command_args, s(&["--exec-path"]));
}

#[test]
fn attr_source_and_super_prefix_mixed_with_namespace() {
    let args = s(&[
        "--attr-source=HEAD:/.gitattributes",
        "--super-prefix",
        "sub/",
        "--namespace",
        "foo",
        "check-attr",
        "eol",
        "a.txt",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&[
            "--attr-source=HEAD:/.gitattributes",
            "--super-prefix",
            "sub/",
            "--namespace",
            "foo"
        ])
    );
    assert_eq!(got.command, Some("check-attr".into()));
    assert_eq!(got.command_args, s(&["eol", "a.txt"]));
}

#[test]
fn bare_and_no_optional_locks_and_no_advice_and_no_lazy_fetch() {
    let args = s(&[
        "--bare",
        "--no-optional-locks",
        "--no-advice",
        "--no-lazy-fetch",
        "rev-parse",
        "HEAD",
    ]);
    let got = parse_git_cli_args(&args);
    assert_eq!(
        got.global_args,
        s(&[
            "--bare",
            "--no-optional-locks",
            "--no-advice",
            "--no-lazy-fetch"
        ])
    );
    assert_eq!(got.command, Some("rev-parse".into()));
    assert_eq!(got.command_args, s(&["HEAD"]));
}

#[test]
fn blame_double_dash_then_filename() {
    let args = vec!["blame", "--", "Readme.md"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let got = parse_git_cli_args(&args);
    assert!(got.global_args.is_empty());
    assert_eq!(got.command.as_deref(), Some("blame"));
    assert_eq!(
        got.command_args,
        vec!["--".to_string(), "Readme.md".to_string()]
    );
    assert!(!got.saw_end_of_opts);

    assert_eq!(got.to_invocation_vec(), args);
}

#[test]
fn blame_filename_starts_with_dash() {
    let args = vec!["blame", "--", "--weird"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let got = parse_git_cli_args(&args);
    assert_eq!(got.command.as_deref(), Some("blame"));
    assert_eq!(
        got.command_args,
        vec!["--".to_string(), "--weird".to_string()]
    );
    assert!(!got.saw_end_of_opts);
}

#[test]
fn inverse_with_end_of_opts_roundtrips() {
    let args = vec!["-C", ".", "--", "--weird"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let parsed = parse_git_cli_args(&args);
    assert_eq!(parsed.global_args, vec!["-C".to_string(), ".".to_string()]);
    assert_eq!(parsed.command.as_deref(), Some("--weird"));
    assert!(parsed.saw_end_of_opts);
    assert_eq!(parsed.to_invocation_vec(), args);
}

#[test]
fn inverse_with_end_of_opts_no_command() {
    let args = vec!["-C", ".", "--"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let parsed = parse_git_cli_args(&args);
    assert_eq!(parsed.command, None);
    assert!(parsed.saw_end_of_opts);
    assert_eq!(parsed.to_invocation_vec(), args);
}

#[test]
fn inverse_simple_commit() {
    let args = vec!["-C", "..", "commit", "-m", "foo"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let parsed = parse_git_cli_args(&args);
    assert_eq!(parsed.to_invocation_vec(), args);
}

#[test]
fn inverse_meta_no_command() {
    let args = vec!["--version"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let parsed = parse_git_cli_args(&args);
    // global=[], command=None, command_args=["--version"]
    assert_eq!(parsed.to_invocation_vec(), args);
}

#[test]
fn inverse_unknown_option_passthrough() {
    let args = vec!["--mystery", "status", "-s"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let parsed = parse_git_cli_args(&args);
    assert_eq!(parsed.command, None);
    assert_eq!(parsed.to_invocation_vec(), args);
}

#[test]
fn inverse_end_of_opts_note() {
    let args = vec!["-C", ".", "--", "--weird"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    let parsed = parse_git_cli_args(&args);
    assert_eq!(parsed.global_args, vec!["-C".to_string(), ".".to_string()]);
    assert_eq!(parsed.command.as_deref(), Some("--weird"));
    assert_eq!(parsed.command_args, Vec::<String>::new());
    assert_eq!(
        parsed.to_invocation_vec(),
        vec![
            "-C".to_string(),
            ".".to_string(),
            "--".to_string(),
            "--weird".to_string()
        ]
    );
}
