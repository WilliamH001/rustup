use crate::common;
use crate::errors::*;
use crate::help::*;
use crate::self_update;
use crate::term2;
use clap::{App, AppSettings, Arg, ArgGroup, ArgMatches, Shell, SubCommand};
use rustup::{command, Cfg, Toolchain};
use rustup_dist::dist::{PartialTargetTriple, PartialToolchainDesc, TargetTriple};
use rustup_dist::manifest::Component;
use rustup_utils::utils::{self, ExitCode};
use std::error::Error;
use std::io::{self, Write};
use std::iter;
use std::path::Path;
use std::process::{self, Command};

fn handle_epipe(res: Result<()>) -> Result<()> {
    match res {
        Err(Error(ErrorKind::Io(ref err), _)) if err.kind() == std::io::ErrorKind::BrokenPipe => {
            Ok(())
        }
        res => res,
    }
}

pub fn main() -> Result<()> {
    crate::self_update::cleanup_self_updater()?;

    let ref matches = cli().get_matches();
    let verbose = matches.is_present("verbose");
    let ref cfg = common::set_globals(verbose)?;

    if maybe_upgrade_data(cfg, matches)? {
        return Ok(());
    }

    cfg.check_metadata_version()?;

    match matches.subcommand() {
        ("show", Some(c)) => match c.subcommand() {
            ("active-toolchain", Some(_)) => handle_epipe(show_active_toolchain(cfg))?,
            (_, _) => handle_epipe(show(cfg))?,
        },
        ("install", Some(m)) => update(cfg, m)?,
        ("update", Some(m)) => update(cfg, m)?,
        ("uninstall", Some(m)) => toolchain_remove(cfg, m)?,
        ("default", Some(m)) => default_(cfg, m)?,
        ("toolchain", Some(c)) => match c.subcommand() {
            ("install", Some(m)) => update(cfg, m)?,
            ("list", Some(_)) => common::list_toolchains(cfg)?,
            ("link", Some(m)) => toolchain_link(cfg, m)?,
            ("uninstall", Some(m)) => toolchain_remove(cfg, m)?,
            (_, _) => unreachable!(),
        },
        ("target", Some(c)) => match c.subcommand() {
            ("list", Some(m)) => target_list(cfg, m)?,
            ("add", Some(m)) => target_add(cfg, m)?,
            ("remove", Some(m)) => target_remove(cfg, m)?,
            (_, _) => unreachable!(),
        },
        ("component", Some(c)) => match c.subcommand() {
            ("list", Some(m)) => component_list(cfg, m)?,
            ("add", Some(m)) => component_add(cfg, m)?,
            ("remove", Some(m)) => component_remove(cfg, m)?,
            (_, _) => unreachable!(),
        },
        ("override", Some(c)) => match c.subcommand() {
            ("list", Some(_)) => common::list_overrides(cfg)?,
            ("set", Some(m)) => override_add(cfg, m)?,
            ("unset", Some(m)) => override_remove(cfg, m)?,
            (_, _) => unreachable!(),
        },
        ("run", Some(m)) => run(cfg, m)?,
        ("which", Some(m)) => which(cfg, m)?,
        ("doc", Some(m)) => doc(cfg, m)?,
        ("man", Some(m)) => man(cfg, m)?,
        ("self", Some(c)) => match c.subcommand() {
            ("update", Some(_)) => self_update::update()?,
            ("uninstall", Some(m)) => self_uninstall(m)?,
            (_, _) => unreachable!(),
        },
        ("set", Some(c)) => match c.subcommand() {
            ("default-host", Some(m)) => set_default_host_triple(&cfg, m)?,
            (_, _) => unreachable!(),
        },
        ("completions", Some(c)) => {
            if let Some(shell) = c.value_of("shell") {
                cli().gen_completions_to(
                    "rustup",
                    shell.parse::<Shell>().unwrap(),
                    &mut io::stdout(),
                );
            }
        }
        (_, _) => unreachable!(),
    }

    Ok(())
}

pub fn cli() -> App<'static, 'static> {
    let mut app = App::new("rustup")
        .version(common::version())
        .about("The Rust toolchain installer")
        .after_help(RUSTUP_HELP)
        .setting(AppSettings::VersionlessSubcommands)
        .setting(AppSettings::DeriveDisplayOrder)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .arg(
            Arg::with_name("verbose")
                .help("Enable verbose output")
                .short("v")
                .long("verbose"),
        )
        .subcommand(
            SubCommand::with_name("show")
                .about("Show the active and installed toolchains")
                .after_help(SHOW_HELP)
                .setting(AppSettings::VersionlessSubcommands)
                .setting(AppSettings::DeriveDisplayOrder)
                .subcommand(
                    SubCommand::with_name("active-toolchain")
                        .about("Show the active toolchain")
                        .after_help(SHOW_ACTIVE_TOOLCHAIN_HELP),
                ),
        )
        .subcommand(
            SubCommand::with_name("install")
                .about("Update Rust toolchains")
                .after_help(INSTALL_HELP)
                .setting(AppSettings::Hidden) // synonym for 'toolchain install'
                .arg(
                    Arg::with_name("toolchain")
                        .help(TOOLCHAIN_ARG_HELP)
                        .required(true)
                        .multiple(true),
                )
                .arg(
                    Arg::with_name("no-self-update")
                        .help("Don't perform self update when running the `rustup` command")
                        .long("no-self-update")
                        .takes_value(false)
                        .hidden(true),
                )
                .arg(
                    Arg::with_name("force")
                        .help("Force an update, even if some components are missing")
                        .long("force")
                        .takes_value(false),
                ),
        )
        .subcommand(
            SubCommand::with_name("uninstall")
                .about("Uninstall Rust toolchains")
                .setting(AppSettings::Hidden) // synonym for 'toolchain uninstall'
                .arg(
                    Arg::with_name("toolchain")
                        .help(TOOLCHAIN_ARG_HELP)
                        .required(true)
                        .multiple(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("update")
                .about("Update Rust toolchains and rustup")
                .after_help(UPDATE_HELP)
                .arg(
                    Arg::with_name("toolchain")
                        .help(TOOLCHAIN_ARG_HELP)
                        .required(false)
                        .multiple(true),
                )
                .arg(
                    Arg::with_name("no-self-update")
                        .help("Don't perform self update when running the `rustup` command")
                        .long("no-self-update")
                        .takes_value(false)
                        .hidden(true),
                )
                .arg(
                    Arg::with_name("force")
                        .help("Force an update, even if some components are missing")
                        .long("force")
                        .takes_value(false),
                ),
        )
        .subcommand(
            SubCommand::with_name("default")
                .about("Set the default toolchain")
                .after_help(DEFAULT_HELP)
                .arg(
                    Arg::with_name("toolchain")
                        .help(TOOLCHAIN_ARG_HELP)
                        .required(false),
                ),
        )
        .subcommand(
            SubCommand::with_name("toolchain")
                .about("Modify or query the installed toolchains")
                .after_help(TOOLCHAIN_HELP)
                .setting(AppSettings::VersionlessSubcommands)
                .setting(AppSettings::DeriveDisplayOrder)
                .setting(AppSettings::SubcommandRequiredElseHelp)
                .subcommand(SubCommand::with_name("list").about("List installed toolchains"))
                .subcommand(
                    SubCommand::with_name("install")
                        .about("Install or update a given toolchain")
                        .aliases(&["update", "add"])
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .required(true)
                                .multiple(true),
                        )
                        .arg(
                            Arg::with_name("no-self-update")
                                .help("Don't perform self update when running the `rustup` command")
                                .long("no-self-update")
                                .takes_value(false)
                                .hidden(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("uninstall")
                        .about("Uninstall a toolchain")
                        .alias("remove")
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .required(true)
                                .multiple(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("link")
                        .about("Create a custom toolchain by symlinking to a directory")
                        .after_help(TOOLCHAIN_LINK_HELP)
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .required(true),
                        )
                        .arg(Arg::with_name("path").required(true)),
                ),
        )
        .subcommand(
            SubCommand::with_name("target")
                .about("Modify a toolchain's supported targets")
                .setting(AppSettings::VersionlessSubcommands)
                .setting(AppSettings::DeriveDisplayOrder)
                .setting(AppSettings::SubcommandRequiredElseHelp)
                .subcommand(
                    SubCommand::with_name("list")
                        .about("List installed and available targets")
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .long("toolchain")
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("add")
                        .about("Add a target to a Rust toolchain")
                        .alias("install")
                        .arg(Arg::with_name("target").required(true).multiple(true))
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .long("toolchain")
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("remove")
                        .about("Remove a target from a Rust toolchain")
                        .alias("uninstall")
                        .arg(Arg::with_name("target").required(true).multiple(true))
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .long("toolchain")
                                .takes_value(true),
                        ),
                ),
        )
        .subcommand(
            SubCommand::with_name("component")
                .about("Modify a toolchain's installed components")
                .setting(AppSettings::VersionlessSubcommands)
                .setting(AppSettings::DeriveDisplayOrder)
                .setting(AppSettings::SubcommandRequiredElseHelp)
                .subcommand(
                    SubCommand::with_name("list")
                        .about("List installed and available components")
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .long("toolchain")
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("add")
                        .about("Add a component to a Rust toolchain")
                        .arg(Arg::with_name("component").required(true).multiple(true))
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .long("toolchain")
                                .takes_value(true),
                        )
                        .arg(Arg::with_name("target").long("target").takes_value(true)),
                )
                .subcommand(
                    SubCommand::with_name("remove")
                        .about("Remove a component from a Rust toolchain")
                        .arg(Arg::with_name("component").required(true).multiple(true))
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .long("toolchain")
                                .takes_value(true),
                        )
                        .arg(Arg::with_name("target").long("target").takes_value(true)),
                ),
        )
        .subcommand(
            SubCommand::with_name("override")
                .about("Modify directory toolchain overrides")
                .after_help(OVERRIDE_HELP)
                .setting(AppSettings::VersionlessSubcommands)
                .setting(AppSettings::DeriveDisplayOrder)
                .setting(AppSettings::SubcommandRequiredElseHelp)
                .subcommand(
                    SubCommand::with_name("list").about("List directory toolchain overrides"),
                )
                .subcommand(
                    SubCommand::with_name("set")
                        .about("Set the override toolchain for a directory")
                        .alias("add")
                        .arg(
                            Arg::with_name("toolchain")
                                .help(TOOLCHAIN_ARG_HELP)
                                .required(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("unset")
                        .about("Remove the override toolchain for a directory")
                        .after_help(OVERRIDE_UNSET_HELP)
                        .alias("remove")
                        .arg(
                            Arg::with_name("path")
                                .long("path")
                                .takes_value(true)
                                .help("Path to the directory"),
                        )
                        .arg(
                            Arg::with_name("nonexistent")
                                .long("nonexistent")
                                .takes_value(false)
                                .help("Remove override toolchain for all nonexistent directories"),
                        ),
                ),
        )
        .subcommand(
            SubCommand::with_name("run")
                .about("Run a command with an environment configured for a given toolchain")
                .after_help(RUN_HELP)
                .setting(AppSettings::TrailingVarArg)
                .arg(
                    Arg::with_name("install")
                        .help("Install the requested toolchain if needed")
                        .long("install"),
                )
                .arg(
                    Arg::with_name("toolchain")
                        .help(TOOLCHAIN_ARG_HELP)
                        .required(true),
                )
                .arg(
                    Arg::with_name("command")
                        .required(true)
                        .multiple(true)
                        .use_delimiter(false),
                ),
        )
        .subcommand(
            SubCommand::with_name("which")
                .about("Display which binary will be run for a given command")
                .arg(Arg::with_name("command").required(true)),
        )
        .subcommand(
            SubCommand::with_name("doc")
                .alias("docs")
                .about("Open the documentation for the current toolchain")
                .after_help(DOC_HELP)
                .arg(
                    Arg::with_name("path")
                        .long("path")
                        .help("Only print the path to the documentation"),
                )
                .args(
                    &DOCS_DATA
                        .into_iter()
                        .map(|(name, help_msg, _)| Arg::with_name(name).long(name).help(help_msg))
                        .collect::<Vec<_>>(),
                )
                .arg(
                    Arg::with_name("toolchain")
                        .help(TOOLCHAIN_ARG_HELP)
                        .long("toolchain")
                        .takes_value(true),
                )
                .group(
                    ArgGroup::with_name("page").args(
                        &DOCS_DATA
                            .into_iter()
                            .map(|(name, _, _)| *name)
                            .collect::<Vec<_>>(),
                    ),
                ),
        );

    if cfg!(not(target_os = "windows")) {
        app = app.subcommand(
            SubCommand::with_name("man")
                .about("View the man page for a given command")
                .arg(Arg::with_name("command").required(true))
                .arg(
                    Arg::with_name("toolchain")
                        .help(TOOLCHAIN_ARG_HELP)
                        .long("toolchain")
                        .takes_value(true),
                ),
        );
    }

    app.subcommand(
        SubCommand::with_name("self")
            .about("Modify the rustup installation")
            .setting(AppSettings::VersionlessSubcommands)
            .setting(AppSettings::DeriveDisplayOrder)
            .setting(AppSettings::SubcommandRequiredElseHelp)
            .subcommand(
                SubCommand::with_name("update").about("Download and install updates to rustup"),
            )
            .subcommand(
                SubCommand::with_name("uninstall")
                    .about("Uninstall rustup.")
                    .arg(Arg::with_name("no-prompt").short("y")),
            )
            .subcommand(
                SubCommand::with_name("upgrade-data").about("Upgrade the internal data format."),
            ),
    )
    .subcommand(
        SubCommand::with_name("set")
            .about("Alter rustup settings")
            .setting(AppSettings::SubcommandRequiredElseHelp)
            .subcommand(
                SubCommand::with_name("default-host")
                    .about("The triple used to identify toolchains when not specified")
                    .arg(Arg::with_name("host_triple").required(true)),
            ),
    )
    .subcommand(
        SubCommand::with_name("completions")
            .about("Generate completion scripts for your shell")
            .after_help(COMPLETIONS_HELP)
            .setting(AppSettings::ArgRequiredElseHelp)
            .arg(Arg::with_name("shell").possible_values(&Shell::variants())),
    )
}

fn maybe_upgrade_data(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<bool> {
    match m.subcommand() {
        ("self", Some(c)) => match c.subcommand() {
            ("upgrade-data", Some(_)) => {
                cfg.upgrade_data()?;
                Ok(true)
            }
            _ => Ok(false),
        },
        _ => Ok(false),
    }
}

fn update_bare_triple_check(cfg: &Cfg, name: &str) -> Result<()> {
    if let Some(triple) = PartialTargetTriple::from_str(name) {
        warn!("(partial) target triple specified instead of toolchain name");
        let installed_toolchains = cfg.list_toolchains()?;
        let default = cfg.find_default()?;
        let default_name = default.map(|t| t.name().to_string()).unwrap_or("".into());
        let mut candidates = vec![];
        for t in installed_toolchains {
            if t == default_name {
                continue;
            }
            if let Ok(desc) = PartialToolchainDesc::from_str(&t) {
                fn triple_comp_eq(given: &str, from_desc: Option<&String>) -> bool {
                    from_desc.map_or(false, |s| *s == *given)
                }

                let triple_matches = triple
                    .arch
                    .as_ref()
                    .map_or(true, |s| triple_comp_eq(s, desc.target.arch.as_ref()))
                    && triple
                        .os
                        .as_ref()
                        .map_or(true, |s| triple_comp_eq(s, desc.target.os.as_ref()))
                    && triple
                        .env
                        .as_ref()
                        .map_or(true, |s| triple_comp_eq(s, desc.target.env.as_ref()));
                if triple_matches {
                    candidates.push(t);
                }
            }
        }
        match candidates.len() {
            0 => err!("no candidate toolchains found"),
            1 => println!("\nyou may use the following toolchain: {}\n", candidates[0]),
            _ => {
                println!("\nyou may use one of the following toolchains:");
                for n in &candidates {
                    println!("{}", n);
                }
                println!();
            }
        }
        return Err(ErrorKind::ToolchainNotInstalled(name.to_string()).into());
    }
    Ok(())
}

fn default_bare_triple_check(cfg: &Cfg, name: &str) -> Result<()> {
    if let Some(triple) = PartialTargetTriple::from_str(name) {
        warn!("(partial) target triple specified instead of toolchain name");
        let default = cfg.find_default()?;
        let default_name = default.map(|t| t.name().to_string()).unwrap_or("".into());
        if let Ok(mut desc) = PartialToolchainDesc::from_str(&default_name) {
            desc.target = triple;
            let maybe_toolchain = format!("{}", desc);
            let ref toolchain = cfg.get_toolchain(maybe_toolchain.as_ref(), false)?;
            if toolchain.name() == default_name {
                warn!(
                    "(partial) triple '{}' resolves to a toolchain that is already default",
                    name
                );
            } else {
                println!(
                    "\nyou may use the following toolchain: {}\n",
                    toolchain.name()
                );
            }
            return Err(ErrorKind::ToolchainNotInstalled(name.to_string()).into());
        }
    }
    Ok(())
}

fn default_(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    if m.is_present("toolchain") {
        let ref toolchain = m.value_of("toolchain").expect("");
        default_bare_triple_check(cfg, toolchain)?;
        let ref toolchain = cfg.get_toolchain(toolchain, false)?;

        let status = if !toolchain.is_custom() {
            Some(toolchain.install_from_dist_if_not_installed()?)
        } else if !toolchain.exists() {
            return Err(ErrorKind::ToolchainNotInstalled(toolchain.name().to_string()).into());
        } else {
            None
        };

        toolchain.make_default()?;

        if let Some(status) = status {
            println!();
            common::show_channel_update(cfg, toolchain.name(), Ok(status))?;
        }
    } else {
        println!("{} (default)", cfg.get_default()?);
    }

    Ok(())
}

fn update(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let self_update = !m.is_present("no-self-update") && !self_update::NEVER_SELF_UPDATE;
    if let Some(names) = m.values_of("toolchain") {
        for name in names {
            update_bare_triple_check(cfg, name)?;
            let toolchain = cfg.get_toolchain(name, false)?;

            let status = if !toolchain.is_custom() {
                Some(toolchain.install_from_dist(m.is_present("force"))?)
            } else if !toolchain.exists() {
                return Err(ErrorKind::InvalidToolchainName(toolchain.name().to_string()).into());
            } else {
                None
            };

            if let Some(status) = status {
                println!();
                common::show_channel_update(cfg, toolchain.name(), Ok(status))?;
            }
        }
        if self_update {
            common::self_update(|| Ok(()))?;
        }
    } else {
        common::update_all_channels(cfg, self_update, m.is_present("force"))?;
    }

    Ok(())
}

fn run(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let ref toolchain = m.value_of("toolchain").expect("");
    let args = m.values_of("command").unwrap();
    let args: Vec<_> = args.collect();
    let cmd = cfg.create_command_for_toolchain(toolchain, m.is_present("install"), args[0])?;

    let ExitCode(c) = command::run_command_for_dir(cmd, args[0], &args[1..])?;

    process::exit(c)
}

fn which(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let binary = m.value_of("command").expect("");

    let binary_path = cfg
        .which_binary(&utils::current_dir()?, binary)?
        .expect("binary not found");

    utils::assert_is_file(&binary_path)?;

    println!("{}", binary_path.display());

    Ok(())
}

fn show(cfg: &Cfg) -> Result<()> {
    // Print host triple
    {
        let mut t = term2::stdout();
        t.attr(term2::Attr::Bold)?;
        write!(t, "Default host: ")?;
        t.reset()?;
        writeln!(t, "{}", cfg.get_default_host_triple()?)?;
        writeln!(t)?;
    }

    let ref cwd = utils::current_dir()?;
    let installed_toolchains = cfg.list_toolchains()?;
    let active_toolchain = cfg.find_override_toolchain_or_default(cwd);

    // active_toolchain will carry the reason we don't have one in its detail.
    let active_targets = if let Ok(ref at) = active_toolchain {
        if let Some((ref t, _)) = *at {
            match t.list_components() {
                Ok(cs_vec) => cs_vec
                    .into_iter()
                    .filter(|c| c.component.short_name_in_manifest() == "rust-std")
                    .filter(|c| c.installed)
                    .collect(),
                Err(_) => vec![],
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let show_installed_toolchains = installed_toolchains.len() > 1;
    let show_active_targets = active_targets.len() > 1;
    let show_active_toolchain = true;

    // Only need to display headers if we have multiple sections
    let show_headers = [
        show_installed_toolchains,
        show_active_targets,
        show_active_toolchain,
    ]
    .iter()
    .filter(|x| **x)
    .count()
        > 1;

    if show_installed_toolchains {
        let mut t = term2::stdout();
        if show_headers {
            print_header(&mut t, "installed toolchains")?;
        }
        let default_name = cfg.get_default()?;
        for it in installed_toolchains {
            if default_name == it {
                writeln!(t, "{} (default)", it)?;
            } else {
                writeln!(t, "{}", it)?;
            }
        }
        if show_headers {
            writeln!(t)?
        };
    }

    if show_active_targets {
        let mut t = term2::stdout();
        if show_headers {
            print_header(&mut t, "installed targets for active toolchain")?;
        }
        for at in active_targets {
            writeln!(
                t,
                "{}",
                at.component
                    .target
                    .as_ref()
                    .expect("rust-std should have a target")
            )?;
        }
        if show_headers {
            writeln!(t)?;
        };
    }

    if show_active_toolchain {
        let mut t = term2::stdout();
        if show_headers {
            print_header(&mut t, "active toolchain")?;
        }

        match active_toolchain {
            Ok(atc) => match atc {
                Some((ref toolchain, Some(ref reason))) => {
                    writeln!(t, "{} ({})", toolchain.name(), reason)?;
                    writeln!(t, "{}", common::rustc_version(toolchain))?;
                }
                Some((ref toolchain, None)) => {
                    writeln!(t, "{} (default)", toolchain.name())?;
                    writeln!(t, "{}", common::rustc_version(toolchain))?;
                }
                None => {
                    writeln!(t, "no active toolchain")?;
                }
            },
            Err(err) => {
                if let Some(cause) = err.source() {
                    writeln!(t, "(error: {}, {})", err, cause)?;
                } else {
                    writeln!(t, "(error: {})", err)?;
                }
            }
        }

        if show_headers {
            writeln!(t)?
        }
    }

    fn print_header(t: &mut term2::Terminal<std::io::Stdout>, s: &str) -> Result<()> {
        t.attr(term2::Attr::Bold)?;
        writeln!(t, "{}", s)?;
        writeln!(t, "{}", iter::repeat("-").take(s.len()).collect::<String>())?;
        writeln!(t)?;
        t.reset()?;
        Ok(())
    }

    Ok(())
}

fn show_active_toolchain(cfg: &Cfg) -> Result<()> {
    let ref cwd = utils::current_dir()?;
    if let Some((toolchain, reason)) = cfg.find_override_toolchain_or_default(cwd)? {
        if reason.is_some() {
            println!("{} ({})", toolchain.name(), reason.unwrap());
        } else {
            println!("{} (default)", toolchain.name());
        }
    }
    Ok(())
}

fn target_list(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let toolchain = explicit_or_dir_toolchain(cfg, m)?;

    common::list_targets(&toolchain)
}

fn target_add(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let toolchain = explicit_or_dir_toolchain(cfg, m)?;

    for target in m.values_of("target").expect("") {
        let new_component =
            Component::new("rust-std".to_string(), Some(TargetTriple::from_str(target)));

        toolchain.add_component(new_component)?;
    }

    Ok(())
}

fn target_remove(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let toolchain = explicit_or_dir_toolchain(cfg, m)?;

    for target in m.values_of("target").expect("") {
        let new_component =
            Component::new("rust-std".to_string(), Some(TargetTriple::from_str(target)));

        toolchain.remove_component(new_component)?;
    }

    Ok(())
}

fn component_list(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let toolchain = explicit_or_dir_toolchain(cfg, m)?;

    common::list_components(&toolchain)
}

fn component_add(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let toolchain = explicit_or_dir_toolchain(cfg, m)?;
    let target = m
        .value_of("target")
        .map(TargetTriple::from_str)
        .or_else(|| {
            toolchain
                .desc()
                .as_ref()
                .ok()
                .map(|desc| desc.target.clone())
        });

    for component in m.values_of("component").expect("") {
        let new_component = Component::new(component.to_string(), target.clone());

        toolchain.add_component(new_component)?;
    }

    Ok(())
}

fn component_remove(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let toolchain = explicit_or_dir_toolchain(cfg, m)?;
    let target = m
        .value_of("target")
        .map(TargetTriple::from_str)
        .or_else(|| {
            toolchain
                .desc()
                .as_ref()
                .ok()
                .map(|desc| desc.target.clone())
        });

    for component in m.values_of("component").expect("") {
        let new_component = Component::new(component.to_string(), target.clone());

        toolchain.remove_component(new_component)?;
    }

    Ok(())
}

fn explicit_or_dir_toolchain<'a>(cfg: &'a Cfg, m: &ArgMatches<'_>) -> Result<Toolchain<'a>> {
    let toolchain = m.value_of("toolchain");
    if let Some(toolchain) = toolchain {
        let toolchain = cfg.get_toolchain(toolchain, false)?;
        return Ok(toolchain);
    }

    let ref cwd = utils::current_dir()?;
    let (toolchain, _) = cfg.toolchain_for_dir(cwd)?;

    Ok(toolchain)
}

fn toolchain_link(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let ref toolchain = m.value_of("toolchain").expect("");
    let ref path = m.value_of("path").expect("");
    let toolchain = cfg.get_toolchain(toolchain, true)?;

    Ok(toolchain.install_from_dir(Path::new(path), true)?)
}

fn toolchain_remove(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    for toolchain in m.values_of("toolchain").expect("") {
        let toolchain = cfg.get_toolchain(toolchain, false)?;
        toolchain.remove()?;
    }
    Ok(())
}

fn override_add(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let ref toolchain = m.value_of("toolchain").expect("");
    let toolchain = cfg.get_toolchain(toolchain, false)?;

    let status = if !toolchain.is_custom() {
        Some(toolchain.install_from_dist_if_not_installed()?)
    } else if !toolchain.exists() {
        return Err(ErrorKind::ToolchainNotInstalled(toolchain.name().to_string()).into());
    } else {
        None
    };

    toolchain.make_override(&utils::current_dir()?)?;

    if let Some(status) = status {
        println!();
        common::show_channel_update(cfg, toolchain.name(), Ok(status))?;
    }

    Ok(())
}

fn override_remove(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let paths = if m.is_present("nonexistent") {
        let list: Vec<_> = cfg.settings_file.with(|s| {
            Ok(s.overrides
                .iter()
                .filter_map(|(k, _)| {
                    if Path::new(k).is_dir() {
                        None
                    } else {
                        Some(k.clone())
                    }
                })
                .collect())
        })?;
        if list.is_empty() {
            info!("no nonexistent paths detected");
        }
        list
    } else {
        if m.is_present("path") {
            vec![m.value_of("path").unwrap().to_string()]
        } else {
            vec![utils::current_dir()?.to_str().unwrap().to_string()]
        }
    };

    for path in paths {
        if cfg
            .settings_file
            .with_mut(|s| Ok(s.remove_override(&Path::new(&path), cfg.notify_handler.as_ref())))?
        {
            info!("override toolchain for '{}' removed", path);
        } else {
            info!("no override toolchain for '{}'", path);
            if !m.is_present("path") && !m.is_present("nonexistent") {
                info!(
                    "you may use `--path <path>` option to remove override toolchain \
                     for a specific path"
                );
            }
        }
    }
    Ok(())
}

const DOCS_DATA: &[(&'static str, &'static str, &'static str,)] = &[
    // flags can be used to open specific documents, e.g. `rustup doc --nomicon`
    // tuple elements: document name used as flag, help message, document index path
    ("alloc", "The Rust core allocation and collections library", "alloc/index.html"),
    ("book", "The Rust Programming Language book", "book/index.html"),
    ("cargo", "The Cargo Book", "cargo/index.html"),
    ("core", "The Rust Core Library", "core/index.html"),
    ("edition-guide", "The Rust Edition Guide", "edition-guide/index.html"),
    ("nomicon", "The Dark Arts of Advanced and Unsafe Rust Programming", "nomicon/index.html"),
    ("proc_macro", "A support library for macro authors when defining new macros", "proc_macro/index.html"),
    ("reference", "The Rust Reference", "reference/index.html"),
    ("rust-by-example", "A collection of runnable examples that illustrate various Rust concepts and standard libraries", "rust-by-example/index.html"),
    ("rustc", "The compiler for the Rust programming language", "rustc/index.html"),
    ("rustdoc", "Generate documentation for Rust projects", "rustdoc/index.html"),
    ("std", "Standard library API documentation", "std/index.html"),
    ("test", "Support code for rustc's built in unit-test and micro-benchmarking framework", "test/index.html"),
    ("unstable-book", "The Unstable Book", "unstable-book/index.html"),
];

fn doc(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let toolchain = explicit_or_dir_toolchain(cfg, m)?;

    let doc_url = if let Some((_, _, path)) = DOCS_DATA
        .into_iter()
        .find(|(name, _, _)| m.is_present(name))
    {
        path
    } else {
        "index.html"
    };

    if m.is_present("path") {
        let doc_path = toolchain.doc_path(doc_url)?;
        println!("{}", doc_path.display());
        Ok(())
    } else {
        Ok(toolchain.open_docs(doc_url)?)
    }
}

fn man(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    let manpage = m.value_of("command").expect("");
    let toolchain = explicit_or_dir_toolchain(cfg, m)?;
    let mut man_path = toolchain.path().to_path_buf();
    man_path.push("share");
    man_path.push("man");
    man_path.push("man1");
    man_path.push(manpage.to_owned() + ".1");
    utils::assert_is_file(&man_path)?;
    Command::new("man")
        .arg(man_path)
        .status()
        .expect("failed to open man page");
    Ok(())
}

fn self_uninstall(m: &ArgMatches<'_>) -> Result<()> {
    let no_prompt = m.is_present("no-prompt");

    self_update::uninstall(no_prompt)
}

fn set_default_host_triple(cfg: &Cfg, m: &ArgMatches<'_>) -> Result<()> {
    cfg.set_default_host_triple(m.value_of("host_triple").expect(""))?;
    Ok(())
}
