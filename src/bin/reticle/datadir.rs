//! Where the chip databases are, and fetching them when they are nowhere.
//!
//! The library never touches the network or the disk: a database reaches
//! it through a `FileProvider` rooted wherever the caller says. This
//! module is how the command line decides where that is. In order:
//!
//! 1. the path given on the command line (`--chipdb`);
//! 2. the database's environment variable (`RETICLE_CHIPDB`,
//!    `RETICLE_GOWINDB`, `RETICLE_TRELLISDB`);
//! 3. the per-user cache, `$XDG_CACHE_HOME/reticle/<name>/<version>`, or
//!    `~/.cache/reticle/...` without it (`%LOCALAPPDATA%\reticle\...` on
//!    Windows);
//! 4. failing all three, a download into that cache — unless `--offline`
//!    or `RETICLE_OFFLINE` says not to.
//!
//! A path given in (1) or (2) that does not hold the database is an
//! error, not a reason to download: somebody pointed at it on purpose.
//!
//! # What is downloaded, and how it is trusted
//!
//! Each database is pinned to one upstream version and to SHA-256 digests
//! compiled into this binary, so the cache always holds exactly the bytes
//! the tests were written against and never whatever upstream has moved
//! to since. For Project X-Ray that is one digest per file, recorded from
//! a checkout of the pinned commit (`prjxray-db.manifest`, next to this
//! file); for Project Apicula it is the digest PyPI publishes for the
//! wheel. A file that does not match is refused and nothing is installed.
//!
//! The transfer itself is the system's `curl`, over HTTPS only, redirects
//! included. That keeps the binary free of a TLS stack and of every
//! dependency one would bring; the digests, not `curl`, are what make
//! the result trustworthy.
//!
//! A download is assembled in a scratch directory beside its final place
//! and renamed into it once every file has checked out, so an interrupted
//! fetch leaves nothing that looks like a database, and two processes
//! fetching at once cannot leave a mixture.

use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Outcome;
use crate::args::{ArgError, Args};
use crate::{sha256, zip};

/// One database this command line knows how to find and fetch.
pub(crate) struct Database {
    /// The short name: the cache directory, and what `reticle fetch` takes.
    pub(crate) name: &'static str,
    /// The pinned upstream version: a commit, or a release number.
    pub(crate) version: &'static str,
    /// One line on what it is, for `reticle fetch` to print.
    pub(crate) what: &'static str,
    /// The environment variable that names an existing copy.
    pub(crate) env: &'static str,
    /// A file every complete copy has, relative to its root. It is how a
    /// path from the command line or the environment is checked.
    pub(crate) probe: &'static str,
    source: Source,
}

/// Where a database's bytes come from.
enum Source {
    /// Individual files under one URL prefix, each checked against a
    /// manifest of `<sha256> <size> <path>` lines.
    Files {
        base: &'static str,
        manifest: &'static str,
    },
    /// A Python wheel, checked whole, from which the members under
    /// `prefix` with one of `suffixes` are taken, flattened into the root,
    /// along with the licence member saved as `LICENSE`.
    Wheel {
        url: &'static str,
        sha256: &'static str,
        size: u64,
        prefix: &'static str,
        suffixes: &'static [&'static str],
        license: &'static str,
    },
}

/// Project X-Ray's database: the Artix-7 part of it, which is what the
/// 7-series flow loads and what `tests/fpga_xray.rs` reads, including
/// the Vivado reference bitstreams in `artix7/harness/`.
pub(crate) const PRJXRAY: Database = Database {
    name: "prjxray-db",
    version: "0a0addedd73e7e4139d52a6d8db4258763e0f1f3",
    what: "Project X-Ray's Xilinx 7-series database (f4pga/prjxray-db, CC0), Artix-7 part",
    env: "RETICLE_CHIPDB",
    probe: "artix7/xc7a50t/tilegrid.json",
    source: Source::Files {
        base: "https://raw.githubusercontent.com/f4pga/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3/",
        manifest: include_str!("prjxray-db.manifest"),
    },
};

/// Project Apicula's Gowin databases: every `<device>.msgpack.xz` the
/// 0.33 release ships, which is what `tests/fpga_gowin.rs` reads.
pub(crate) const APICULA: Database = Database {
    name: "apicula",
    version: "0.33",
    what: "Project Apicula's Gowin databases (apycula 0.33 on PyPI, MIT)",
    env: "RETICLE_GOWINDB",
    probe: "GW2A-18.msgpack.xz",
    source: Source::Wheel {
        url: "https://files.pythonhosted.org/packages/0f/04/4614442e8be95e79df75f06c3b2670cc4764749ec4d3d2d3de5b54ef93e7/apycula-0.33-py3-none-any.whl",
        sha256: "8c78b766da07fc9290d4e096fe64f143d67926434cb2d361405d7cd49f578286",
        size: 4_073_380,
        prefix: "apycula/",
        suffixes: &[".msgpack.xz"],
        license: "apycula-0.33.dist-info/licenses/LICENSE",
    },
};

/// Project Trellis' Lattice ECP5 database: the LFE5U-12F part of it, which
/// is what the ECP5 flow loads and what `tests/fpga_trellis.rs` reads.
///
/// Two things about this one are worth knowing.
///
/// It is the **database repository**, `YosysHQ/prjtrellis-db`, not
/// `prjtrellis` itself: `prjtrellis` carries it as a git submodule and the
/// version below is the commit that submodule points at, so what is
/// downloaded is what `ecppack` and nextpnr would read.
///
/// And it is a **subset**: 191 files of the repository's 705. The whole
/// thing is 81 MB across five device families; what an LFE5U-12F needs is
/// that part's three files, the 185 shared `bits.db` files of the ECP5
/// family, and the licence — 5.8 MB. The 45F and 85F are not here, and
/// neither is the timing data, because nothing reads them yet. The 12F's
/// files are byte-identical to the 25F's (the same die; Lattice's
/// TN-02039 Table B.4 says so too), so one copy serves both parts and only
/// the IDCODE tells them apart.
pub(crate) const TRELLIS: Database = Database {
    name: "prjtrellis-db",
    version: "015e0330630d7c238c0e4f2cdd9c8157eb78c54a",
    what: "Project Trellis' Lattice ECP5 database (YosysHQ/prjtrellis-db, CC0), LFE5U-12F part",
    env: "RETICLE_TRELLISDB",
    probe: "ECP5/LFE5U-12F/tilegrid.json",
    source: Source::Files {
        base: "https://raw.githubusercontent.com/YosysHQ/prjtrellis-db/015e0330630d7c238c0e4f2cdd9c8157eb78c54a/",
        manifest: include_str!("prjtrellis-db.manifest"),
    },
};

/// Every database, in the order `reticle fetch` lists them.
pub(crate) const ALL: [&Database; 3] = [&PRJXRAY, &APICULA, &TRELLIS];

/// The database called `name`.
pub(crate) fn by_name(name: &str) -> Option<&'static Database> {
    ALL.into_iter().find(|db| db.name == name)
}

/// The per-user cache directory, `.../reticle`, if the environment says
/// where a home is at all.
pub(crate) fn cache_root() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(dir) = var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(dir).join("reticle"));
    }
    if cfg!(windows)
        && let Some(dir) = var("LOCALAPPDATA")
    {
        return Some(PathBuf::from(dir).join("reticle"));
    }
    var("HOME").map(|home| PathBuf::from(home).join(".cache").join("reticle"))
}

/// True when `RETICLE_OFFLINE` is set to anything but empty or `0`.
pub(crate) fn offline_from_env() -> bool {
    std::env::var("RETICLE_OFFLINE").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Where a database was found, or where it would be cached.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// A complete copy, here.
    At(PathBuf),
    /// Nowhere yet; this is where a fetch would put it.
    Missing(PathBuf),
}

impl Database {
    /// This database's directory inside a cache root.
    pub(crate) fn cached_in(&self, root: &Path) -> PathBuf {
        root.join(self.name).join(self.version)
    }

    /// Steps 1 to 3 of the lookup, with every input passed in, so the
    /// order can be tested without touching the environment.
    pub(crate) fn find(
        &self,
        explicit: Option<&str>,
        env: Option<&str>,
        cache: Option<&Path>,
    ) -> Result<Found, String> {
        let named = explicit
            .map(|p| (p, "the path given"))
            .or_else(|| env.filter(|p| !p.is_empty()).map(|p| (p, self.env)));
        if let Some((path, from)) = named {
            let root = PathBuf::from(path);
            if root.join(self.probe).is_file() {
                return Ok(Found::At(root));
            }
            return Err(format!(
                "{from} is `{path}`, which does not hold {}: `{}` is not there",
                self.name, self.probe
            ));
        }
        let Some(cache) = cache else {
            return Err(format!(
                "no {} was given and there is no home directory to cache one in: \
                 set {} or XDG_CACHE_HOME",
                self.name, self.env
            ));
        };
        let dir = self.cached_in(cache);
        if dir.join(self.probe).is_file() {
            Ok(Found::At(dir))
        } else {
            Ok(Found::Missing(dir))
        }
    }

    /// The whole lookup: find it, and fetch it if it is nowhere and
    /// fetching is allowed. Progress goes to standard error.
    pub(crate) fn locate(&self, explicit: Option<&str>, offline: bool) -> Result<PathBuf, String> {
        let env = std::env::var(self.env).ok();
        let cache = cache_root();
        match self.find(explicit, env.as_deref(), cache.as_deref())? {
            Found::At(dir) => Ok(dir),
            Found::Missing(dir) if offline || offline_from_env() => Err(format!(
                "{} is not in the cache (`{}`) and fetching is off: run `reticle fetch {}` \
                 with a network, or set {} to a copy",
                self.name,
                dir.display(),
                self.name,
                self.env
            )),
            Found::Missing(dir) => {
                self.fetch_into(&dir)?;
                Ok(dir)
            }
        }
    }

    /// A one-line summary of what a fetch downloads.
    pub(crate) fn download_summary(&self) -> String {
        match &self.source {
            Source::Files { base, manifest } => {
                let files = parse_manifest(manifest).unwrap_or_default();
                let bytes: u64 = files.iter().map(|f| f.size).sum();
                format!(
                    "{} files, {}, from {}",
                    files.len(),
                    megabytes(bytes),
                    host(base)
                )
            }
            Source::Wheel { url, size, .. } => {
                format!("one wheel, {}, from {}", megabytes(*size), host(url))
            }
        }
    }

    /// Downloads, checks, and installs this database at `dest`.
    pub(crate) fn fetch_into(&self, dest: &Path) -> Result<(), String> {
        let parent = dest.parent().ok_or("a cache directory with no parent")?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create `{}`: {e}", parent.display()))?;
        eprintln!(
            "note: fetching {} {} ({}) into {}",
            self.name,
            short(self.version),
            self.download_summary(),
            dest.display()
        );
        let scratch = parent.join(format!(".{}.partial-{}", self.version, std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        fs::create_dir_all(&scratch)
            .map_err(|e| format!("cannot create `{}`: {e}", scratch.display()))?;
        let built = match &self.source {
            Source::Files { base, manifest } => fetch_files(base, manifest, &scratch),
            Source::Wheel {
                url,
                sha256,
                prefix,
                suffixes,
                license,
                ..
            } => fetch_wheel(url, sha256, prefix, suffixes, license, &scratch),
        };
        if let Err(err) = built {
            let _ = fs::remove_dir_all(&scratch);
            return Err(format!("fetching {} failed: {err}", self.name));
        }
        match fs::rename(&scratch, dest) {
            Ok(()) => {}
            // Another process finished first; its copy is the same bytes.
            Err(_) if dest.join(self.probe).is_file() => {
                let _ = fs::remove_dir_all(&scratch);
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&scratch);
                return Err(format!(
                    "cannot move the download into `{}`: {e}",
                    dest.display()
                ));
            }
        }
        eprintln!("note: {} is in {}", self.name, dest.display());
        Ok(())
    }
}

/// One line of a manifest.
#[derive(Debug)]
struct ManifestFile<'a> {
    sha256: &'a str,
    size: u64,
    path: &'a str,
}

/// Parses `<sha256> <size> <path>` lines, refusing any path that could
/// land outside the directory it is written into.
fn parse_manifest(text: &str) -> Result<Vec<ManifestFile<'_>>, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let mut parts = line.split(' ');
            let (Some(sha256), Some(size), Some(path), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Err(format!("a malformed manifest line: `{line}`"));
            };
            let size = size
                .parse()
                .map_err(|_| format!("a malformed size in `{line}`"))?;
            let safe = path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-/".contains(&b));
            if sha256.len() != 64
                || !safe
                || path.starts_with('/')
                || path.split('/').any(|seg| seg.is_empty() || seg == "..")
            {
                return Err(format!("a manifest line this will not write: `{line}`"));
            }
            Ok(ManifestFile { sha256, size, path })
        })
        .collect()
}

/// Every file of a manifest into `scratch`, by one `curl` running them in
/// parallel, then each checked.
fn fetch_files(base: &str, manifest: &str, scratch: &Path) -> Result<(), String> {
    let files = parse_manifest(manifest)?;
    // One `curl` for every file: a config file of url/output pairs.
    // Manifest paths are plain characters, so nothing needs quoting.
    let mut config = String::new();
    for f in &files {
        config.push_str(&format!(
            "url = \"{base}{}\"\noutput = \"{}\"\n",
            f.path, f.path
        ));
    }
    let config_path = beside(scratch, "curl");
    fs::write(&config_path, config).map_err(|e| format!("cannot write the curl config: {e}"))?;
    let ran = curl(
        scratch,
        &[
            "--parallel".as_ref(),
            "--parallel-max".as_ref(),
            "8".as_ref(),
            "--create-dirs".as_ref(),
            "--config".as_ref(),
            config_path.as_os_str(),
        ],
    );
    let _ = fs::remove_file(&config_path);
    ran?;
    for f in &files {
        let bytes = fs::read(scratch.join(f.path)).map_err(|e| format!("`{}`: {e}", f.path))?;
        if bytes.len() as u64 != f.size || sha256::hex(&bytes) != f.sha256 {
            return Err(format!(
                "`{}` is not the file the manifest records: {} bytes, SHA-256 {}",
                f.path,
                bytes.len(),
                sha256::hex(&bytes)
            ));
        }
    }
    Ok(())
}

/// The wheel into memory, checked whole, and its database members into
/// `scratch`.
fn fetch_wheel(
    url: &str,
    sha256_hex: &str,
    prefix: &str,
    suffixes: &[&str],
    license: &str,
    scratch: &Path,
) -> Result<(), String> {
    let wheel_path = beside(scratch, "whl");
    let ran = curl(
        scratch,
        &["--output".as_ref(), wheel_path.as_os_str(), url.as_ref()],
    )
    .and_then(|()| {
        fs::read(&wheel_path).map_err(|e| format!("cannot read the download back: {e}"))
    });
    let _ = fs::remove_file(&wheel_path);
    let wheel = ran?;
    let got = sha256::hex(&wheel);
    if got != sha256_hex {
        return Err(format!(
            "the wheel's SHA-256 is {got}, not the {sha256_hex} PyPI publishes"
        ));
    }
    let mut taken = 0usize;
    for member in zip::members(&wheel)? {
        let target = if member.name == license {
            "LICENSE"
        } else {
            match member.name.strip_prefix(prefix) {
                Some(rest)
                    if !rest.contains('/')
                        && !rest.starts_with('.')
                        && suffixes.iter().any(|s| rest.ends_with(s)) =>
                {
                    rest
                }
                _ => continue,
            }
        };
        let bytes = zip::extract(&wheel, &member)?;
        fs::write(scratch.join(target), bytes).map_err(|e| format!("`{target}`: {e}"))?;
        taken += 1;
    }
    if taken < 2 {
        return Err("the wheel does not hold the members it should".to_owned());
    }
    Ok(())
}

/// Runs `curl` in `dir` with `args`, over HTTPS only, redirects
/// included, and failing on any HTTP error rather than saving the page.
fn curl(dir: &Path, args: &[&OsStr]) -> Result<(), String> {
    let status = Command::new("curl")
        .current_dir(dir)
        .args(["--fail", "--location", "--silent", "--show-error"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--retry", "3", "--connect-timeout", "30"])
        .args(args)
        .status();
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("curl exited with {status}")),
        Err(e) if e.kind() == ErrorKind::NotFound => Err(
            "downloading needs `curl` on PATH; install it, or fetch the database by hand \
             (docs/fpga-xray.md, docs/fpga-gowin.md) and point the environment variable at it"
                .to_owned(),
        ),
        Err(e) => Err(format!("cannot run curl: {e}")),
    }
}

/// `<scratch>.<ext>`: a file next to the scratch directory, so it never
/// ends up inside the installed copy, and as unique as the directory is.
fn beside(scratch: &Path, ext: &str) -> PathBuf {
    let mut name = scratch.as_os_str().to_owned();
    name.push(".");
    name.push(ext);
    PathBuf::from(name)
}

/// `https://host/...` to `host`.
fn host(url: &str) -> &str {
    let rest = url.strip_prefix("https://").unwrap_or(url);
    rest.split('/').next().unwrap_or(rest)
}

/// A commit shortened the way `git log --oneline` does; a release as is.
fn short(version: &str) -> &str {
    if version.len() == 40 {
        &version[..7]
    } else {
        version
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{}.{} MB", bytes / 1_000_000, bytes % 1_000_000 / 100_000)
}

/// `reticle fetch`: list the databases, or make sure the named ones are
/// present and say where.
pub(crate) fn fetch_cmd(args: &Args) -> Result<Outcome, ArgError> {
    let offline = args.flag("offline");
    let path_only = args.flag("path");
    let mut wanted: Vec<&'static Database> = Vec::new();
    for name in args.positionals() {
        let named: &[&'static Database] = if name == "all" {
            &ALL
        } else if let Some(db) = by_name(name) {
            &[db]
        } else {
            let known: Vec<&str> = ALL.iter().map(|db| db.name).collect();
            return Ok(Outcome::Usage(format!(
                "no database is called `{name}`; there is {} and `all`",
                known.join(", ")
            )));
        };
        for &db in named {
            if !wanted.iter().any(|w| w.name == db.name) {
                wanted.push(db);
            }
        }
    }
    if wanted.is_empty() {
        if path_only {
            return Ok(Outcome::Usage("`--path` needs a database named".to_owned()));
        }
        let cache = cache_root();
        for db in ALL {
            let env = std::env::var(db.env).ok();
            let at = match db.find(None, env.as_deref(), cache.as_deref()) {
                Ok(Found::At(dir)) => format!("present   {}", dir.display()),
                Ok(Found::Missing(dir)) => format!("absent    {}", dir.display()),
                Err(err) => format!("error     {err}"),
            };
            println!("{:<11} {:<8} {at}", db.name, short(db.version));
            println!("{:<20} {}", "", db.what);
        }
        return Ok(Outcome::Ok);
    }
    let mut failed = false;
    for db in wanted {
        match db.locate(None, offline) {
            Ok(dir) if path_only => println!("{}", dir.display()),
            Ok(dir) => println!("{}: {}", db.name, dir.display()),
            Err(err) => {
                eprintln!("error: {err}");
                failed = true;
            }
        }
    }
    Ok(if failed { Outcome::Failed } else { Outcome::Ok })
}

#[cfg(test)]
mod tests {
    use super::{ALL, APICULA, Found, PRJXRAY, Source, TRELLIS, parse_manifest};
    use std::fs;
    use std::path::PathBuf;

    /// A fresh directory under the system's temporary one.
    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reticle-datadir-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A directory that holds `db`'s probe file and nothing else.
    fn fake_copy(root: &std::path::Path, db: &super::Database) {
        let probe = root.join(db.probe);
        fs::create_dir_all(probe.parent().unwrap()).unwrap();
        fs::write(probe, b"").unwrap();
    }

    #[test]
    fn a_named_path_wins_and_a_wrong_one_is_an_error_not_a_download() {
        let dir = scratch("named");
        let (good, cache) = (dir.join("good"), dir.join("cache"));
        fake_copy(&good, &PRJXRAY);
        fake_copy(&PRJXRAY.cached_in(&cache), &PRJXRAY);
        let good_str = good.to_str().unwrap();
        let wrong = dir.join("wrong");
        let wrong_str = wrong.to_str().unwrap();

        // The command line beats the environment, which beats the cache.
        let found = PRJXRAY.find(Some(good_str), Some(wrong_str), Some(&cache));
        assert_eq!(found, Ok(Found::At(good.clone())));
        let found = PRJXRAY.find(None, Some(good_str), Some(&cache));
        assert_eq!(found, Ok(Found::At(good)));
        let err = PRJXRAY
            .find(None, Some(wrong_str), Some(&cache))
            .unwrap_err();
        assert!(err.contains("RETICLE_CHIPDB"), "{err}");
        let err = PRJXRAY
            .find(Some(wrong_str), None, Some(&cache))
            .unwrap_err();
        assert!(err.contains("the path given"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cache_is_used_when_complete_and_named_when_not() {
        let dir = scratch("cache");
        let expect = APICULA.cached_in(&dir);
        assert_eq!(expect, dir.join("apicula").join("0.33"));
        // An empty environment variable counts as unset.
        assert_eq!(
            APICULA.find(None, Some(""), Some(&dir)),
            Ok(Found::Missing(expect.clone()))
        );
        // A directory without the probe file is not a copy: a fetch that
        // died half way leaves nothing under the final name, but a
        // hand-made directory might.
        fs::create_dir_all(&expect).unwrap();
        assert_eq!(
            APICULA.find(None, None, Some(&dir)),
            Ok(Found::Missing(expect.clone()))
        );
        fake_copy(&expect, &APICULA);
        assert_eq!(APICULA.find(None, None, Some(&dir)), Ok(Found::At(expect)));
        assert!(APICULA.find(None, None, None).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    /// The manifest compiled in is the one `docs/fpga-xray.md`'s sparse
    /// checkout produces, plus the licence, and every path in it is one
    /// the fetch will agree to write.
    #[test]
    fn the_xray_manifest_is_the_documented_checkout() {
        let Source::Files { manifest, .. } = &PRJXRAY.source else {
            panic!("prjxray-db is fetched file by file")
        };
        let files = parse_manifest(manifest).expect("every line parses");
        assert_eq!(files.len(), 275);
        let has = |p: &str| files.iter().any(|f| f.path == p);
        assert!(has("LICENSE"));
        assert!(has(PRJXRAY.probe));
        assert!(has("artix7/harness/basys3/swbut/design.bit"));
        assert!(has("artix7/xc7a35tcpg236-1/package_pins.csv"));
        assert!(
            files
                .iter()
                .all(|f| f.path == "LICENSE" || f.path.starts_with("artix7/"))
        );
    }

    #[test]
    fn a_manifest_line_that_escapes_its_directory_is_refused() {
        let sha = "0".repeat(64);
        for path in ["../x", "/etc/passwd", "a//b", "a/../b", "a b", "a\\b"] {
            let line = format!("{sha} 1 {path}");
            assert!(parse_manifest(&line).is_err(), "{path}");
        }
        assert!(parse_manifest(&format!("{sha} 1 artix7/a.db")).is_ok());
    }

    /// The tests find a fetched copy by the same name and version, which
    /// they have to spell out because they cannot see this module. This
    /// is what keeps the two in step.
    #[test]
    fn the_integration_tests_look_for_the_versions_pinned_here() {
        let xray = include_str!("../../../tests/fpga_xray.rs");
        let gowin = include_str!("../../../tests/fpga_gowin.rs");
        let trellis_test = include_str!("../../../tests/fpga_trellis.rs");
        for (db, test) in [
            (&PRJXRAY, xray),
            (&APICULA, gowin),
            (&TRELLIS, trellis_test),
        ] {
            for pinned in [db.name, db.version, db.env] {
                let quoted = format!("\"{pinned}\"");
                assert!(
                    test.contains(&quoted),
                    "{} should look for {quoted}",
                    db.name
                );
            }
            // And for the file it probes, which is what tells "the
            // database is not there" from "it is there and incomplete".
            assert!(
                test.contains(db.probe),
                "{} should probe for {}",
                db.name,
                db.probe
            );
        }
        assert_eq!(ALL.len(), 3);
    }
}
